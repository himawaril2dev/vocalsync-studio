//! YouTube 下載引擎（yt-dlp CLI 包裝）
//!
//! 透過 subprocess 呼叫系統上的 yt-dlp CLI，實現影片/音訊下載。
//! 使用 `--newline` 取得結構化進度。
//!
//! # 外部依賴
//!
//! - **yt-dlp**：使用 SHA-256 驗證通過的 managed binary 或使用者信任的本機檔案
//! - **FFmpeg**：使用 install manifest 記錄 hash 的 managed binary 或使用者信任的本機檔案

use crate::{error::AppError, security};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
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
const YTDLP_VERSION: &str = "2026.03.17";

/// yt-dlp GitHub Releases 下載 URL（Windows 獨立執行檔）
#[cfg(windows)]
const YTDLP_DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/download/2026.03.17/yt-dlp.exe";
/// yt-dlp SHA-256（Windows exe）— 來自官方 SHA2-256SUMS
#[cfg(windows)]
const YTDLP_SHA256: &str = "3db811b366b2da47337d2fcfdfe5bbd9a258dad3f350c54974f005df115a1545";

/// yt-dlp GitHub Releases 下載 URL（Linux / macOS）
#[cfg(not(windows))]
const YTDLP_DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/download/2026.03.17/yt-dlp";
#[cfg(not(windows))]
const YTDLP_SHA256: &str = "3bda0968a01cde70d26720653003b28553c71be14dcb2e5f4c24e9921fdad745";

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

const TOOL_MANIFEST_NAME: &str = "tool-manifest.json";
const SHARED_TOOL_DIR_NAME: &str = "com.vocalsync.tools";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ToolManifest {
    ytdlp_sha256: Option<String>,
    ytdlp_path: Option<String>,
    ffmpeg_sha256: Option<String>,
    ffprobe_sha256: Option<String>,
    ffmpeg_path: Option<String>,
    ffprobe_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalFfmpegCandidate {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub ffmpeg_sha256: String,
    pub ffprobe_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalYtdlpCandidate {
    pub ytdlp_path: String,
    pub ytdlp_sha256: String,
}

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

fn app_tool_manifest_path() -> Option<PathBuf> {
    get_app_bin_dir().map(|dir| dir.join(TOOL_MANIFEST_NAME))
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn dir_is_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }

    let probe = dir.join(format!(".vocalsync-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn get_preferred_tool_dir() -> Option<PathBuf> {
    if let Some(dir) = current_exe_dir() {
        if dir_is_writable(&dir) {
            return Some(dir);
        }
    }

    get_app_bin_dir()
}

fn tool_manifest_path() -> Option<PathBuf> {
    get_preferred_tool_dir().map(|dir| dir.join(TOOL_MANIFEST_NAME))
}

fn tool_manifest_path_in_dir(dir: &Path) -> PathBuf {
    dir.join(TOOL_MANIFEST_NAME)
}

fn load_tool_manifest_from_dir(dir: &Path) -> Option<ToolManifest> {
    load_tool_manifest_from_path(&tool_manifest_path_in_dir(dir))
}

fn load_tool_manifest_from_path(path: &Path) -> Option<ToolManifest> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return None,
    };

    match serde_json::from_str::<ToolManifest>(&content) {
        Ok(manifest) => Some(manifest),
        Err(err) => {
            log::warn!(
                "[security] 工具 manifest 解析失敗，忽略既有檔案 {:?}: {}",
                path,
                err
            );
            None
        }
    }
}

fn save_tool_manifest_to_path(path: &Path, manifest: &ToolManifest) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::Io)?;
    }
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|e| AppError::Internal(format!("工具 manifest 序列化失敗: {}", e)))?;
    std::fs::write(path, content).map_err(AppError::Io)
}

fn save_tool_manifest_to_dir(dir: &Path, manifest: &ToolManifest) -> Result<(), AppError> {
    save_tool_manifest_to_path(&tool_manifest_path_in_dir(dir), manifest)
}

fn save_tool_manifest(manifest: &ToolManifest) -> Result<(), AppError> {
    let path = tool_manifest_path()
        .ok_or_else(|| AppError::Internal("無法取得工具 manifest 路徑".into()))?;
    save_tool_manifest_to_path(&path, manifest)
}

fn shared_tool_manifest_path() -> Option<PathBuf> {
    dirs_next::data_dir().map(|dir| dir.join(SHARED_TOOL_DIR_NAME).join(TOOL_MANIFEST_NAME))
}

fn load_shared_tool_manifest() -> Option<ToolManifest> {
    load_tool_manifest_from_path(&shared_tool_manifest_path()?)
}

