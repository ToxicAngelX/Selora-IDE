//! OS keyring access for the Selora API key (PLAN.md §5, ARCHITECTURE.md §7).
//!
//! The key's home is the platform credential store — Windows Credential
//! Manager in the shipped build, via the `keyring` crate — under service
//! [`SERVICE`], entry [`ENTRY`].
//!
//! A plaintext fallback file in the settings dir (`%APPDATA%\selora-ide\
//! .api_key` / `.brave_key`) is kept **only** as a resilience path for
//! when the credential store is unavailable:
//! - `set` writes the keyring and drops any stale fallback copy; it writes
//!   the fallback only if the keyring write fails;
//! - `get` reads the keyring, falls back to the file when the keyring has
//!   no usable entry, and opportunistically restores the keyring — a read
//!   never (re)writes the plaintext file;
//! - `delete` removes the fallback first and surfaces a failure, so a key
//!   can never silently survive on disk after Disconnect.
//!
//! Semantics the callers rely on (ARCHITECTURE.md §7):
//! - a missing key is a *state*, not an error: `get` → `Ok(None)`;
//! - `delete` of a missing entry is `Ok(())` (Disconnect is idempotent);
//! - failures surface as [`SeloraError::Config`] carrying the keyring's
//!   own message — which never contains the key itself.

use std::path::{Path, PathBuf};

use selora_core::SeloraError;

/// The keyring service: the app's stable identity in the credential store.
pub const SERVICE: &str = "selora-ide";
/// The keyring entry (the `user` field) holding the API key.
pub const ENTRY: &str = "api-key";
/// The entry holding the Brave Search API key (Phase R5: the agent
/// section's field stores it here — never in settings.json).
pub const BRAVE_ENTRY: &str = "brave-key";

/// The entry for `service` (tests use throwaway service names so the
/// round-trips never touch the real key).
fn entry(service: &str) -> Result<keyring::Entry, SeloraError> {
    entry_for(service, ENTRY)
}

/// The entry for `service` and `user`.
fn entry_for(service: &str, user: &str) -> Result<keyring::Entry, SeloraError> {
    keyring::Entry::new(service, user)
        .map_err(|err| SeloraError::Config(format!("keyring unavailable: {err}")))
}

/// The file the fallback copy of `user`'s key lives in inside `dir`.
///
/// `None` for a name Selora does not own — the fallback only ever mirrors
/// the two documented entries.
fn fallback_path_in(dir: &Path, user: &str) -> Option<PathBuf> {
    let filename = match user {
        ENTRY => ".api_key",
        BRAVE_ENTRY => ".brave_key",
        _ => return None,
    };
    Some(dir.join(filename))
}

/// Read the fallback key for `user` from `dir`. `None` = absent, unreadable
/// or blank: the fallback is a resilience path, never an error state.
fn fallback_read_in(dir: &Path, user: &str) -> Option<String> {
    let path = fallback_path_in(dir, user)?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Read the live fallback key for `user` (settings dir).
fn fallback_get(user: &str) -> Option<String> {
    let dir = crate::Settings::settings_dir().ok()?;
    fallback_read_in(&dir, user)
}

/// Write `key` (trimmed) as the fallback copy for `user` inside `dir`.
fn fallback_write_in(dir: &Path, user: &str, key: &str) -> Result<(), SeloraError> {
    let Some(path) = fallback_path_in(dir, user) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            SeloraError::Config(format!(
                "could not create the fallback key directory: {err}"
            ))
        })?;
    }
    std::fs::write(&path, key.trim()).map_err(|err| {
        SeloraError::Config(format!(
            "could not write the fallback key file {}: {err}",
            path.display()
        ))
    })
}

/// Write the live fallback copy for `user` (settings dir).
fn fallback_set(user: &str, key: &str) -> Result<(), SeloraError> {
    let dir = crate::Settings::settings_dir()?;
    fallback_write_in(&dir, user, key)
}

/// Remove the fallback copy for `user` inside `dir`.
///
/// A missing file is `Ok(())` (Disconnect is idempotent); every other
/// failure is reported so a key can never silently survive on disk.
fn fallback_delete_in(dir: &Path, user: &str) -> Result<(), SeloraError> {
    let Some(path) = fallback_path_in(dir, user) else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(SeloraError::Config(format!(
            "could not remove the fallback key file {}: {err}",
            path.display()
        ))),
    }
}

/// Remove the live fallback copy for `user` (settings dir).
fn fallback_delete(user: &str) -> Result<(), SeloraError> {
    match crate::Settings::settings_dir() {
        Ok(dir) => fallback_delete_in(&dir, user),
        // Without a settings directory there is no fallback file to remove
        // (`set` could never have written one either).
        Err(_) => Ok(()),
    }
}

