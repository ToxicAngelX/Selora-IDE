//! The "Check for updates" client (Phase 6b, UI_SPEC.md §11, RELEASE.md §4).
//!
//! One GET against the GitHub Releases API for the repo's latest release.
//! This is the only non-gateway HTTP in the app; it lives in selora-ai so
//! the workspace keeps a single HTTP stack (rustls reqwest — PLAN.md §4.7).
//!
//! Version comparison is a numeric triple compare (`v0.6.0` > `0.1.0`): a
//! tag that does not parse as a semver-ish triple is reported as *not*
//! newer (never a false "update available" for a malformed tag).

use serde::Deserialize;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use selora_core::SeloraError;

/// The repository's public home, from the workspace manifest
/// (`[workspace.package] repository`). Shared by the About dialog and the
/// update check, so a rename is a one-line Cargo.toml change.
pub const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");

/// The `owner/repo` part of [`REPOSITORY_URL`] (no scheme, no `.git`).
fn release_slug() -> &'static str {
    let url = REPOSITORY_URL.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    url.strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .unwrap_or(url)
}

/// The repo's "latest release" endpoint, derived from the workspace manifest
/// so a repository rename cannot leave the update check on a stale slug.
pub fn latest_release_url() -> String {
    format!(
        "https://api.github.com/repos/{}/releases/latest",
        release_slug()
    )
}

/// What the update check learned about the latest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// The release's tag as published (e.g. `v0.6.0`).
    pub tag: String,
    /// The release page's URL (the toast's "download" link).
    pub html_url: String,
    /// Whether the tag is a semver-ish triple strictly newer than the
    /// running build.
    pub newer: bool,
    /// The release's `Selora-IDE-Setup-<tag>.exe` asset, when published.
    pub installer: Option<ReleaseAsset>,
    /// The matching `.exe.sha256` asset, when published.
    pub checksum: Option<ReleaseAsset>,
}

/// One file attached to a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    /// GitHub-reported size in bytes (0 when the payload omitted it).
    pub size: u64,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Clone, Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

impl AssetJson {
    fn into_asset(self) -> ReleaseAsset {
        ReleaseAsset {
            name: self.name,
            download_url: self.browser_download_url,
            size: self.size,
        }
    }
}

/// Parse a `v0.6.0` / `0.6.0` triple. `None` for anything else (suffixes
/// like `-rc1` are not produced by the release pipeline and never compare
/// newer).
fn version_triple(tag: &str) -> Option<(u64, u64, u64)> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = tag.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// The update check (UI_SPEC.md §11): fetch the latest release and compare
/// it against `current` (the running build's `CARGO_PKG_VERSION`).
pub async fn check_latest_release(current: &str) -> Result<ReleaseInfo, SeloraError> {
    check_latest_release_at(&latest_release_url(), current).await
}

