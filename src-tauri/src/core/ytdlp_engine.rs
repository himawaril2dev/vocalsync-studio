//! YouTube 下載引擎（yt-dlp CLI 包裝）
//!
//! 透過 subprocess 呼叫系統上的 yt-dlp CLI，實現影片/音訊下載。
//! 使用 `--newline` 取得結構化進度。
//!
//! # 外部依賴
//!
//! - **yt-dlp**：自動下載到 app 資料夾，或使用系統 PATH 上的版本
//! - **FFmpeg**：影音合併/轉碼需要（yt-dlp 會自動呼叫）

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows: 隱藏 cmd 視窗
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// yt-dlp 固定版本（避免 supply-chain 攻擊使用 latest）
const YTDLP_VERSION: &str = "2025.03.31";

/// yt-dlp GitHub Releases 下載 URL（Windows 獨立執行檔）
#[cfg(windows)]
const YTDLP_DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/download/2025.03.31/yt-dlp.exe";
/// yt-dlp SHA-256（Windows exe）— 來自官方 SHA2-256SUMS
#[cfg(windows)]
const YTDLP_SHA256: &str = "5374c46da65bbe661d3220a23646c785c3e53264485edc31436e8dba3889337c";

/// yt-dlp GitHub Releases 下載 URL（Linux / macOS）
#[cfg(not(windows))]
const YTDLP_DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/download/2025.03.31/yt-dlp";
#[cfg(not(windows))]
const YTDLP_SHA256: &str = "0e8bc5558efce5ae2a6397710eed72fd8d434e45904e4fe029dd21c610a95d4d";

/// yt-dlp 執行檔名
#[cfg(windows)]
const YTDLP_EXE_NAME: &str = "yt-dlp.exe";

#[cfg(not(windows))]
const YTDLP_EXE_NAME: &str = "yt-dlp";

/// FFmpeg 固定版本建置 URL（Windows，GyanD essentials build）
/// 約 80 MB，解壓後只保留 ffmpeg.exe + ffprobe.exe
#[cfg(windows)]
const FFMPEG_DOWNLOAD_URL: &str =
    "https://github.com/GyanD/codexffmpeg/releases/download/2026-03-30-git-e54e117998/ffmpeg-2026-03-30-git-e54e117998-essentials_build.zip";
#[cfg(windows)]
const FFMPEG_ZIP_SHA256: &str = "e1872e1eab6a280da863f6336fa719ed13368dc294cf8010c81d3f63144c45b7";

// ── 型別定義 ─────────────────────────────────────────────────────

/// URL 類型
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlType {
    Video,
    Playlist,
    Channel,
}

/// 下載格式
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadFormat {
    Video,
    Mp3,
    M4a,
    Wav,
    SubtitleOnly,
}

/// 影片畫質
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    Best,
    #[serde(rename = "1080p")]
    Q1080p,
    #[serde(rename = "720p")]
    Q720p,
    #[serde(rename = "480p")]
    Q480p,
    #[serde(rename = "360p")]
    Q360p,
}

/// 字幕語言
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleLang {
    TraditionalChinese,
    SimplifiedChinese,
    English,
    Japanese,
    All,
    None,
}

/// 下載進度（推送給前端的 payload）
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    /// 百分比（0.0 ~ 100.0）
    pub percent: f64,
    /// 目前檔案名稱
    pub filename: String,
    /// 狀態："downloading" | "postprocessing" | "finished" | "error"
    pub status: String,
    /// 已下載大小（人類可讀）
    pub downloaded: String,
    /// 總大小（人類可讀）
    pub total: String,
    /// 下載速度
    pub speed: String,
    /// 預估剩餘時間
    pub eta: String,
}

/// 下載完成結果
#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub success: bool,
    pub message: String,
    pub output_dir: String,
    /// 下載完成後在 output_dir 中找到的字幕檔案路徑
    pub subtitle_paths: Vec<String>,
}

/// 下載請求參數
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub format: DownloadFormat,
    pub quality: VideoQuality,
    pub subtitle_lang: SubtitleLang,
    pub output_dir: String,
}

// ── 工具偵測 ─────────────────────────────────────────────────────