/// Store `key` under `user`: the keyring is the home.
///
/// Only when the credential store cannot take it does the plaintext
/// fallback become the resilience copy — the read path prefers the keyring
/// and restores it once the store is available again. `Ok` means the key
/// was persisted somewhere; `Err` means it was persisted nowhere.
fn store_key(
    user: &str,
    key: &str,
    entry: Result<keyring::Entry, SeloraError>,
) -> Result<(), SeloraError> {
    let keyring_error = match entry {
        Ok(entry) => match entry.set_password(key) {
            Ok(()) => {
                // The keyring is authoritative now: drop a stale plaintext
                // copy left by an older build or an earlier keyring outage.
                // Best-effort — the write itself already succeeded.
                let _ = fallback_delete(user);
                return Ok(());
            }
            Err(err) => format!("keyring write failed: {err}"),
        },
        Err(SeloraError::Config(message)) => message,
        Err(other) => other.to_string(),
    };
    fallback_set(user, key)
        .map_err(|fallback_error| SeloraError::Config(format!("{keyring_error}; {fallback_error}")))
}

/// Read the stored API key. `Ok(None)` = nothing stored yet (first run,
/// or after Disconnect).
pub fn api_key_get() -> Result<Option<String>, SeloraError> {
    match entry(SERVICE)?.get_password() {
        Ok(key) if !key.trim().is_empty() => {
            // The keyring is authoritative. A stale plaintext copy is dropped
            // (best-effort); a read never *writes* the fallback and never
            // fails because the cleanup failed.
            let _ = fallback_delete(ENTRY);
            Ok(Some(key))
        }
        _ => {
            if let Some(key) = fallback_get(ENTRY) {
                // Opportunistically restore back into OS keyring
                if let Ok(ent) = entry(SERVICE) {
                    let _ = ent.set_password(&key);
                }
                Ok(Some(key))
            } else {
                Ok(None)
            }
        }
    }
}

/// Store (overwrite) the API key — the Connect flow's final step, after
/// validation succeeded.
pub fn api_key_set(key: &str) -> Result<(), SeloraError> {
    store_key(ENTRY, key, entry(SERVICE))
}

/// Read the stored Brave Search API key. `Ok(None)` = nothing stored.
/// Phase R5: the agent's web_search tool reads it through the shell's
/// turn-request seam; settings.json never carries it.
pub fn brave_key_get() -> Result<Option<String>, SeloraError> {
    match entry_for(SERVICE, BRAVE_ENTRY)?.get_password() {
        Ok(key) if !key.trim().is_empty() => {
            let _ = fallback_delete(BRAVE_ENTRY);
            Ok(Some(key))
        }
        _ => {
            if let Some(key) = fallback_get(BRAVE_ENTRY) {
                if let Ok(ent) = entry_for(SERVICE, BRAVE_ENTRY) {
                    let _ = ent.set_password(&key);
                }
                Ok(Some(key))
            } else {
                Ok(None)
            }
        }
    }
}

/// Store (overwrite) the Brave Search API key — the Agent section's
/// field, on commit.
pub fn brave_key_set(key: &str) -> Result<(), SeloraError> {
    store_key(BRAVE_ENTRY, key, entry_for(SERVICE, BRAVE_ENTRY))
}

/// Remove the stored Brave Search API key. A missing entry is a no-op.
pub fn brave_key_delete() -> Result<(), SeloraError> {
    // The fallback goes first: if the plaintext copy cannot be removed the
    // caller must see the error — the key must not silently stay on disk
    // after Disconnect.
    fallback_delete(BRAVE_ENTRY)?;
    match entry_for(SERVICE, BRAVE_ENTRY)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(SeloraError::Config(format!("keyring delete failed: {err}"))),
    }
}