fn save_shared_tool_manifest(manifest: &ToolManifest) -> Result<(), AppError> {
    let path = shared_tool_manifest_path()
        .ok_or_else(|| AppError::Internal("無法取得共享工具 manifest 路徑".into()))?;
    save_tool_manifest_to_path(&path, manifest)
}

fn publish_shared_ytdlp(candidate: &LocalYtdlpCandidate) {
    let mut manifest = load_shared_tool_manifest().unwrap_or_default();
    manifest.ytdlp_path = Some(candidate.ytdlp_path.clone());
    manifest.ytdlp_sha256 = Some(candidate.ytdlp_sha256.clone());
    let _ = save_shared_tool_manifest(&manifest);
}

fn publish_shared_ffmpeg(candidate: &LocalFfmpegCandidate) {
    let mut manifest = load_shared_tool_manifest().unwrap_or_default();
    manifest.ffmpeg_path = Some(candidate.ffmpeg_path.clone());
    manifest.ffprobe_path = Some(candidate.ffprobe_path.clone());
    manifest.ffmpeg_sha256 = Some(candidate.ffmpeg_sha256.clone());
    manifest.ffprobe_sha256 = Some(candidate.ffprobe_sha256.clone());
    let _ = save_shared_tool_manifest(&manifest);
}

fn find_tool_in_dir(dir: &std::path::Path, exe_name: &str) -> Option<PathBuf> {
    let candidate = dir.join(exe_name);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn canonical_existing_tool_file(path: &str, label: &str) -> Result<PathBuf, AppError> {
    if path.is_empty() {
        return Err(AppError::Audio(format!("{} 路徑不可為空", label)));
    }
    if path.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(AppError::Audio(format!("{} 路徑含無效字元", label)));
    }

    let raw_path = Path::new(path);
    if !raw_path.is_absolute() {
        return Err(AppError::Audio(format!("{} 路徑必須為絕對路徑", label)));
    }

    let canonical = raw_path.canonicalize().map_err(AppError::Io)?;
    if !canonical.is_file() {
        return Err(AppError::Audio(format!("{} 必須是既有檔案", label)));
    }

    Ok(canonical)
}

fn file_name_matches(path: &Path, allowed_names: &[&str]) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            allowed_names
                .iter()
                .any(|allowed| name.eq_ignore_ascii_case(allowed))
        })
}

fn trusted_path_with_hash(path: PathBuf, expected_hash: &str, label: &str) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }

    match verify_sha256(&path, expected_hash) {
        Ok(()) => Some(path),
        Err(err) => {
            log::warn!(
                "[security] 忽略未通過驗證的 {}：{} ({:?})",
                label,
                err,
                path
            );
            None
        }
    }
}

fn manifest_ffmpeg_path<'a>(manifest: &'a ToolManifest, exe_name: &str) -> Option<&'a str> {
    match exe_name {
        "ffmpeg.exe" | "ffmpeg" => manifest.ffmpeg_path.as_deref(),
        "ffprobe.exe" | "ffprobe" => manifest.ffprobe_path.as_deref(),
        _ => None,
    }
}

fn manifest_ffmpeg_hash<'a>(manifest: &'a ToolManifest, exe_name: &str) -> Option<&'a str> {
    match exe_name {
        "ffmpeg.exe" | "ffmpeg" => manifest.ffmpeg_sha256.as_deref(),
        "ffprobe.exe" | "ffprobe" => manifest.ffprobe_sha256.as_deref(),
        _ => None,
    }
}

fn trusted_ffmpeg_path_from_manifest(
    manifest: &ToolManifest,
    base_dir: &Path,
    exe_name: &str,
) -> Option<PathBuf> {
    let candidate = match manifest_ffmpeg_path(manifest, exe_name) {
        Some(path) => PathBuf::from(path),
        None => find_tool_in_dir(base_dir, exe_name)?,
    };
    let expected_hash = manifest_ffmpeg_hash(manifest, exe_name)?;

    trusted_path_with_hash(candidate, expected_hash, exe_name)
}

fn trusted_ffmpeg_path_in_dir(dir: &Path, exe_name: &str) -> Option<PathBuf> {
    let manifest = load_tool_manifest_from_dir(dir)?;
    trusted_ffmpeg_path_from_manifest(&manifest, dir, exe_name)
}

fn trusted_app_ffmpeg_path(exe_name: &str) -> Option<PathBuf> {
    let bin_dir = get_app_bin_dir()?;
    let manifest_path = app_tool_manifest_path()?;
    let manifest = load_tool_manifest_from_path(&manifest_path)?;
    trusted_ffmpeg_path_from_manifest(&manifest, &bin_dir, exe_name)
}

fn trusted_shared_ffmpeg_path(exe_name: &str) -> Option<PathBuf> {
    let manifest_path = shared_tool_manifest_path()?;
    let manifest = load_tool_manifest_from_path(&manifest_path)?;
    let base_dir = manifest_path.parent()?;
    trusted_ffmpeg_path_from_manifest(&manifest, base_dir, exe_name)
}

