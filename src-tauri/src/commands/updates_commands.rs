//! 版本更新檢查 Commands
//!
//! 走後端 ureq 打 GitHub Releases API，避免前端 CSP 需要放寬 connect-src。
//! 前端只透過 invoke 取得結構化資料，再自行比對版本。

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/himawaril2dev/vocalsync-studio/releases/latest";

/// GitHub API 呼叫 timeout（秒）。避免使用者網路不穩時 UI 卡住。
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// GitHub 要求所有 API 請求帶 User-Agent。
const USER_AGENT: &str = concat!("vocalsync-studio/", env!("CARGO_PKG_VERSION"));

/// 回傳給前端的精簡版 release 資訊。只保留 UI 會用到的欄位，
/// 避免把整包 GitHub JSON 塞回前端。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReleaseInfo {
    /// 例如 "v0.2.1"
    pub tag_name: String,
    /// Release 頁面 URL，前端可直接開外部瀏覽器
    pub html_url: String,
}

/// GitHub API 回傳 JSON 的最小反序列化結構。
#[derive(Debug, Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    html_url: String,
}

/// 純函式：把 GitHub API 的 JSON body 解析成 ReleaseInfo。
///
/// 拆成獨立函式是為了 unit test 能驗證解析邏輯，不用真的打網路。
pub(crate) fn parse_release_json(body: &str) -> Result<ReleaseInfo, AppError> {
    let parsed: GithubReleaseResponse = serde_json::from_str(body)
        .map_err(|e| AppError::Internal(format!("解析 GitHub API 回應失敗：{}", e)))?;

    let tag = parsed.tag_name.trim();
    let url = parsed.html_url.trim();

    if tag.is_empty() {
        return Err(AppError::Internal(
            "GitHub API 回應缺少 tag_name 欄位".to_string(),
        ));
    }
    if url.is_empty() {
        return Err(AppError::Internal(
            "GitHub API 回應缺少 html_url 欄位".to_string(),
        ));
    }

    Ok(ReleaseInfo {
        tag_name: tag.to_string(),
        html_url: url.to_string(),
    })
}

/// 檢查 GitHub 上最新 release 的資訊。
///
/// 前端收到後自行跟當前版本比對（保留原先 compareVersions 邏輯）。
#[tauri::command]
pub async fn check_latest_release() -> Result<ReleaseInfo, AppError> {
    // ureq 的同步 call 丟到 blocking thread，避免卡住 Tauri 的 async runtime。
    tauri::async_runtime::spawn_blocking(fetch_latest_release_blocking)
        .await
        .map_err(|e| AppError::Internal(format!("背景工作執行失敗：{}", e)))?
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
        .map_err(|e| AppError::Internal(format!("無法連線到 GitHub：{}", e)))?;

    let body = resp
        .into_string()
        .map_err(|e| AppError::Internal(format!("讀取 GitHub 回應失敗：{}", e)))?;

    parse_release_json(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_release_json() {
        let body = r#"{
            "tag_name": "v0.2.2",
            "html_url": "https://github.com/himawaril2dev/vocalsync-studio/releases/tag/v0.2.2"
        }"#;

        let info = parse_release_json(body).expect("應該解析成功");
        assert_eq!(info.tag_name, "v0.2.2");
        assert_eq!(
            info.html_url,
            "https://github.com/himawaril2dev/vocalsync-studio/releases/tag/v0.2.2"
        );
    }

    #[test]
    fn ignores_extra_fields_in_json() {
        // GitHub 實際回傳有 ~30 個欄位，確認多的欄位會被 serde 忽略
        let body = r#"{
            "tag_name": "v0.3.0",
            "html_url": "https://example.com/r/v0.3.0",
            "name": "Some Release",
            "body": "changelog here",
            "prerelease": false,
            "draft": false,
            "published_at": "2026-04-19T08:00:00Z",
            "author": { "login": "someone" }
        }"#;

        let info = parse_release_json(body).expect("多欄位不該讓解析失敗");
        assert_eq!(info.tag_name, "v0.3.0");
    }

    #[test]
    fn rejects_malformed_json() {
        let body = "not a json at all";
        let err = parse_release_json(body).unwrap_err();
        assert!(err.to_string().contains("解析"));
    }

    #[test]
    fn rejects_missing_tag_name() {
        let body = r#"{ "html_url": "https://example.com/r/x" }"#;
        let err = parse_release_json(body).unwrap_err();
        // 缺欄位是 serde 層級的錯，會歸類為解析失敗
        assert!(err.to_string().contains("解析"));
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
        let body = r#"{ "tag_name": "  v0.2.2  ", "html_url": "  https://example.com/r/x  " }"#;
        let info = parse_release_json(body).expect("應該解析成功");
        assert_eq!(info.tag_name, "v0.2.2");
        assert_eq!(info.html_url, "https://example.com/r/x");
    }
}
