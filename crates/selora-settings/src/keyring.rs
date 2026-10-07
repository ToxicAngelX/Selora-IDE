//! OS keyring access for the Selora API key (PLAN.md §5, ARCHITECTURE.md §7).
//!
//! The key never touches a file. It lives in the platform credential
//! store — Windows Credential Manager in the shipped build, via the
//! `keyring` crate — under service [`SERVICE`], entry [`ENTRY`].
//!
//! Semantics the callers rely on (ARCHITECTURE.md §7):
//! - a missing key is a *state*, not an error: `get` → `Ok(None)`;
//! - `delete` of a missing entry is `Ok(())` (Disconnect is idempotent);
//! - failures surface as [`SeloraError::Config`] carrying the keyring's
//!   own message — which never contains the key itself.

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

/// Read the stored API key. `Ok(None)` = nothing stored yet (first run,
/// or after Disconnect).
pub fn api_key_get() -> Result<Option<String>, SeloraError> {
    match entry(SERVICE)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(SeloraError::Config(format!("keyring read failed: {err}"))),
    }
}

/// Store (overwrite) the API key — the Connect flow's final step, after
/// validation succeeded.
pub fn api_key_set(key: &str) -> Result<(), SeloraError> {
    entry(SERVICE)?
        .set_password(key)
        .map_err(|err| SeloraError::Config(format!("keyring write failed: {err}")))
}

/// Read the stored Brave Search API key. `Ok(None)` = nothing stored.
/// Phase R5: the agent's web_search tool reads it through the shell's
/// turn-request seam; settings.json never carries it.
pub fn brave_key_get() -> Result<Option<String>, SeloraError> {
    match entry_for(SERVICE, BRAVE_ENTRY)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(SeloraError::Config(format!("keyring read failed: {err}"))),
    }
}

/// Store (overwrite) the Brave Search API key — the Agent section's
/// field, on commit.
pub fn brave_key_set(key: &str) -> Result<(), SeloraError> {
    entry_for(SERVICE, BRAVE_ENTRY)?
        .set_password(key)
        .map_err(|err| SeloraError::Config(format!("keyring write failed: {err}")))
}

/// Remove the stored Brave Search API key. A missing entry is a no-op.
pub fn brave_key_delete() -> Result<(), SeloraError> {
    match entry_for(SERVICE, BRAVE_ENTRY)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(SeloraError::Config(format!("keyring delete failed: {err}"))),
    }
}

/// Remove the stored API key (Disconnect). A missing entry is a no-op.
pub fn api_key_delete() -> Result<(), SeloraError> {
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
}
