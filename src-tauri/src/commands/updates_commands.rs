//! Manual release update check commands.
//!
//! The frontend invokes this only when the user presses "check update".
//! Network access stays in Rust so the frontend CSP does not need GitHub in connect-src.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/himawaril2dev/vocalsync-studio/releases/latest";
const REQUEST_TIMEOUT_SECS: u64 = 10;
const USER_AGENT: &str = concat!("vocalsync-studio/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    html_url: String,
}

pub(crate) fn parse_release_json(body: &str) -> Result<ReleaseInfo, AppError> {
    let parsed: GithubReleaseResponse = serde_json::from_str(body)
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub release response: {e}")))?;

    let tag = parsed.tag_name.trim();
    let url = parsed.html_url.trim();

    if tag.is_empty() {
        return Err(AppError::Internal(
            "GitHub release response is missing tag_name".into(),
        ));
    }
    if url.is_empty() {
        return Err(AppError::Internal(
            "GitHub release response is missing html_url".into(),
        ));
    }

    Ok(ReleaseInfo {
        tag_name: tag.to_string(),
        html_url: url.to_string(),
    })
}

#[tauri::command]
pub async fn check_latest_release() -> Result<ReleaseInfo, AppError> {
    tauri::async_runtime::spawn_blocking(fetch_latest_release_blocking)
        .await
        .map_err(|e| AppError::Internal(format!("Update check worker failed: {e}")))?
}

fn fetch_latest_release_blocking() -> Result<ReleaseInfo, AppError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .timeout_read(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build();

    let resp = agent
        .get(GITHUB_RELEASES_API)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| AppError::Internal(format!("Could not connect to GitHub: {e}")))?;

    let body = resp
        .into_string()
        .map_err(|e| AppError::Internal(format!("Could not read GitHub response: {e}")))?;

    parse_release_json(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_release_json() {
        let body = r#"{
            "tag_name": "v0.3.11",
            "html_url": "https://github.com/himawaril2dev/vocalsync-studio/releases/tag/v0.3.11"
        }"#;

        let info = parse_release_json(body).expect("release JSON should parse");
        assert_eq!(info.tag_name, "v0.3.11");
        assert_eq!(
            info.html_url,
            "https://github.com/himawaril2dev/vocalsync-studio/releases/tag/v0.3.11"
        );
    }

    #[test]
    fn ignores_extra_fields_in_json() {
        let body = r#"{
            "tag_name": "v0.3.12",
            "html_url": "https://example.com/r/v0.3.12",
            "name": "Release",
            "body": "changelog",
            "draft": false,
            "prerelease": false,
            "author": { "login": "someone" }
        }"#;

        let info = parse_release_json(body).expect("extra fields should be ignored");
        assert_eq!(info.tag_name, "v0.3.12");
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_release_json("not json").unwrap_err();
        assert!(err.to_string().contains("parse"));
    }

    #[test]
    fn rejects_missing_tag_name() {
        let err = parse_release_json(r#"{ "html_url": "https://example.com/r/x" }"#).unwrap_err();
        assert!(err.to_string().contains("parse"));
    }

    #[test]
    fn rejects_empty_tag_name() {
        let body = r#"{ "tag_name": "   ", "html_url": "https://example.com/r/x" }"#;
        let err = parse_release_json(body).unwrap_err();
        assert!(err.to_string().contains("tag_name"));
    }

    #[test]
    fn rejects_empty_html_url() {
        let body = r#"{ "tag_name": "v1.0.0", "html_url": "" }"#;
        let err = parse_release_json(body).unwrap_err();
        assert!(err.to_string().contains("html_url"));
    }

    #[test]
    fn trims_whitespace_around_fields() {
        let body = r#"{ "tag_name": "  v0.3.11  ", "html_url": "  https://example.com/r/x  " }"#;
        let info = parse_release_json(body).expect("release JSON should parse");
        assert_eq!(info.tag_name, "v0.3.11");
        assert_eq!(info.html_url, "https://example.com/r/x");
    }
}
