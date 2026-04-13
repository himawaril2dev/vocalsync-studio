//! YouTube 下載 Commands
//!
//! 前端可呼叫：
//! - `check_download_tools`：檢查 yt-dlp / FFmpeg 是否已安裝
//! - `detect_url_type`：偵測 URL 類型（影片/播放清單/頻道）
//! - `start_download`：開始下載（背景執行，透過 event 推送進度）
//! - `cancel_download`：取消下載

use crate::core::ytdlp_engine::{
    self, DownloadRequest, DownloadResult, ToolStatus, UrlType,
};
use crate::error::AppError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, State};

/// 全域取消旗標，供前端呼叫 cancel_download 時使用。
pub struct DownloadCancelFlag(pub Arc<AtomicBool>);

// ── Tauri Commands ──────────────────────────────────────────────

/// 檢查 yt-dlp 與 FFmpeg 的安裝狀態。
#[tauri::command]
pub fn check_download_tools() -> ToolStatus {
    ytdlp_engine::check_tool_status()
}

/// 偵測 YouTube URL 的類型。
#[tauri::command]
pub fn detect_download_url_type(url: String) -> String {
    let url_type = ytdlp_engine::detect_url_type(&url);
    match url_type {
        UrlType::Video => "video".into(),
        UrlType::Playlist => "playlist".into(),
        UrlType::Channel => "channel".into(),
    }
}

/// 開始下載。在背景執行緒中執行 yt-dlp，透過 `ytdlp:progress` event 推送進度。
#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    cancel_flag: State<'_, DownloadCancelFlag>,
    request: DownloadRequest,
) -> Result<DownloadResult, AppError> {
    // 重設取消旗標
    cancel_flag.0.store(false, Ordering::Relaxed);
    let flag = cancel_flag.0.clone();

    // 在 blocking 執行緒中跑（yt-dlp 是 subprocess，會阻塞）
    let result = tauri::async_runtime::spawn_blocking(move || {
        ytdlp_engine::run_download(&app, request, flag)
    })
    .await
    .map_err(|e| AppError::Internal(format!("下載任務失敗: {}", e)))?;

    result
}

/// 取消目前的下載。
#[tauri::command]
pub fn cancel_download(cancel_flag: State<'_, DownloadCancelFlag>) {
    cancel_flag.0.store(true, Ordering::Relaxed);
}

/// 取得預設下載目錄（桌面或下載資料夾）。
#[tauri::command]
pub fn get_default_download_dir() -> Option<String> {
    dirs_next::download_dir()
        .or_else(dirs_next::desktop_dir)
        .map(|p| p.to_string_lossy().to_string())
}

/// 自動下載 yt-dlp 到 app 資料夾。
/// 透過 `ytdlp:install_progress` event 推送進度。
#[tauri::command]
pub async fn install_ytdlp(app: AppHandle) -> Result<String, AppError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        ytdlp_engine::download_ytdlp(&app)
    })
    .await
    .map_err(|e| AppError::Internal(format!("安裝任務失敗: {}", e)))?;

    result.map(|p| p.to_string_lossy().to_string())
}

/// 自動下載 FFmpeg 到 app 資料夾。
/// 透過 `ffmpeg:install_progress` event 推送進度。
#[tauri::command]
pub async fn install_ffmpeg(app: AppHandle) -> Result<String, AppError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        ytdlp_engine::download_ffmpeg(&app)
    })
    .await
    .map_err(|e| AppError::Internal(format!("安裝任務失敗: {}", e)))?;

    result.map(|p| p.to_string_lossy().to_string())
}