/// 取得 app 內部的 bin 資料夾路徑（用於存放 yt-dlp 執行檔）。
///
/// 位置：`%APPDATA%/com.vocalsync.studio/bin/`（Windows）
///       `~/.local/share/com.vocalsync.studio/bin/`（Linux）
///       `~/Library/Application Support/com.vocalsync.studio/bin/`（macOS）
pub fn get_app_bin_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|d| d.join("com.vocalsync.studio").join("bin"))
}

fn find_tool_in_dir(dir: &std::path::Path, exe_name: &str) -> Option<PathBuf> {
    let candidate = dir.join(exe_name);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn find_tool_next_to_current_exe(exe_name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    find_tool_in_dir(dir, exe_name)
}

/// 搜尋 yt-dlp 可執行檔。
///
/// 搜尋順序：
/// 1. app 資料夾（自動下載的版本）
/// 2. 系統 PATH
/// 3. 應用程式目錄（portable / 打包模式）
pub fn find_ytdlp() -> Option<PathBuf> {
    // 1. app bin 資料夾
    if let Some(bin_dir) = get_app_bin_dir() {
        if let Some(candidate) = find_tool_in_dir(&bin_dir, YTDLP_EXE_NAME) {
            return Some(candidate);
        }
    }

    // 2. 系統 PATH
    if let Ok(path) = which::which("yt-dlp") {
        return Some(path);
    }

    // 3. 應用程式目錄（portable / 打包模式）
    find_tool_next_to_current_exe(YTDLP_EXE_NAME)
}

/// 搜尋系統上的 FFmpeg 可執行檔。
pub fn find_ffmpeg() -> Option<PathBuf> {
    // 1. app bin 資料夾
    if let Some(bin_dir) = get_app_bin_dir() {
        if let Some(candidate) = find_tool_in_dir(
            &bin_dir,
            if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            },
        ) {
            return Some(candidate);
        }
    }

    // 2. 系統 PATH
    if let Ok(path) = which::which("ffmpeg") {
        return Some(path);
    }

    // 3. 應用程式目錄（打包模式）
    if let Some(candidate) = find_tool_next_to_current_exe(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }) {
        return Some(candidate);
    }

    // 4. Windows 常見路徑
    #[cfg(target_os = "windows")]
    {
        let common = [
            r"C:\ffmpeg\bin\ffmpeg.exe",
            r"C:\Program Files\ffmpeg\bin\ffmpeg.exe",
            r"C:\Program Files (x86)\ffmpeg\bin\ffmpeg.exe",
        ];
        for p in common {
            let path = PathBuf::from(p);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

/// 取得 yt-dlp 版本字串。
pub fn get_ytdlp_version() -> Option<String> {
    let ytdlp = find_ytdlp()?;
    let mut cmd = Command::new(ytdlp);
    cmd.arg("--version");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output().ok()?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// 取得 FFmpeg 版本字串（第一行）。
pub fn get_ffmpeg_version() -> Option<String> {
    let ffmpeg = find_ffmpeg()?;
    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-version");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|s| s.to_string())
}

/// 工具狀態（給前端顯示）
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub ytdlp_available: bool,
    pub ytdlp_version: Option<String>,
    pub ytdlp_path: Option<String>,
    pub ffmpeg_available: bool,
    pub ffmpeg_version: Option<String>,
}

pub fn check_tool_status() -> ToolStatus {
    let ytdlp_path = find_ytdlp();
    ToolStatus {
        ytdlp_available: ytdlp_path.is_some(),
        ytdlp_version: get_ytdlp_version(),
        ytdlp_path: ytdlp_path.map(|p| p.to_string_lossy().to_string()),
        ffmpeg_available: find_ffmpeg().is_some(),
        ffmpeg_version: get_ffmpeg_version(),
    }
}

// ── yt-dlp 自動下載 ─────────────────────────────────────────────

/// 下載進度（安裝 yt-dlp 時推送）
#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    pub percent: f64,
    pub status: String,
    pub message: String,
}

/// 驗證檔案的 SHA-256 hash。空字串的 expected_hash 表示跳過驗證（首次部署用）。
fn verify_sha256(path: &std::path::Path, expected_hash: &str) -> Result<(), AppError> {
    if expected_hash.is_empty() {
        // hash 尚未設定，記錄實際 hash 供開發者填入
        let mut file = std::fs::File::open(path).map_err(AppError::Io)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = file.read(&mut buf).map_err(AppError::Io)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let hash = format!("{:x}", hasher.finalize());
        log::warn!(
            "[security] SHA-256 驗證已跳過（hash 未設定）。實際 hash: {} ← 請填入程式碼",
            hash
        );
        return Ok(());
    }

    let mut file = std::fs::File::open(path).map_err(AppError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(AppError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_hash {
        return Err(AppError::Internal(format!(
            "SHA-256 驗證失敗！預期: {}，實際: {}。檔案可能被篡改。",
            expected_hash, actual
        )));
    }
    log::info!("[security] SHA-256 驗證通過: {}", expected_hash);
    Ok(())
}

/// 從 GitHub Releases 下載 yt-dlp 到 app bin 資料夾。
///
/// 透過 `ytdlp:install_progress` event 推送進度。
pub fn download_ytdlp(app: &AppHandle) -> Result<PathBuf, AppError> {
    let bin_dir =
        get_app_bin_dir().ok_or_else(|| AppError::Internal("無法取得應用程式資料目錄".into()))?;

    // 建立目錄
    std::fs::create_dir_all(&bin_dir).map_err(|e| AppError::Io(e))?;

    let target_path = bin_dir.join(YTDLP_EXE_NAME);

    let _ = app.emit(
        "ytdlp:install_progress",
        &InstallProgress {
            percent: 0.0,
            status: "downloading".into(),
            message: "正在從 GitHub 下載 yt-dlp...".into(),
        },
    );

    log::info!(
        "[ytdlp] 下載 yt-dlp: {} -> {:?}",
        YTDLP_DOWNLOAD_URL,
        target_path
    );

    // 使用 ureq 下載（專案已有此依賴）
    let resp = ureq::get(YTDLP_DOWNLOAD_URL)
        .call()
        .map_err(|e| AppError::Internal(format!("下載 yt-dlp 失敗: {}", e)))?;

    let content_length = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // 寫入暫存檔再改名（避免不完整的檔案）
    let tmp_path = target_path.with_extension("tmp");
    let mut _tmp_guard = TempFileGuard::new(tmp_path.clone());
    let mut file = std::fs::File::create(&tmp_path).map_err(|e| AppError::Io(e))?;

    let mut reader = resp.into_reader();
    let mut buf = [0u8; 65536];
    let mut downloaded: u64 = 0;
    let mut last_reported_pct: f64 = 0.0;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| AppError::Internal(format!("下載讀取失敗: {}", e)))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| AppError::Io(e))?;
        downloaded += n as u64;

        // 每 5% 報告一次
        if content_length > 0 {
            let pct = (downloaded as f64 / content_length as f64) * 100.0;
            if pct - last_reported_pct >= 5.0 {
                last_reported_pct = pct;
                let _ = app.emit(
                    "ytdlp:install_progress",
                    &InstallProgress {
                        percent: pct,
                        status: "downloading".into(),
                        message: format!(
                            "下載中... {:.1} MB / {:.1} MB",
                            downloaded as f64 / 1_048_576.0,
                            content_length as f64 / 1_048_576.0,
                        ),
                    },
                );
            }
        }
    }

    drop(file);

    // 重命名（成功後解除 guard）
    if target_path.exists() {
        std::fs::remove_file(&target_path).map_err(|e| AppError::Io(e))?;
    }
    std::fs::rename(&tmp_path, &target_path).map_err(|e| AppError::Io(e))?;
    _tmp_guard.disarm(); // 重命名成功，不再刪除

    // SHA-256 驗證
    verify_sha256(&target_path, YTDLP_SHA256)?;

    // Unix: 設定可執行權限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| AppError::Io(e))?;
    }

    let _ = app.emit(
        "ytdlp:install_progress",
        &InstallProgress {
            percent: 100.0,
            status: "finished".into(),
            message: format!("yt-dlp {} 安裝完成", YTDLP_VERSION),
        },
    );

    log::info!(
        "[ytdlp] yt-dlp {} 已下載至: {:?}",
        YTDLP_VERSION,
        target_path
    );

    Ok(target_path)
}