fn trusted_portable_ffmpeg_path(exe_name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    trusted_ffmpeg_path_in_dir(dir, exe_name)
}

fn trusted_ytdlp_path_from_manifest(manifest: &ToolManifest, base_dir: &Path) -> Option<PathBuf> {
    let candidate = match manifest.ytdlp_path.as_deref() {
        Some(path) => PathBuf::from(path),
        None => find_tool_in_dir(base_dir, YTDLP_EXE_NAME)?,
    };
    let expected_hash = manifest.ytdlp_sha256.as_deref()?;

    trusted_path_with_hash(candidate, expected_hash, "yt-dlp")
}

fn trusted_app_ytdlp_path() -> Option<PathBuf> {
    let bin_dir = get_app_bin_dir()?;
    if let Some(manifest_path) = app_tool_manifest_path() {
        if let Some(manifest) = load_tool_manifest_from_path(&manifest_path) {
            if let Some(path) = trusted_ytdlp_path_from_manifest(&manifest, &bin_dir) {
                return Some(path);
            }
        }
    }

    let candidate = find_tool_in_dir(&bin_dir, YTDLP_EXE_NAME)?;
    trusted_path_with_hash(candidate, YTDLP_SHA256, "yt-dlp")
}

fn trusted_shared_ytdlp_path() -> Option<PathBuf> {
    let manifest_path = shared_tool_manifest_path()?;
    let manifest = load_tool_manifest_from_path(&manifest_path)?;
    let base_dir = manifest_path.parent()?;
    trusted_ytdlp_path_from_manifest(&manifest, base_dir)
}

fn trusted_portable_ytdlp_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    if let Some(manifest) = load_tool_manifest_from_dir(dir) {
        if let Some(path) = trusted_ytdlp_path_from_manifest(&manifest, dir) {
            return Some(path);
        }
    }

    let candidate = find_tool_in_dir(dir, YTDLP_EXE_NAME)?;
    trusted_path_with_hash(candidate, YTDLP_SHA256, "portable yt-dlp")
}

fn local_ytdlp_candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(path) = which::which(YTDLP_EXE_NAME) {
        paths.push(path);
    }
    if let Ok(path) = which::which("yt-dlp") {
        paths.push(path);
    }
    if let Ok(path) = which::which("yt-dlp.exe") {
        paths.push(path);
    }

    paths
}

fn find_local_ytdlp_path() -> Option<PathBuf> {
    for path in local_ytdlp_candidate_paths() {
        if path.is_file() {
            return path.canonicalize().ok();
        }
    }
    None
}

fn ytdlp_candidate_from_path(path: PathBuf) -> Result<LocalYtdlpCandidate, AppError> {
    let ytdlp_sha256 = compute_sha256(&path)?;

    Ok(LocalYtdlpCandidate {
        ytdlp_path: path.to_string_lossy().to_string(),
        ytdlp_sha256,
    })
}

fn ytdlp_candidate_from_trust_request(
    candidate: LocalYtdlpCandidate,
) -> Result<LocalYtdlpCandidate, AppError> {
    let path = canonical_existing_tool_file(&candidate.ytdlp_path, "yt-dlp")?;
    let allowed_names = if cfg!(windows) {
        [YTDLP_EXE_NAME, "yt-dlp"]
    } else {
        [YTDLP_EXE_NAME, YTDLP_EXE_NAME]
    };
    if !file_name_matches(&path, &allowed_names) {
        return Err(AppError::Audio("yt-dlp 路徑必須指向 yt-dlp 執行檔".into()));
    }

    let refreshed = ytdlp_candidate_from_path(path)?;
    if refreshed.ytdlp_sha256 != candidate.ytdlp_sha256 {
        return Err(AppError::Audio(
            "yt-dlp 檔案已變更，請重新偵測後再信任".into(),
        ));
    }

    Ok(refreshed)
}

pub fn detect_local_ytdlp_candidate() -> Result<Option<LocalYtdlpCandidate>, AppError> {
    let Some(path) = find_local_ytdlp_path() else {
        return Ok(None);
    };

    ytdlp_candidate_from_path(path).map(Some)
}

pub fn trust_local_ytdlp_candidate(
    candidate: LocalYtdlpCandidate,
) -> Result<LocalYtdlpCandidate, AppError> {
    let trusted_candidate = ytdlp_candidate_from_trust_request(candidate)?;

    let mut manifest = tool_manifest_path()
        .and_then(|path| load_tool_manifest_from_path(&path))
        .unwrap_or_default();
    manifest.ytdlp_sha256 = Some(trusted_candidate.ytdlp_sha256.clone());
    manifest.ytdlp_path = Some(trusted_candidate.ytdlp_path.clone());
    save_tool_manifest(&manifest)?;
    publish_shared_ytdlp(&trusted_candidate);

    Ok(trusted_candidate)
}

