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

use selora_core::SeloraError;

/// The repo's "latest release" endpoint (RELEASE.md §7: the owner publishes
/// GitHub Releases from the `v*.*.*` tag pipeline).
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest";

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
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
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
    check_latest_release_at(LATEST_RELEASE_URL, current).await
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
    Ok(ReleaseInfo {
        tag: release.tag_name,
        html_url: release.html_url,
        newer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