/// 從 GitHub 下載 FFmpeg 靜態建置並解壓到 app bin 資料夾。
///
/// 只保留 `ffmpeg.exe` 和 `ffprobe.exe`（Windows），其餘捨棄。
/// 透過 `ffmpeg:install_progress` event 推送進度。
#[cfg(windows)]
pub fn download_ffmpeg(app: &AppHandle) -> Result<PathBuf, AppError> {
    let bin_dir =
        get_app_bin_dir().ok_or_else(|| AppError::Internal("無法取得應用程式資料目錄".into()))?;

    std::fs::create_dir_all(&bin_dir).map_err(|e| AppError::Io(e))?;

    let _ = app.emit(
        "ffmpeg:install_progress",
        &InstallProgress {
            percent: 0.0,
            status: "downloading".into(),
            message: "正在從 GitHub 下載 FFmpeg（約 80 MB）...".into(),
        },
    );

    log::info!("[ffmpeg] 下載 FFmpeg: {}", FFMPEG_DOWNLOAD_URL);

    // 下載 zip 到暫存檔
    let tmp_zip = bin_dir.join("ffmpeg-download.zip");
    let _zip_guard = TempFileGuard::new(tmp_zip.clone());

    let resp = ureq::get(FFMPEG_DOWNLOAD_URL)
        .call()
        .map_err(|e| AppError::Internal(format!("下載 FFmpeg 失敗: {}", e)))?;

    let content_length = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tmp_zip).map_err(|e| AppError::Io(e))?;

    let mut buf = [0u8; 65536];
    let mut downloaded: u64 = 0;
    let mut last_reported_pct: f64 = 0.0;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| AppError::Internal(format!("下載讀取失敗: {}", e)))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| AppError::Io(e))?;
        downloaded += n as u64;

        if content_length > 0 {
            // 下載佔總進度的 0~80%
            let pct = (downloaded as f64 / content_length as f64) * 80.0;
            if pct - last_reported_pct >= 3.0 {
                last_reported_pct = pct;
                let _ = app.emit(
                    "ffmpeg:install_progress",
                    &InstallProgress {
                        percent: pct,
                        status: "downloading".into(),
                        message: format!(
                            "下載中... {:.1} MB / {:.1} MB",
                            downloaded as f64 / 1_048_576.0,
                            content_length as f64 / 1_048_576.0,
                        ),
                    },
                );
            }
        }
    }

    drop(file);

    // 解壓 — 只提取 ffmpeg.exe 和 ffprobe.exe
    let _ = app.emit(
        "ffmpeg:install_progress",
        &InstallProgress {
            percent: 85.0,
            status: "downloading".into(),
            message: "解壓縮中...".into(),
        },
    );

    // SHA-256 驗證（解壓前）
    verify_sha256(&tmp_zip, FFMPEG_ZIP_SHA256)?;

    log::info!("[ffmpeg] 解壓: {:?}", tmp_zip);

    let zip_file = std::fs::File::open(&tmp_zip).map_err(|e| AppError::Io(e))?;
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|e| AppError::Internal(format!("zip 開啟失敗: {}", e)))?;

    let targets = ["ffmpeg.exe", "ffprobe.exe"];
    let mut extracted_count = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Internal(format!("zip 讀取失敗: {}", e)))?;

        let entry_name = entry.name().to_string();

        // zip 內的結構：ffmpeg-<version>-essentials_build/bin/ffmpeg.exe
        for target in &targets {
            if entry_name.ends_with(&format!("bin/{}", target)) {
                let out_path = bin_dir.join(target);
                let mut out_file = std::fs::File::create(&out_path).map_err(|e| AppError::Io(e))?;
                std::io::copy(&mut entry, &mut out_file).map_err(|e| AppError::Io(e))?;
                log::info!("[ffmpeg] 解壓: {} -> {:?}", entry_name, out_path);
                extracted_count += 1;
                break;
            }
        }

        if extracted_count >= targets.len() {
            break;
        }
    }

    if extracted_count == 0 {
        return Err(AppError::Internal(
            "zip 中找不到 ffmpeg.exe / ffprobe.exe".into(),
        ));
    }

    let ffmpeg_path = bin_dir.join("ffmpeg.exe");

    let _ = app.emit(
        "ffmpeg:install_progress",
        &InstallProgress {
            percent: 100.0,
            status: "finished".into(),
            message: "FFmpeg 安裝完成".into(),
        },
    );

    log::info!("[ffmpeg] FFmpeg 已安裝至: {:?}", bin_dir);

    Ok(ffmpeg_path)
}