fn ffmpeg_tool_names() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("ffmpeg.exe", "ffprobe.exe")
    } else {
        ("ffmpeg", "ffprobe")
    }
}

fn local_ffmpeg_candidate_dirs() -> Vec<PathBuf> {
    let (ffmpeg_name, ffprobe_name) = ffmpeg_tool_names();
    let mut dirs = Vec::new();

    if let Ok(path) = which::which(ffmpeg_name) {
        if let Some(parent) = path.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    if let Ok(path) = which::which(ffprobe_name) {
        if let Some(parent) = path.parent() {
            dirs.push(parent.to_path_buf());
        }
    }

    #[cfg(windows)]
    {
        dirs.extend(
            [
                r"C:\ffmpeg\bin",
                r"C:\Program Files\ffmpeg\bin",
                r"C:\Program Files (x86)\ffmpeg\bin",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
    }

    dirs
}

fn canonical_tool_pair(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let (ffmpeg_name, ffprobe_name) = ffmpeg_tool_names();
    let ffmpeg = dir.join(ffmpeg_name);
    let ffprobe = dir.join(ffprobe_name);
    if !ffmpeg.is_file() || !ffprobe.is_file() {
        return None;
    }

    let ffmpeg = ffmpeg.canonicalize().ok()?;
    let ffprobe = ffprobe.canonicalize().ok()?;
    Some((ffmpeg, ffprobe))
}

fn find_local_ffmpeg_pair() -> Option<(PathBuf, PathBuf)> {
    for dir in local_ffmpeg_candidate_dirs() {
        if let Some(pair) = canonical_tool_pair(&dir) {
            return Some(pair);
        }
    }
    None
}

fn candidate_from_pair(
    ffmpeg_path: PathBuf,
    ffprobe_path: PathBuf,
) -> Result<LocalFfmpegCandidate, AppError> {
    let ffmpeg_sha256 = compute_sha256(&ffmpeg_path)?;
    let ffprobe_sha256 = compute_sha256(&ffprobe_path)?;

    Ok(LocalFfmpegCandidate {
        ffmpeg_path: ffmpeg_path.to_string_lossy().to_string(),
        ffprobe_path: ffprobe_path.to_string_lossy().to_string(),
        ffmpeg_sha256,
        ffprobe_sha256,
    })
}

fn ffmpeg_candidate_from_trust_request(
    candidate: LocalFfmpegCandidate,
) -> Result<LocalFfmpegCandidate, AppError> {
    let ffmpeg_path = canonical_existing_tool_file(&candidate.ffmpeg_path, "FFmpeg")?;
    let ffprobe_path = canonical_existing_tool_file(&candidate.ffprobe_path, "ffprobe")?;
    let (ffmpeg_name, ffprobe_name) = ffmpeg_tool_names();

    if !file_name_matches(&ffmpeg_path, &[ffmpeg_name]) {
        return Err(AppError::Audio("FFmpeg 路徑必須指向 ffmpeg 執行檔".into()));
    }
    if !file_name_matches(&ffprobe_path, &[ffprobe_name]) {
        return Err(AppError::Audio(
            "ffprobe 路徑必須指向 ffprobe 執行檔".into(),
        ));
    }
    if ffmpeg_path.parent() != ffprobe_path.parent() {
        return Err(AppError::Audio(
            "ffmpeg 和 ffprobe 必須位於同一個資料夾".into(),
        ));
    }

    let refreshed = candidate_from_pair(ffmpeg_path, ffprobe_path)?;
    if refreshed.ffmpeg_sha256 != candidate.ffmpeg_sha256
        || refreshed.ffprobe_sha256 != candidate.ffprobe_sha256
    {
        return Err(AppError::Audio(
            "FFmpeg 檔案已變更，請重新偵測後再信任".into(),
        ));
    }

    Ok(refreshed)
}

pub fn detect_local_ffmpeg_candidate() -> Result<Option<LocalFfmpegCandidate>, AppError> {
    let Some((ffmpeg_path, ffprobe_path)) = find_local_ffmpeg_pair() else {
        return Ok(None);
    };

    candidate_from_pair(ffmpeg_path, ffprobe_path).map(Some)
}

pub fn trust_local_ffmpeg_candidate(
    candidate: LocalFfmpegCandidate,
) -> Result<LocalFfmpegCandidate, AppError> {
    let trusted_candidate = ffmpeg_candidate_from_trust_request(candidate)?;

    let mut manifest = tool_manifest_path()
        .and_then(|path| load_tool_manifest_from_path(&path))
        .unwrap_or_default();
    manifest.ffmpeg_sha256 = Some(trusted_candidate.ffmpeg_sha256.clone());
    manifest.ffprobe_sha256 = Some(trusted_candidate.ffprobe_sha256.clone());
    manifest.ffmpeg_path = Some(trusted_candidate.ffmpeg_path.clone());
    manifest.ffprobe_path = Some(trusted_candidate.ffprobe_path.clone());
    save_tool_manifest(&manifest)?;
    publish_shared_ffmpeg(&trusted_candidate);

    Ok(trusted_candidate)
}

/// 搜尋 yt-dlp 可執行檔。
///
/// 只信任 hash 驗證通過的 managed binary。
pub fn find_ytdlp() -> Option<PathBuf> {
    if let Some(path) = trusted_portable_ytdlp_path() {
        return Some(path);
    }

    if let Some(path) = trusted_shared_ytdlp_path() {
        return Some(path);
    }

    if let Some(path) = trusted_app_ytdlp_path() {
        return Some(path);
    }

    None
}

/// 搜尋受信任的 FFmpeg 可執行檔。
pub fn find_ffmpeg() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        trusted_portable_ffmpeg_path("ffmpeg.exe")
            .or_else(|| trusted_shared_ffmpeg_path("ffmpeg.exe"))
            .or_else(|| trusted_app_ffmpeg_path("ffmpeg.exe"))
    }

    #[cfg(not(windows))]
    {
        trusted_app_ffmpeg_path("ffmpeg").or_else(|| which::which("ffmpeg").ok())
    }
}

pub fn find_ffprobe() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        trusted_portable_ffmpeg_path("ffprobe.exe")
            .or_else(|| trusted_shared_ffmpeg_path("ffprobe.exe"))
            .or_else(|| trusted_app_ffmpeg_path("ffprobe.exe"))
    }

    #[cfg(not(windows))]
    {
        trusted_app_ffmpeg_path("ffprobe").or_else(|| which::which("ffprobe").ok())
    }
}