/// Remove the stored API key (Disconnect). A missing entry is a no-op.
pub fn api_key_delete() -> Result<(), SeloraError> {
    fallback_delete(ENTRY)?;
    match entry(SERVICE)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(SeloraError::Config(format!("keyring delete failed: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    /// A throwaway service name so the round-trips below never read,
    /// overwrite, or delete the owner's real `selora-ide` entry.
    fn test_service() -> String {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        format!("selora-ide-keyring-test-{}-{n}", std::process::id())
    }

    #[test]
    fn round_trip_set_get_delete() {
        let service = test_service();
        let entry = entry(&service).expect("entry");

        entry.set_password("sk-gw-roundtrip").expect("set");
        assert_eq!(entry.get_password().expect("get"), "sk-gw-roundtrip");

        entry.delete_credential().expect("delete");
        // The raw entry delete of a missing entry is NoEntry — the public
        // `api_key_delete` normalizes that to `Ok(())` (idempotent
        // Disconnect); this test asserts the raw backend's shape.
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry on second delete, got {other:?}"),
        }
        match entry.get_password() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry after delete, got {other:?}"),
        }
    }

    /// Phase R5: the Brave entry round-trips under the same throwaway
    /// service discipline (the real service's entry is never touched).
    #[test]
    fn brave_entry_round_trip() {
        let service = test_service();
        let entry = entry_for(&service, BRAVE_ENTRY).expect("brave entry");

        entry.set_password("BSA-key").expect("set");
        assert_eq!(entry.get_password().expect("get"), "BSA-key");
        entry.delete_credential().expect("delete");
        match entry.get_password() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry after delete, got {other:?}"),
        }
    }

    #[test]
    fn get_and_delete_of_a_missing_entry_are_ok() {
        let service = test_service();
        // Never written under this unique service name.
        match entry(&service).expect("entry").get_password() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry, got {other:?}"),
        }
        // Raw backend: delete of a missing entry is NoEntry (the public
        // `api_key_delete` normalizes it to `Ok(())`).
        match entry(&service).expect("entry").delete_credential() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // The plaintext fallback: exercised through the directory seam, so
    // these tests never touch a credential store at all (throwaway or
    // otherwise) — they only read/write a temp settings dir.
    // -----------------------------------------------------------------

    fn fallback_dir(label: &str) -> PathBuf {
        crate::settings::tests::temp_settings_dir(label)
    }

    #[test]
    fn fallback_write_read_delete_round_trips_in_a_temp_dir() {
        let dir = fallback_dir("keyring-fallback-round-trip");
        std::fs::create_dir_all(&dir).expect("temp dir");

        // Absent → None (a missing fallback is a state, not an error).
        assert_eq!(fallback_read_in(&dir, ENTRY), None);

        // Written trimmed, under exactly the documented filename.
        fallback_write_in(&dir, ENTRY, "  sk-gw-fallback \n").expect("write");
        assert_eq!(
            fallback_read_in(&dir, ENTRY).as_deref(),
            Some("sk-gw-fallback")
        );
        assert!(
            dir.join(".api_key").is_file(),
            "the fallback lives at <settings dir>/.api_key"
        );

        fallback_delete_in(&dir, ENTRY).expect("delete");
        assert_eq!(fallback_read_in(&dir, ENTRY), None);
        // Disconnect is idempotent: a second delete is still Ok.
        fallback_delete_in(&dir, ENTRY).expect("second delete");

        // The Brave key lives in its own file and never collides.
        fallback_write_in(&dir, BRAVE_ENTRY, "BSA-fallback").expect("brave write");
        assert!(dir.join(".brave_key").is_file());
        assert_eq!(
            fallback_read_in(&dir, BRAVE_ENTRY).as_deref(),
            Some("BSA-fallback")
        );
        assert_eq!(fallback_read_in(&dir, ENTRY), None);
        fallback_delete_in(&dir, BRAVE_ENTRY).expect("brave delete");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallback_read_rejects_blank_and_unknown_entries() {
        let dir = fallback_dir("keyring-fallback-blank");
        std::fs::create_dir_all(&dir).expect("temp dir");

        // Whitespace-only content is not a key.
        fallback_write_in(&dir, ENTRY, "  \n\t ").expect("write");
        assert_eq!(fallback_read_in(&dir, ENTRY), None);

        // Only the two documented entries have a fallback file.
        assert!(fallback_path_in(&dir, "not-a-selora-entry").is_none());
        assert!(fallback_path_in(&dir, ENTRY).is_some());
        assert!(fallback_path_in(&dir, BRAVE_ENTRY).is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallback_delete_reports_a_path_it_cannot_remove() {
        let dir = fallback_dir("keyring-fallback-delete-error");
        // A directory squatting on the key path makes `remove_file` fail on
        // every supported platform: the helper must report it instead of
        // pretending the key is gone (goal: no silent plaintext survivor).
        std::fs::create_dir_all(dir.join(".api_key")).expect("dir at the key path");

        let err = fallback_delete_in(&dir, ENTRY).expect_err("must report the failure");
        assert!(
            matches!(err, SeloraError::Config(ref message) if message.contains("could not remove the fallback key file")),
            "unexpected error: {err:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