/// 非 Windows 平台的 FFmpeg 下載提示（建議用系統套件管理器）。
#[cfg(not(windows))]
pub fn download_ffmpeg(_app: &AppHandle) -> Result<PathBuf, AppError> {
    Err(AppError::Internal(
        "Linux / macOS 請使用套件管理器安裝 FFmpeg：apt install ffmpeg / brew install ffmpeg"
            .into(),
    ))
}

// ── URL 偵測 ─────────────────────────────────────────────────────

/// 偵測 YouTube URL 類型。
pub fn detect_url_type(url: &str) -> UrlType {
    if url.contains("playlist?list=") || url.contains("&list=") {
        return UrlType::Playlist;
    }

    let lower = url.to_lowercase();
    let channel_patterns = ["/@", "/channel/", "/user/", "/c/"];
    for pat in channel_patterns {
        if lower.contains(pat) && !lower.contains("watch?v=") {
            return UrlType::Channel;
        }
    }

    UrlType::Video
}

// ── 格式字串 ─────────────────────────────────────────────────────

/// 產生 yt-dlp 的 video format selector。
fn build_video_format(quality: VideoQuality) -> String {
    match quality {
        VideoQuality::Best => {
            "bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best".to_string()
        }
        q => {
            let h = match q {
                VideoQuality::Q1080p => 1080,
                VideoQuality::Q720p => 720,
                VideoQuality::Q480p => 480,
                VideoQuality::Q360p => 360,
                _ => unreachable!(),
            };
            format!(
                "bestvideo[height<={h}][ext=mp4]+bestaudio[ext=m4a]\
                 /bestvideo[height<={h}]+bestaudio\
                 /best[height<={h}]/best"
            )
        }
    }
}