fn is_managed_ytdlp_version(version: &str) -> bool {
    version.trim() == YTDLP_VERSION
}

/// 取得 yt-dlp 版本字串。
fn get_ytdlp_version_for_path(ytdlp: &Path) -> Option<String> {
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

/// 取得 yt-dlp 版本字串。
pub fn get_ytdlp_version() -> Option<String> {
    let ytdlp = find_ytdlp()?;
    get_ytdlp_version_for_path(&ytdlp)
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
    pub managed_ytdlp_version: String,
    pub ytdlp_update_available: bool,
    pub ffmpeg_available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffmpeg_path: Option<String>,
}

pub fn check_tool_status() -> ToolStatus {
    let ytdlp_path = find_ytdlp();
    let ffmpeg_path = find_ffmpeg();
    let ytdlp_version = ytdlp_path.as_deref().and_then(get_ytdlp_version_for_path);
    let ytdlp_update_available = ytdlp_version
        .as_deref()
        .is_some_and(|version| !is_managed_ytdlp_version(version));

    ToolStatus {
        ytdlp_available: ytdlp_path.is_some(),
        ytdlp_version,
        ytdlp_path: ytdlp_path.map(|p| p.to_string_lossy().to_string()),
        managed_ytdlp_version: YTDLP_VERSION.to_string(),
        ytdlp_update_available,
        ffmpeg_available: ffmpeg_path.is_some(),
        ffmpeg_version: get_ffmpeg_version(),
        ffmpeg_path: ffmpeg_path.map(|p| p.to_string_lossy().to_string()),
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
fn compute_sha256(path: &Path) -> Result<String, AppError> {
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
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_sha256(path: &std::path::Path, expected_hash: &str) -> Result<(), AppError> {
    if expected_hash.is_empty() {
        // hash 尚未設定，記錄實際 hash 供開發者填入
        let hash = compute_sha256(path)?;
        log::warn!(
            "[security] SHA-256 驗證已跳過（hash 未設定）。實際 hash: {} ← 請填入程式碼",
            hash
        );
        return Ok(());
    }

    let actual = compute_sha256(path)?;
    if actual != expected_hash {
        return Err(AppError::Internal(format!(
            "SHA-256 驗證失敗！預期: {}，實際: {}。檔案可能被篡改。",
            expected_hash, actual
        )));
    }
    log::info!("[security] SHA-256 驗證通過: {}", expected_hash);
    Ok(())
}

/// 從 GitHub Releases 下載 yt-dlp 到偏好的工具資料夾。
///
/// 透過 `ytdlp:install_progress` event 推送進度。
pub fn download_ytdlp(app: &AppHandle) -> Result<PathBuf, AppError> {
    let bin_dir = get_preferred_tool_dir()
        .ok_or_else(|| AppError::Internal("無法取得工具安裝目錄".into()))?;

    // 建立目錄
    std::fs::create_dir_all(&bin_dir).map_err(AppError::Io)?;

    let target_path = bin_dir.join(YTDLP_EXE_NAME);

    if let Some(current_path) = find_ytdlp() {
        if get_ytdlp_version_for_path(&current_path)
            .as_deref()
            .is_some_and(is_managed_ytdlp_version)
        {
            let _ = app.emit(
                "ytdlp:install_progress",
                &InstallProgress {
                    percent: 100.0,
                    status: "finished".into(),
                    message: format!("yt-dlp {} 已是最新", YTDLP_VERSION),
                },
            );
            return Ok(current_path);
        }
    }

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
    let mut file = std::fs::File::create(&tmp_path).map_err(AppError::Io)?;

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
        file.write_all(&buf[..n]).map_err(AppError::Io)?;
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

    // 先驗證暫存檔，再覆蓋正式執行檔，避免把未驗證 binary 留在 target。
    verify_sha256(&tmp_path, YTDLP_SHA256)?;

    if target_path.exists() {
        std::fs::remove_file(&target_path).map_err(AppError::Io)?;
    }
    std::fs::rename(&tmp_path, &target_path).map_err(AppError::Io)?;
    _tmp_guard.disarm();

    // Unix: 設定可執行權限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| AppError::Io(e))?;
    }

    let mut manifest = load_tool_manifest_from_dir(&bin_dir).unwrap_or_default();
    manifest.ytdlp_sha256 = Some(YTDLP_SHA256.into());
    manifest.ytdlp_path = None;
    save_tool_manifest_to_dir(&bin_dir, &manifest)?;
    publish_shared_ytdlp(&LocalYtdlpCandidate {
        ytdlp_path: target_path.to_string_lossy().to_string(),
        ytdlp_sha256: YTDLP_SHA256.into(),
    });

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

/// 從 GitHub 下載 FFmpeg 靜態建置並解壓到偏好的工具資料夾。
///
/// 只保留 `ffmpeg.exe` 和 `ffprobe.exe`（Windows），其餘捨棄。
/// 透過 `ffmpeg:install_progress` event 推送進度。
#[cfg(windows)]
pub fn download_ffmpeg(app: &AppHandle) -> Result<PathBuf, AppError> {
    let bin_dir = get_preferred_tool_dir()
        .ok_or_else(|| AppError::Internal("無法取得工具安裝目錄".into()))?;

    std::fs::create_dir_all(&bin_dir).map_err(AppError::Io)?;

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
    let mut file = std::fs::File::create(&tmp_zip).map_err(AppError::Io)?;

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
        file.write_all(&buf[..n]).map_err(AppError::Io)?;
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

    let zip_file = std::fs::File::open(&tmp_zip).map_err(AppError::Io)?;
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|e| AppError::Internal(format!("zip 開啟失敗: {}", e)))?;

    let targets = ["ffmpeg.exe", "ffprobe.exe"];
    let ffmpeg_tmp = bin_dir.join("ffmpeg.exe.tmp");
    let ffprobe_tmp = bin_dir.join("ffprobe.exe.tmp");
    let mut ffmpeg_guard = TempFileGuard::new(ffmpeg_tmp.clone());
    let mut ffprobe_guard = TempFileGuard::new(ffprobe_tmp.clone());
    let mut extracted_count = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Internal(format!("zip 讀取失敗: {}", e)))?;

        let entry_name = entry.name().to_string();

        // zip 內的結構：ffmpeg-<version>-essentials_build/bin/ffmpeg.exe
        for target in &targets {
            if entry_name.ends_with(&format!("bin/{}", target)) {
                let out_path = if *target == "ffmpeg.exe" {
                    &ffmpeg_tmp
                } else {
                    &ffprobe_tmp
                };
                let mut out_file = std::fs::File::create(out_path).map_err(AppError::Io)?;
                std::io::copy(&mut entry, &mut out_file).map_err(AppError::Io)?;
                log::info!("[ffmpeg] 解壓: {} -> {:?}", entry_name, out_path);
                extracted_count += 1;
                break;
            }
        }

        if extracted_count >= targets.len() {
            break;
        }
    }

    if extracted_count < targets.len() {
        return Err(AppError::Internal(
            "zip 中缺少 ffmpeg.exe 或 ffprobe.exe".into(),
        ));
    }

    let ffmpeg_path = bin_dir.join("ffmpeg.exe");
    let ffprobe_path = bin_dir.join("ffprobe.exe");
    let ffmpeg_sha256 = compute_sha256(&ffmpeg_tmp)?;
    let ffprobe_sha256 = compute_sha256(&ffprobe_tmp)?;

    if ffmpeg_path.exists() {
        std::fs::remove_file(&ffmpeg_path).map_err(AppError::Io)?;
    }
    if ffprobe_path.exists() {
        std::fs::remove_file(&ffprobe_path).map_err(AppError::Io)?;
    }

    std::fs::rename(&ffmpeg_tmp, &ffmpeg_path).map_err(AppError::Io)?;
    ffmpeg_guard.disarm();
    std::fs::rename(&ffprobe_tmp, &ffprobe_path).map_err(AppError::Io)?;
    ffprobe_guard.disarm();

    let mut manifest = load_tool_manifest_from_dir(&bin_dir).unwrap_or_default();
    manifest.ffmpeg_sha256 = Some(ffmpeg_sha256);
    manifest.ffprobe_sha256 = Some(ffprobe_sha256);
    manifest.ffmpeg_path = None;
    manifest.ffprobe_path = None;
    save_tool_manifest_to_dir(&bin_dir, &manifest)?;
    publish_shared_ffmpeg(&LocalFfmpegCandidate {
        ffmpeg_path: ffmpeg_path.to_string_lossy().to_string(),
        ffprobe_path: ffprobe_path.to_string_lossy().to_string(),
        ffmpeg_sha256: manifest.ffmpeg_sha256.clone().unwrap_or_default(),
        ffprobe_sha256: manifest.ffprobe_sha256.clone().unwrap_or_default(),
    });

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
    if url.contains('\0') {
        return Err(AppError::Audio("URL 包含無效字元".into()));
    }
    // 🔴 Codex 安全審查 P1 #2：擋 yt-dlp option injection。
    // 正常的 URL 不會含空白字元；若允許，攻擊者可傳 `http://x --exec "calc.exe"`
    // 讓 yt-dlp 把第二段當成獨立 argument 執行 hook 指令。
    if url.chars().any(char::is_whitespace) {
        return Err(AppError::Audio("URL 不可包含空白字元".into()));
    }

    let parsed = url::Url::parse(url).map_err(|_| AppError::Audio("URL 格式錯誤".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::Audio(
            "URL 必須以 http:// 或 https:// 開頭".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Audio("URL 缺少主機名稱".into()))?;
    if !is_allowed_youtube_host(host) {
        return Err(AppError::Audio(
            "目前只支援 YouTube、youtu.be、youtube-nocookie.com URL".into(),
        ));
    }

    Ok(())
}

fn is_allowed_youtube_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "youtu.be"
        || host == "youtube.com"
        || host.ends_with(".youtube.com")
        || host == "youtube-nocookie.com"
        || host.ends_with(".youtube-nocookie.com")
}

fn escape_ytdlp_output_template_literal(value: &str) -> String {
    value.replace('%', "%%")
}

fn build_output_template(output_dir: &str, url_type: UrlType) -> String {
    let output_dir = escape_ytdlp_output_template_literal(output_dir);
    let separator = if output_dir.ends_with('/') || output_dir.ends_with('\\') {
        ""
    } else {
        "/"
    };
    let filename_template = if url_type == UrlType::Channel {
        "%(uploader)s/%(title)s.%(ext)s"
    } else {
        "%(title)s.%(ext)s"
    };

    format!("{output_dir}{separator}{filename_template}")
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
    args.push("-o".into());
    args.push(build_output_template(&req.output_dir, url_type));

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
    security::validate_path_safe(&req.output_dir)?;
    std::fs::create_dir_all(&req.output_dir).map_err(AppError::Io)?;
    let canonical_output_dir = std::fs::canonicalize(&req.output_dir).map_err(AppError::Io)?;
    let req = DownloadRequest {
        output_dir: canonical_output_dir.to_string_lossy().to_string(),
        ..req
    };

    let ytdlp = find_ytdlp().ok_or_else(|| {
        AppError::Audio("找不到受信任的 yt-dlp。請點擊「自動安裝」重新安裝".into())
    })?;

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
                    .map_while(Result::ok)
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
        assert!(validate_url("https://www.youtube-nocookie.com/embed/abc").is_ok());
    }

    #[test]
    fn validates_url_rejects_non_youtube_hosts() {
        assert!(validate_url("https://example.com/watch?v=abc123").is_err());
        assert!(validate_url("https://youtube.com.evil.test/watch?v=abc123").is_err());
        assert!(validate_url("https://youtube.com@evil.test/watch?v=abc123").is_err());
    }

    #[test]
    fn validates_url_rejects_too_long() {
        let long_url = format!("https://example.com/{}", "a".repeat(2100));
        assert!(validate_url(&long_url).is_err());
    }

    #[test]
    fn output_template_escapes_literal_percent_in_output_dir() {
        let template = build_output_template("C:\\Users\\me\\100% Mix", UrlType::Video);
        assert!(template.contains("100%% Mix"));
        assert!(template.ends_with("%(title)s.%(ext)s"));
    }

    #[test]
    fn channel_output_template_preserves_uploader_folder() {
        let template = build_output_template("C:\\Users\\me\\Downloads", UrlType::Channel);
        assert!(template.ends_with("%(uploader)s/%(title)s.%(ext)s"));
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

    #[test]
    fn trusted_ffmpeg_path_requires_matching_manifest_hash() {
        let dir =
            std::env::temp_dir().join(format!("vocalsync-trusted-ffmpeg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ffmpeg = dir.join("ffmpeg.exe");
        let ffprobe = dir.join("ffprobe.exe");
        std::fs::write(&ffmpeg, b"ffmpeg-test").unwrap();
        std::fs::write(&ffprobe, b"ffprobe-test").unwrap();

        let manifest = ToolManifest {
            ytdlp_sha256: None,
            ytdlp_path: None,
            ffmpeg_sha256: Some(compute_sha256(&ffmpeg).unwrap()),
            ffprobe_sha256: Some(compute_sha256(&ffprobe).unwrap()),
            ffmpeg_path: None,
            ffprobe_path: None,
        };
        std::fs::write(
            tool_manifest_path_in_dir(&dir),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(
            trusted_ffmpeg_path_in_dir(&dir, "ffmpeg.exe"),
            Some(ffmpeg.clone())
        );
        assert_eq!(
            trusted_ffmpeg_path_in_dir(&dir, "ffprobe.exe"),
            Some(ffprobe.clone())
        );

        let manifest = ToolManifest {
            ytdlp_sha256: None,
            ytdlp_path: None,
            ffmpeg_sha256: Some("wrong".into()),
            ffprobe_sha256: Some(compute_sha256(&ffprobe).unwrap()),
            ffmpeg_path: None,
            ffprobe_path: None,
        };
        std::fs::write(
            tool_manifest_path_in_dir(&dir),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        assert!(trusted_ffmpeg_path_in_dir(&dir, "ffmpeg.exe").is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn trusted_ytdlp_path_requires_matching_manifest_hash() {
        let dir =
            std::env::temp_dir().join(format!("vocalsync-trusted-ytdlp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ytdlp = dir.join(YTDLP_EXE_NAME);
        std::fs::write(&ytdlp, b"yt-dlp-test").unwrap();

        let manifest = ToolManifest {
            ytdlp_sha256: Some(compute_sha256(&ytdlp).unwrap()),
            ytdlp_path: None,
            ffmpeg_sha256: None,
            ffprobe_sha256: None,
            ffmpeg_path: None,
            ffprobe_path: None,
        };

        assert_eq!(
            trusted_ytdlp_path_from_manifest(&manifest, &dir),
            Some(ytdlp.clone())
        );

        let manifest = ToolManifest {
            ytdlp_sha256: Some("wrong".into()),
            ytdlp_path: None,
            ffmpeg_sha256: None,
            ffprobe_sha256: None,
            ffmpeg_path: None,
            ffprobe_path: None,
        };

        assert!(trusted_ytdlp_path_from_manifest(&manifest, &dir).is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn managed_ytdlp_version_accepts_only_current_version() {
        assert!(is_managed_ytdlp_version(YTDLP_VERSION));
        assert!(is_managed_ytdlp_version(&format!("{}\n", YTDLP_VERSION)));
        assert!(!is_managed_ytdlp_version("2025.03.31"));
    }

    #[test]
    fn ytdlp_trust_request_rejects_changed_hash() {
        let dir = std::env::temp_dir().join(format!(
            "vocalsync-ytdlp-trust-request-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ytdlp = dir.join(YTDLP_EXE_NAME);
        std::fs::write(&ytdlp, b"yt-dlp-before").unwrap();
        let candidate = ytdlp_candidate_from_path(ytdlp.clone()).unwrap();
        std::fs::write(&ytdlp, b"yt-dlp-after").unwrap();

        assert!(ytdlp_candidate_from_trust_request(candidate).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ffmpeg_trust_request_rejects_changed_hash() {
        let dir = std::env::temp_dir().join(format!(
            "vocalsync-ffmpeg-trust-request-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let (ffmpeg_name, ffprobe_name) = ffmpeg_tool_names();
        let ffmpeg = dir.join(ffmpeg_name);
        let ffprobe = dir.join(ffprobe_name);
        std::fs::write(&ffmpeg, b"ffmpeg-before").unwrap();
        std::fs::write(&ffprobe, b"ffprobe-before").unwrap();
        let candidate = candidate_from_pair(ffmpeg.clone(), ffprobe.clone()).unwrap();
        std::fs::write(&ffmpeg, b"ffmpeg-after").unwrap();

        assert!(ffmpeg_candidate_from_trust_request(candidate).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }
}