/// The same check against an explicit endpoint (the wiremock seam).
pub async fn check_latest_release_at(url: &str, current: &str) -> Result<ReleaseInfo, SeloraError> {
    let client = reqwest::Client::builder()
        // GitHub's API rejects requests without a User-Agent.
        .user_agent(concat!("selora-ide/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| SeloraError::Net(error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| SeloraError::Net(error.to_string()))?;
    if !response.status().is_success() {
        return Err(SeloraError::Net(format!(
            "releases endpoint answered {}",
            response.status()
        )));
    }
    let release: ReleaseJson = response
        .json()
        .await
        .map_err(|error| SeloraError::Net(format!("unreadable release payload: {error}")))?;
    let newer = match (version_triple(&release.tag_name), version_triple(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    };
    // The release pipeline names both assets after the tag without the `v`
    // (`installer/selora-ide.iss` OutputBaseFilename + the checksum step). A
    // release published by hand may omit them; the check still reports
    // `newer`, and the UI falls back to the release page.
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let legacy_installer_name = format!("Selora-IDE-Setup-{version}.exe");
    let asset = |name: &str| {
        release
            .assets
            .iter()
            .find(|candidate| candidate.name == name)
            .map(|candidate| candidate.clone().into_asset())
    };
    let installer_name = if asset(&legacy_installer_name).is_some() {
        legacy_installer_name.as_str()
    } else {
        "selora-installer-v1.exe"
    };
    let checksum_name = format!("{installer_name}.sha256");
    Ok(ReleaseInfo {
        tag: release.tag_name,
        html_url: release.html_url,
        newer,
        installer: asset(installer_name),
        checksum: asset(&checksum_name),
    })
}

/// Parse a published `.sha256` file: one lowercase-or-uppercase hex digest,
/// optionally followed by the file name. `None` for anything else, so a
/// truncated or HTML error page can never compare equal to a download.
pub fn parse_checksum(text: &str) -> Option<String> {
    let digest = text.split_whitespace().next()?.to_ascii_lowercase();
    if digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(digest)
    } else {
        None
    }
}

fn update_client() -> Result<reqwest::Client, SeloraError> {
    reqwest::Client::builder()
        // GitHub's API rejects requests without a User-Agent.
        .user_agent(concat!("selora-ide/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| SeloraError::Net(error.to_string()))
}

/// Fetch and parse the release's `.sha256` asset.
pub async fn fetch_checksum(url: &str) -> Result<String, SeloraError> {
    let response = update_client()?
        .get(url)
        .send()
        .await
        .map_err(|error| SeloraError::Net(error.to_string()))?;
    if !response.status().is_success() {
        return Err(SeloraError::Net(format!(
            "checksum endpoint answered {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|error| SeloraError::Net(format!("unreadable checksum: {error}")))?;
    parse_checksum(&body)
        .ok_or_else(|| SeloraError::Net("checksum file is not one SHA256 digest".into()))
}

/// Download `asset` into `dest_dir`, hashing while it streams, and return the
/// final path. `progress` receives `(bytes_written, total_bytes)`; a total of
/// `0` means the server did not report a length.
///
/// The file is written as `<name>.part` and only renamed after the size and
/// the published SHA256 both match, so a failed or tampered download can never
/// be launched as an installer.
pub async fn download_installer(
    asset: &ReleaseAsset,
    expected_sha256: &str,
    dest_dir: &Path,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<PathBuf, SeloraError> {
    // `asset.name` comes from the release payload: keep only the leaf.
    let file_name = Path::new(&asset.name)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| SeloraError::Net(format!("unusable asset name: {}", asset.name)))?;
    tokio::fs::create_dir_all(dest_dir).await?;
    let final_path = dest_dir.join(file_name);
    let part_path = dest_dir.join(format!("{file_name}.part"));
    let _ = tokio::fs::remove_file(&part_path).await;
    let _ = tokio::fs::remove_file(&final_path).await;

    let response = update_client()?
        .get(&asset.download_url)
        .send()
        .await
        .map_err(|error| SeloraError::Net(error.to_string()))?;
    if !response.status().is_success() {
        return Err(SeloraError::Net(format!(
            "installer download answered {}",
            response.status()
        )));
    }
    let total = if asset.size > 0 {
        asset.size
    } else {
        response.content_length().unwrap_or(0)
    };

    let mut file = tokio::fs::File::create(&part_path).await?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| SeloraError::Net(format!("download aborted: {error}")))?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        written += chunk.len() as u64;
        progress(written, total);
    }
    file.flush().await?;
    drop(file);

    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let verified = written == total && hex.eq_ignore_ascii_case(expected_sha256);
    if !verified {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(SeloraError::Net(format!(
            "installer failed verification ({written} of {total} bytes, sha256 {hex})"
        )));
    }

    tokio::fs::rename(&part_path, &final_path).await?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_url_derives_from_the_manifest_repository() {
        // A rename in [workspace.package] repository must move the update
        // check with it. CI also runs scripts/check-release-metadata.ps1,
        // which asserts this derivation still exists.
        assert!(REPOSITORY_URL.starts_with("https://github.com/"));
        let slug = release_slug();
        assert_eq!(slug.matches('/').count(), 1, "slug is owner/repo");
        assert!(!slug.contains("://"));
        assert_eq!(
            latest_release_url(),
            format!("https://api.github.com/repos/{slug}/releases/latest")
        );
    }

    #[test]
    fn triples_parse_with_and_without_the_v_prefix() {
        assert_eq!(version_triple("v0.6.0"), Some((0, 6, 0)));
        assert_eq!(version_triple("0.6.0"), Some((0, 6, 0)));
        assert_eq!(version_triple("1.2.34"), Some((1, 2, 34)));
    }

    #[test]
    fn non_triples_never_parse() {
        assert_eq!(version_triple("v0.6"), None);
        assert_eq!(version_triple("v0.6.0-rc1"), None);
        assert_eq!(version_triple("latest"), None);
        assert_eq!(version_triple("v0.6.0.1"), None);
        assert_eq!(version_triple(""), None);
    }

    #[tokio::test]
    async fn a_newer_release_reports_newer() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "tag_name": "v9.9.9",
                    "html_url": "https://github.com/ToxicAngelX/Selora-IDE/releases/tag/v9.9.9"
                })),
            )
            .mount(&server)
            .await;
        let info = check_latest_release_at(&server.uri(), "0.1.0")
            .await
            .expect("ok");
        assert!(info.newer);
        assert_eq!(info.tag, "v9.9.9");
        assert!(info.html_url.ends_with("/v9.9.9"));
    }

    #[tokio::test]
    async fn same_or_older_releases_are_not_newer() {
        for tag in ["v0.1.0", "v0.0.9", "0.1.0"] {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_json(
                        serde_json::json!({ "tag_name": tag, "html_url": "https://x" }),
                    ),
                )
                .mount(&server)
                .await;
            let info = check_latest_release_at(&server.uri(), "0.1.0")
                .await
                .expect("ok");
            assert!(!info.newer, "{tag} must not read newer than 0.1.0");
        }
    }

    #[tokio::test]
    async fn a_malformed_tag_is_not_newer_and_still_surfaces() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "tag_name": "nightly", "html_url": "https://x" }),
            ))
            .mount(&server)
            .await;
        let info = check_latest_release_at(&server.uri(), "0.1.0")
            .await
            .expect("ok");
        assert!(!info.newer);
        assert_eq!(info.tag, "nightly");
    }

    #[tokio::test]
    async fn http_errors_are_net_errors() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let error = check_latest_release_at(&server.uri(), "0.1.0")
            .await
            .expect_err("404 is an error");
        assert!(matches!(error, SeloraError::Net(_)));
        assert!(error.to_string().contains("404"));
    }

    #[tokio::test]
    async fn a_release_exposes_its_setup_and_checksum_assets() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "tag_name": "v0.2.0",
                    "html_url": "https://github.com/x/y/releases/tag/v0.2.0",
                    "assets": [
                        {
                            "name": "Selora-IDE-Setup-0.2.0.exe",
                            "browser_download_url": "https://example.invalid/setup.exe",
                            "size": 1234
                        },
                        {
                            "name": "Selora-IDE-Setup-0.2.0.exe.sha256",
                            "browser_download_url": "https://example.invalid/setup.exe.sha256",
                            "size": 90
                        }
                    ]
                })),
            )
            .mount(&server)
            .await;
        let info = check_latest_release_at(&server.uri(), "0.1.0")
            .await
            .expect("ok");
        assert!(info.newer);
        let installer = info.installer.expect("installer asset");
        assert_eq!(installer.name, "Selora-IDE-Setup-0.2.0.exe");
        assert_eq!(installer.size, 1234);
        assert!(info
            .checksum
            .expect("checksum asset")
            .name
            .ends_with(".exe.sha256"));
    }

    #[tokio::test]
    async fn a_release_exposes_the_fixed_installer_name() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag_name": "v0.2.0",
                "html_url": "https://github.com/x/y/releases/tag/v0.2.0",
                "assets": [
                    { "name": "selora-installer-v1.exe", "browser_download_url": "https://example.invalid/setup.exe", "size": 1234 },
                    { "name": "selora-installer-v1.exe.sha256", "browser_download_url": "https://example.invalid/setup.sha256", "size": 90 }
                ]
            })))
            .mount(&server)
            .await;
        let info = check_latest_release_at(&server.uri(), "0.1.0").await.expect("ok");
        assert_eq!(info.installer.unwrap().name, "selora-installer-v1.exe");
        assert_eq!(info.checksum.unwrap().name, "selora-installer-v1.exe.sha256");
    }

    #[test]
    fn a_checksum_file_is_one_hex_digest() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{digest}  Selora-IDE-Setup-0.2.0.exe\n")),
            Some(digest.clone())
        );
        assert_eq!(parse_checksum(&digest.to_ascii_uppercase()), Some(digest));
        assert_eq!(parse_checksum("<html>not found</html>"), None);
        assert_eq!(parse_checksum(""), None);
    }
}