/// 產生 yt-dlp 的 audio format selector。
fn build_audio_format() -> &'static str {
    "bestaudio/best"
}

/// 取得字幕語言 args。
fn subtitle_args(lang: &SubtitleLang) -> Vec<String> {
    match lang {
        SubtitleLang::None => vec![],
        SubtitleLang::All => vec![
            "--write-subs".into(),
            "--write-auto-subs".into(),
            "--sub-format".into(),
            "srt".into(),
        ],
        _ => {
            let codes = match lang {
                SubtitleLang::TraditionalChinese => "zh-TW,zh-Hant",
                SubtitleLang::SimplifiedChinese => "zh-Hans,zh-CN",
                SubtitleLang::English => "en",
                SubtitleLang::Japanese => "ja",
                _ => return vec![],
            };
            vec![
                "--write-subs".into(),
                "--write-auto-subs".into(),
                "--sub-format".into(),
                "srt".into(),
                "--sub-langs".into(),
                codes.into(),
            ]
        }
    }
}

// ── 輔助工具 ─────────────────────────────────────────────────────

/// 驗證 URL 基本格式（防止 shell injection 和無效輸入）。
fn validate_url(url: &str) -> Result<(), AppError> {
    if url.len() > 2048 {
        return Err(AppError::Audio("URL 過長（上限 2048 字元）".into()));
    }
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(AppError::Audio(
            "URL 必須以 http:// 或 https:// 開頭".into(),
        ));
    }
    if url.contains('\0') {
        return Err(AppError::Audio("URL 包含無效字元".into()));
    }
    // 🔴 Codex 安全審查 P1 #2：擋 yt-dlp option injection。
    // 正常的 URL 不會含空白字元；若允許，攻擊者可傳 `http://x --exec "calc.exe"`
    // 讓 yt-dlp 把第二段當成獨立 argument 執行 hook 指令。
    if url.chars().any(char::is_whitespace) {
        return Err(AppError::Audio("URL 不可包含空白字元".into()));
    }
    Ok(())
}

/// 暫存檔 RAII guard — drop 時自動刪除檔案。
struct TempFileGuard {
    path: PathBuf,
    disarmed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            disarmed: false,
        }
    }

    /// 解除 guard（檔案不再自動刪除）。
    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.disarmed && self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ── 下載執行 ─────────────────────────────────────────────────────

