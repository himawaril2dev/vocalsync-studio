//! 影片內嵌字幕提取器
//!
//! 使用 FFmpeg CLI（ffprobe + ffmpeg）偵測並提取內嵌字幕軌。
//!
//! # 工作流程
//!
//! 1. `probe_subtitles`：用 ffprobe 偵測影片中的字幕軌資訊
//! 2. `extract_subtitle`：用 ffmpeg 提取指定字幕軌到 SRT 檔案
//!
//! # 外部依賴
//!
//! - **FFmpeg** / **ffprobe**：需要系統上已安裝

use crate::core::ytdlp_engine;
use crate::error::AppError;
use crate::security;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 字幕軌資訊
#[derive(Debug, Clone, Serialize)]
pub struct SubtitleStream {
    /// FFmpeg stream index（用於提取時的 `-map 0:s:{index}`）
    pub index: usize,
    /// 語言標籤（如 "eng", "jpn", "chi"，可能為空）
    pub language: String,
    /// 軌道標題（如 "English", "日本語"，可能為空）
    pub title: String,
    /// 編碼格式（如 "subrip", "ass", "mov_text"）
    pub codec: String,
}

/// 偵測影片中的內嵌字幕軌。
///
/// 使用 ffprobe JSON 輸出格式，解析所有 subtitle 類型的 stream。
/// 如果找不到 ffprobe 或影片沒有字幕軌，回傳空 Vec。
pub fn probe_subtitles(video_path: &str) -> Result<Vec<SubtitleStream>, AppError> {
    // 🔴 Codex 安全審查 P1 #2：擋 subprocess argument injection
    // （絕對路徑 + 不以 `-` 開頭，避免被 ffprobe 當 option）
    security::validate_path_safe(video_path)?;
    // 🟡 Y1 修正：先驗證輸入路徑是否存在
    if !Path::new(video_path).exists() {
        return Err(AppError::Audio(format!("影片檔案不存在：{}", video_path)));
    }

    let ffprobe = find_ffprobe()
        .ok_or_else(|| AppError::Audio("找不到 ffprobe。請確認 FFmpeg 已安裝".into()))?;

    let mut cmd = Command::new(&ffprobe);
    cmd.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_streams",
        "-select_streams",
        "s", // 只顯示字幕軌
        video_path,
    ]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd
        .output()
        .map_err(|e| AppError::Audio(format!("ffprobe 執行失敗: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Audio(format!(
            "ffprobe 錯誤: {}",
            stderr.lines().last().unwrap_or("unknown error")
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ffprobe_output(&stdout)
}

/// 提取指定字幕軌到 SRT 檔案。
///
/// `stream_index` 是字幕軌在同類型中的順序索引（0-based），
/// 對應 ffprobe 回傳的 `SubtitleStream.index`。
///
/// 回傳提取出的 SRT 檔案路徑。
pub fn extract_subtitle(
    video_path: &str,
    stream_index: usize,
    output_dir: Option<&str>,
) -> Result<PathBuf, AppError> {
    // 🔴 Codex 安全審查 P1 #2：ffmpeg argument injection 防線
    security::validate_path_safe(video_path)?;
    if let Some(dir) = output_dir {
        security::validate_path_safe(dir)?;
    }
    // 🟡 Y1 修正：先驗證輸入路徑是否存在
    if !Path::new(video_path).exists() {
        return Err(AppError::Audio(format!("影片檔案不存在：{}", video_path)));
    }

    let ffmpeg = ytdlp_engine::find_ffmpeg()
        .ok_or_else(|| AppError::Audio("找不到 FFmpeg。請確認 FFmpeg 已安裝".into()))?;

    let video = Path::new(video_path);
    let stem = video
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitle");

    // 輸出到影片同目錄或指定目錄
    let out_dir = output_dir.map(PathBuf::from).unwrap_or_else(|| {
        video
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    });

    let out_path = out_dir.join(format!("{}.sub{}.srt", stem, stream_index));

    let mut cmd = Command::new(&ffmpeg);
    cmd.args([
        "-i",
        video_path,
        "-map",
        &format!("0:s:{}", stream_index),
        "-c:s",
        "srt", // 轉換為 SRT 格式
        "-y",  // 覆寫已存在的檔案
        &out_path.to_string_lossy(),
    ]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd
        .output()
        .map_err(|e| AppError::Audio(format!("FFmpeg 執行失敗: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Audio(format!(
            "字幕提取失敗: {}",
            stderr.lines().last().unwrap_or("unknown error")
        )));
    }

    if !out_path.exists() {
        return Err(AppError::Audio("字幕提取後找不到輸出檔案".into()));
    }

    Ok(out_path)
}

/// 搜尋 ffprobe 執行檔（跟 ffmpeg 放在一起）
fn find_ffprobe() -> Option<PathBuf> {
    // 1. app bin 資料夾
    if let Some(bin_dir) = ytdlp_engine::get_app_bin_dir() {
        let candidate = bin_dir.join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 2. 系統 PATH
    if let Ok(path) = which::which("ffprobe") {
        return Some(path);
    }

    // 3. 跟 ffmpeg 同目錄
    if let Some(ffmpeg) = ytdlp_engine::find_ffmpeg() {
        if let Some(dir) = ffmpeg.parent() {
            let candidate = dir.join(if cfg!(windows) {
                "ffprobe.exe"
            } else {
                "ffprobe"
            });
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

/// 解析 ffprobe JSON 輸出，提取字幕軌資訊。
fn parse_ffprobe_output(json_str: &str) -> Result<Vec<SubtitleStream>, AppError> {
    let value: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| AppError::Audio(format!("ffprobe 輸出解析失敗: {}", e)))?;

    // 🟡 Y2 修正：避免不必要的 clone，使用引用即可
    let empty_vec = Vec::new();
    let streams = value
        .get("streams")
        .and_then(|s| s.as_array())
        .unwrap_or(&empty_vec);

    let mut result = Vec::new();
    let mut sub_index = 0;

    for stream in streams {
        let codec_type = stream
            .get("codec_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if codec_type != "subtitle" {
            continue;
        }

        let codec = stream
            .get("codec_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tags = stream.get("tags");
        let language = tags
            .and_then(|t| t.get("language"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = tags
            .and_then(|t| t.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        result.push(SubtitleStream {
            index: sub_index,
            language,
            title,
            codec,
        });

        sub_index += 1;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_json_with_subtitles() {
        let json = r#"{
            "streams": [
                {
                    "index": 2,
                    "codec_name": "subrip",
                    "codec_type": "subtitle",
                    "tags": {
                        "language": "eng",
                        "title": "English"
                    }
                },
                {
                    "index": 3,
                    "codec_name": "ass",
                    "codec_type": "subtitle",
                    "tags": {
                        "language": "jpn",
                        "title": "日本語"
                    }
                }
            ]
        }"#;

        let subs = parse_ffprobe_output(json).unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].index, 0);
        assert_eq!(subs[0].language, "eng");
        assert_eq!(subs[0].title, "English");
        assert_eq!(subs[0].codec, "subrip");
        assert_eq!(subs[1].index, 1);
        assert_eq!(subs[1].language, "jpn");
        assert_eq!(subs[1].codec, "ass");
    }

    #[test]
    fn parses_ffprobe_json_no_subtitles() {
        let json = r#"{"streams": []}"#;
        let subs = parse_ffprobe_output(json).unwrap();
        assert!(subs.is_empty());
    }

    #[test]
    fn parses_ffprobe_json_mixed_streams() {
        let json = r#"{
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": "h264"},
                {"index": 1, "codec_type": "audio", "codec_name": "aac"},
                {"index": 2, "codec_type": "subtitle", "codec_name": "mov_text", "tags": {"language": "chi"}}
            ]
        }"#;

        let subs = parse_ffprobe_output(json).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].language, "chi");
        assert_eq!(subs[0].codec, "mov_text");
    }
}
