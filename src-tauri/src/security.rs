//! 路徑與輸入驗證工具
//!
//! 所有從前端接收字串路徑的 command 都應該呼叫這裡的驗證函式，
//! 避免前端 XSS 或其他攻擊面透過 `..` 逃逸、absolute path 偷讀系統檔、
//! 或注入 shell option（ffmpeg/yt-dlp 等 subprocess）。
//!
//! 這是「深度防禦」層：前端 dialog 通常會給出安全路徑，但 backend
//! 不能假設前端永遠誠實。

use crate::error::AppError;
use std::path::{Component, Path};

/// 檢查檔案路徑是否安全：
/// - 必須為絕對路徑（避免 cwd-relative 逃逸）
/// - 不可包含 `..` 組件（擋 parent traversal）
/// - 不可包含 NUL
/// - 不可以 `-` 開頭（避免被 subprocess 如 ffmpeg 當作 option）
pub fn validate_path_safe(path: &str) -> Result<(), AppError> {
    if path.is_empty() {
        return Err(AppError::Audio("路徑不可為空".into()));
    }
    if path.contains('\0') {
        return Err(AppError::Audio("路徑含無效字元".into()));
    }
    // subprocess argument injection 防線：-foo.mp4 會被 ffmpeg 當成 option
    if path.starts_with('-') {
        return Err(AppError::Audio("路徑不可以 '-' 開頭".into()));
    }

    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(AppError::Audio("路徑必須為絕對路徑".into()));
    }
    for comp in p.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(AppError::Audio("路徑不可包含 '..'".into()));
        }
    }
    Ok(())
}

/// 檢查「檔名前綴」（給 `export_audio(prefix)` 這類會被拼進完整路徑的使用者字串）：
/// - 不可含路徑分隔符（`/` 或 `\`）
/// - 不可含 `..`
/// - 不可含 NUL
/// - 長度限制，避免 DoS
pub fn validate_filename_prefix(prefix: &str) -> Result<(), AppError> {
    if prefix.is_empty() {
        return Err(AppError::Audio("檔名前綴不可為空".into()));
    }
    if prefix.len() > 200 {
        return Err(AppError::Audio("檔名前綴過長（上限 200 字元）".into()));
    }
    if prefix.contains('\0') || prefix.contains('/') || prefix.contains('\\') {
        return Err(AppError::Audio(
            "檔名前綴不可包含路徑分隔符或無效字元".into(),
        ));
    }
    if prefix.contains("..") {
        return Err(AppError::Audio("檔名前綴不可包含 '..'".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_path() {
        assert!(validate_path_safe("relative/path.mp4").is_err());
        assert!(validate_path_safe("./file.mp4").is_err());
    }

    #[test]
    fn rejects_parent_dir_traversal() {
        // Windows 絕對路徑含 ..
        assert!(validate_path_safe("C:\\Users\\foo\\..\\bar\\secret.txt").is_err());
        // Unix 絕對路徑含 ..
        assert!(validate_path_safe("/home/user/../etc/passwd").is_err());
    }

    #[test]
    fn rejects_dash_prefix() {
        // ffmpeg argument injection 測試
        assert!(validate_path_safe("-vf.mp4").is_err());
        assert!(validate_path_safe("--exec.wav").is_err());
    }

    #[test]
    fn rejects_nul() {
        assert!(validate_path_safe("C:\\foo\0bar.mp4").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_path_safe("").is_err());
    }

    #[test]
    fn accepts_normal_absolute_paths() {
        // 不驗證檔案是否真的存在，只看路徑格式
        #[cfg(windows)]
        assert!(validate_path_safe("C:\\Users\\himawari168\\video.mp4").is_ok());
        #[cfg(not(windows))]
        assert!(validate_path_safe("/home/user/video.mp4").is_ok());
    }

    #[test]
    fn filename_prefix_rejects_separators() {
        assert!(validate_filename_prefix("../secret").is_err());
        assert!(validate_filename_prefix("foo/bar").is_err());
        assert!(validate_filename_prefix("foo\\bar").is_err());
        assert!(validate_filename_prefix("").is_err());
        assert!(validate_filename_prefix("foo\0bar").is_err());
    }

    #[test]
    fn filename_prefix_accepts_normal_names() {
        assert!(validate_filename_prefix("vocalsync_recording").is_ok());
        assert!(validate_filename_prefix("my-song_2026-04-18").is_ok());
        assert!(validate_filename_prefix("歌曲練習").is_ok());
    }

    #[test]
    fn filename_prefix_rejects_overlength() {
        let long = "a".repeat(201);
        assert!(validate_filename_prefix(&long).is_err());
    }
}