/// 建構 yt-dlp CLI 引數列表。
fn build_args(req: &DownloadRequest) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--newline".into(),
        "--no-colors".into(),
        "--encoding".into(),
        "utf-8".into(),
    ];

    // FFmpeg 路徑
    if let Some(ffmpeg) = find_ffmpeg() {
        args.push("--ffmpeg-location".into());
        args.push(ffmpeg.to_string_lossy().to_string());
    }

    match req.format {
        DownloadFormat::Video => {
            args.push("-f".into());
            args.push(build_video_format(req.quality));
            args.push("--merge-output-format".into());
            args.push("mp4".into());
        }
        DownloadFormat::Mp3 | DownloadFormat::M4a | DownloadFormat::Wav => {
            args.push("-f".into());
            args.push(build_audio_format().into());
            args.push("-x".into());
            args.push("--audio-format".into());
            args.push(match req.format {
                DownloadFormat::Mp3 => "mp3".into(),
                DownloadFormat::M4a => "m4a".into(),
                DownloadFormat::Wav => "wav".into(),
                _ => unreachable!(),
            });
            // WAV 是無損格式，不需要指定位元率
            if req.format != DownloadFormat::Wav {
                args.push("--audio-quality".into());
                args.push("192K".into());
            }
        }
        DownloadFormat::SubtitleOnly => {
            // 不下載影音，只抓字幕
            args.push("--skip-download".into());
        }
    }

    // 字幕
    if req.format == DownloadFormat::SubtitleOnly {
        // 只下載字幕模式：強制啟用字幕下載，使用指定語言或全部
        let effective_lang = if req.subtitle_lang == SubtitleLang::None {
            &SubtitleLang::All
        } else {
            &req.subtitle_lang
        };
        args.extend(subtitle_args(effective_lang));
    } else {
        // 一般模式：有選字幕才下載
        args.extend(subtitle_args(&req.subtitle_lang));
    }

    // 輸出模板
    let url_type = detect_url_type(&req.url);
    let outtmpl = if url_type == UrlType::Channel {
        format!("{}/%(uploader)s/%(title)s.%(ext)s", req.output_dir)
    } else {
        format!("{}/%(title)s.%(ext)s", req.output_dir)
    };
    args.push("-o".into());
    args.push(outtmpl);

    // 忽略錯誤（播放清單/頻道中的個別影片失敗不中斷整體）
    if url_type != UrlType::Video {
        args.push("--ignore-errors".into());
    }

    args.push(req.url.clone());
    args
}

/// 在背景執行 yt-dlp 下載，透過 Tauri events 推送進度。
///
/// `cancel_flag` 可用於中途取消。
pub fn run_download(
    app: &AppHandle,
    req: DownloadRequest,
    cancel_flag: Arc<AtomicBool>,
) -> Result<DownloadResult, AppError> {
    // 驗證 URL
    validate_url(&req.url)?;

    let ytdlp = find_ytdlp()
        .ok_or_else(|| AppError::Audio("找不到 yt-dlp。請點擊「自動安裝」或手動安裝".into()))?;

    let args = build_args(&req);
    log::info!(
        "[ytdlp] 啟動下載，URL 類型: {:?}",
        detect_url_type(&req.url)
    );

    let mut cmd = Command::new(&ytdlp);
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Audio(format!("yt-dlp 啟動失敗: {}", e)))?;

    // 在獨立執行緒中讀取 stderr（避免 race condition）
    let stderr = child.stderr.take();
    let stderr_handle = std::thread::spawn(move || -> String {
        match stderr {
            Some(s) => {
                let reader = BufReader::new(s);
                reader
                    .lines()
                    .filter_map(|l| l.ok())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            None => String::new(),
        }
    });

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let mut last_filename = String::new();

    for line in reader.lines() {
        // 檢查取消
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait(); // 確保 process 完全終止
            return Ok(DownloadResult {
                success: false,
                message: "下載已取消".into(),
                output_dir: req.output_dir,
                subtitle_paths: Vec::new(),
            });
        }

        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if let Some(progress) = parse_progress_line(&line, &mut last_filename) {
            let _ = app.emit("ytdlp:progress", &progress);
        }
    }

    let status = child
        .wait()
        .map_err(|e| AppError::Audio(format!("yt-dlp 執行錯誤: {}", e)))?;

    // 等待 stderr 執行緒完成
    let stderr_output = stderr_handle.join().unwrap_or_default();

    if status.success() {
        let _ = app.emit(
            "ytdlp:progress",
            &DownloadProgress {
                percent: 100.0,
                filename: last_filename,
                status: "finished".into(),
                downloaded: String::new(),
                total: String::new(),
                speed: String::new(),
                eta: String::new(),
            },
        );

        // 掃描下載目錄中的字幕檔案
        let subtitle_paths = crate::core::lyrics_parser::find_subtitle_files(&req.output_dir);

        Ok(DownloadResult {
            success: true,
            message: "下載完成".into(),
            output_dir: req.output_dir,
            subtitle_paths,
        })
    } else {
        let msg = if stderr_output.is_empty() {
            format!("yt-dlp 結束碼: {}", status.code().unwrap_or(-1))
        } else {
            // 取最後幾行有意義的錯誤
            let lines: Vec<&str> = stderr_output.lines().rev().take(3).collect();
            lines.into_iter().rev().collect::<Vec<_>>().join("\n")
        };
        Ok(DownloadResult {
            success: false,
            message: msg,
            output_dir: req.output_dir,
            subtitle_paths: Vec::new(),
        })
    }
}

/// 解析 yt-dlp stdout 的進度行。
///
/// yt-dlp 的 `--newline` 模式會輸出類似：
/// ```text
/// [download]   5.2% of  45.23MiB at  2.30MiB/s ETA 00:18
/// [download] 100% of  45.23MiB in 00:19
/// [download] Destination: output/song.mp4
/// [ExtractAudio] Destination: output/song.mp3
/// ```
fn parse_progress_line(line: &str, last_filename: &mut String) -> Option<DownloadProgress> {
    let trimmed = line.trim();

    // 解析 [download] 進度行
    if trimmed.starts_with("[download]") {
        let content = trimmed.strip_prefix("[download]")?.trim();

        // "Destination: ..." — 記住檔名
        if let Some(dest) = content.strip_prefix("Destination:") {
            *last_filename = dest.trim().to_string();
            return None;
        }

        // 百分比行："  5.2% of  45.23MiB at  2.30MiB/s ETA 00:18"
        if let Some(pct_end) = content.find('%') {
            let pct_str = content[..pct_end].trim();
            if let Ok(pct) = pct_str.parse::<f64>() {
                let rest = &content[pct_end + 1..];

                let total = extract_field(rest, "of ", " ");
                let speed = extract_field(rest, "at ", " ");
                let eta = extract_field(rest, "ETA ", "");
                let downloaded = extract_field(rest, "in ", "");

                return Some(DownloadProgress {
                    percent: pct,
                    filename: last_filename.clone(),
                    status: "downloading".into(),
                    downloaded: downloaded.unwrap_or_default(),
                    total: total.unwrap_or_default(),
                    speed: speed.unwrap_or_default(),
                    eta: eta.unwrap_or_default(),
                });
            }
        }
    }

    // [ExtractAudio] / [Merger] / 其他 postprocessor
    if trimmed.starts_with("[ExtractAudio]")
        || trimmed.starts_with("[Merger]")
        || trimmed.starts_with("[ffmpeg]")
    {
        if let Some(dest) = trimmed.split("Destination:").nth(1) {
            *last_filename = dest.trim().to_string();
        }
        return Some(DownloadProgress {
            percent: 100.0,
            filename: last_filename.clone(),
            status: "postprocessing".into(),
            downloaded: String::new(),
            total: String::new(),
            speed: String::new(),
            eta: String::new(),
        });
    }

    None
}

/// 從字串中提取 "prefix...delimiter" 之間的子字串。
fn extract_field(s: &str, prefix: &str, delimiter: &str) -> Option<String> {
    let start = s.find(prefix)? + prefix.len();
    let rest = &s[start..];
    let end = if delimiter.is_empty() {
        rest.len()
    } else {
        rest.find(delimiter).unwrap_or(rest.len())
    };
    let value = rest[..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

// ── 測試 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_video_url() {
        assert_eq!(
            detect_url_type("https://www.youtube.com/watch?v=abc123"),
            UrlType::Video
        );
        assert_eq!(detect_url_type("https://youtu.be/abc123"), UrlType::Video);
    }

    #[test]
    fn detects_playlist_url() {
        assert_eq!(
            detect_url_type("https://www.youtube.com/playlist?list=PLxyz"),
            UrlType::Playlist
        );
        assert_eq!(
            detect_url_type("https://www.youtube.com/watch?v=abc&list=PLxyz"),
            UrlType::Playlist
        );
    }

    #[test]
    fn detects_channel_url() {
        assert_eq!(
            detect_url_type("https://www.youtube.com/@username"),
            UrlType::Channel
        );
        assert_eq!(
            detect_url_type("https://www.youtube.com/channel/UCxyz"),
            UrlType::Channel
        );
    }

    #[test]
    fn channel_with_video_is_video() {
        // /@username/watch?v=... 應該視為單一影片
        assert_eq!(
            detect_url_type("https://www.youtube.com/@user/watch?v=abc"),
            UrlType::Video
        );
    }

    #[test]
    fn video_format_best() {
        let fmt = build_video_format(VideoQuality::Best);
        assert!(fmt.contains("bestvideo"));
        assert!(fmt.contains("bestaudio"));
    }

    #[test]
    fn video_format_720p() {
        let fmt = build_video_format(VideoQuality::Q720p);
        assert!(fmt.contains("height<=720"));
    }

    #[test]
    fn parse_progress_download_line() {
        let mut filename = String::new();
        let line = "[download]  42.5% of  128.30MiB at  5.20MiB/s ETA 00:15";
        let result = parse_progress_line(line, &mut filename);
        assert!(result.is_some());
        let p = result.unwrap();
        assert!((p.percent - 42.5).abs() < 0.1);
        assert_eq!(p.status, "downloading");
    }

    #[test]
    fn parse_progress_destination_line() {
        let mut filename = String::new();
        let line = "[download] Destination: output/my song.mp4";
        let result = parse_progress_line(line, &mut filename);
        assert!(result.is_none()); // Destination 行不產生進度
        assert_eq!(filename, "output/my song.mp4");
    }

    #[test]
    fn parse_postprocessing_line() {
        let mut filename = "song.webm".to_string();
        let line = "[ExtractAudio] Destination: output/song.mp3";
        let result = parse_progress_line(line, &mut filename);
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, "postprocessing");
        assert_eq!(filename, "output/song.mp3");
    }

    #[test]
    fn subtitle_args_none_is_empty() {
        assert!(subtitle_args(&SubtitleLang::None).is_empty());
    }

    #[test]
    fn subtitle_args_has_lang_codes() {
        let args = subtitle_args(&SubtitleLang::TraditionalChinese);
        assert!(args.contains(&"--write-subs".to_string()));
        assert!(args.iter().any(|a| a.contains("zh-TW")));
    }

    #[test]
    fn validates_url_rejects_invalid() {
        assert!(validate_url("not-a-url").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("https://example.com/\0bad").is_err());
    }

    #[test]
    fn validates_url_accepts_valid() {
        assert!(validate_url("https://www.youtube.com/watch?v=abc123").is_ok());
        assert!(validate_url("http://youtu.be/abc").is_ok());
    }

    #[test]
    fn validates_url_rejects_too_long() {
        let long_url = format!("https://example.com/{}", "a".repeat(2100));
        assert!(validate_url(&long_url).is_err());
    }

    #[test]
    fn temp_file_guard_cleans_up() {
        let dir = std::env::temp_dir();
        let path = dir.join("vocalsync-test-guard.tmp");
        std::fs::write(&path, "test").unwrap();
        assert!(path.exists());

        {
            let _guard = TempFileGuard::new(path.clone());
            // guard drops here
        }
        assert!(!path.exists());
    }

    #[test]
    fn temp_file_guard_disarm_preserves() {
        let dir = std::env::temp_dir();
        let path = dir.join("vocalsync-test-guard-disarm.tmp");
        std::fs::write(&path, "test").unwrap();

        {
            let mut guard = TempFileGuard::new(path.clone());
            guard.disarm();
            // guard drops here, but disarmed
        }
        assert!(path.exists());
        let _ = std::fs::remove_file(&path); // cleanup
    }

    #[test]
    fn find_tool_in_dir_returns_existing_path() {
        let dir = std::env::temp_dir().join("vocalsync-find-tool-existing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(YTDLP_EXE_NAME);
        std::fs::write(&exe, b"dummy").unwrap();

        let found = find_tool_in_dir(&dir, YTDLP_EXE_NAME);
        assert_eq!(found, Some(exe.clone()));

        let _ = std::fs::remove_file(exe);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_tool_in_dir_returns_none_when_missing() {
        let dir = std::env::temp_dir().join("vocalsync-find-tool-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let found = find_tool_in_dir(&dir, YTDLP_EXE_NAME);
        assert!(found.is_none());

        let _ = std::fs::remove_dir_all(dir);
    }
}
