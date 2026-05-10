//! 歌詞解析 Commands

use crate::core::lyrics_aligner::{self, LyricsAlignmentResult};
use crate::core::lyrics_forced_aligner::{self, LyricsTranscriptAlignmentResult};
use crate::core::lyrics_parser::{self, LyricLine};
use crate::core::portable_paths;
use crate::core::subtitle_extractor::{self, SubtitleStream};
use crate::core::whisper_engine;
use crate::error::AppError;
use crate::security;
use std::path::PathBuf;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn load_lyrics(path: String) -> Result<Vec<LyricLine>, AppError> {
    security::validate_path_safe(&path)?;
    lyrics_parser::load_lyrics(&path)
}

/// 掃描目錄中的字幕檔案（.srt, .vtt, .lrc）
#[tauri::command]
pub fn find_subtitle_files(dir: String) -> Result<Vec<String>, AppError> {
    security::validate_path_safe(&dir)?;
    Ok(lyrics_parser::find_subtitle_files(&dir))
}

/// 偵測影片中的內嵌字幕軌
#[tauri::command]
pub fn probe_embedded_subtitles(video_path: String) -> Result<Vec<SubtitleStream>, AppError> {
    security::validate_path_safe(&video_path)?;
    subtitle_extractor::probe_subtitles(&video_path)
}

fn subtitle_default_file_name(default_file_name: Option<String>, extension: &str) -> String {
    let raw = default_file_name.unwrap_or_else(|| "lyrics_synced.lrc".into());
    let file_name = raw
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("lyrics_synced.lrc")
        .trim();
    let cleaned = file_name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'))
        .take(180)
        .collect::<String>();
    let cleaned = cleaned.trim();
    let mut path = if cleaned.is_empty() {
        PathBuf::from("lyrics_synced")
    } else {
        PathBuf::from(cleaned)
    };
    path.set_extension(extension);
    let fallback = format!("lyrics_synced.{}", extension);
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && security::validate_filename_prefix(name).is_ok())
        .map(str::to_string)
        .unwrap_or(fallback)
}

fn normalize_subtitle_output_path(
    mut path: PathBuf,
    expected_extension: &str,
) -> Result<PathBuf, AppError> {
    if path.extension().is_none() {
        path.set_extension(expected_extension);
    }

    let path_str = path.to_string_lossy();
    security::validate_path_safe(&path_str)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Audio("LRC 檔名無效".into()))?;
    security::validate_filename_prefix(file_name)?;

    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if !extension.eq_ignore_ascii_case(expected_extension) {
        return Err(AppError::Audio(format!(
            "字幕匯出路徑必須使用 .{} 副檔名",
            expected_extension
        )));
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Audio("字幕匯出資料夾無效".into()))?;
    let parent = parent.canonicalize()?;
    if !parent.is_dir() {
        return Err(AppError::Audio("字幕匯出資料夾不存在".into()));
    }

    Ok(parent.join(file_name))
}

fn normalize_export_format(format: &str) -> Result<&'static str, AppError> {
    match format.trim().to_ascii_lowercase().as_str() {
        "lrc" => Ok("lrc"),
        "srt" => Ok("srt"),
        "ass" => Ok("ass"),
        _ => Err(AppError::Audio("不支援的字幕格式".into())),
    }
}

fn export_filter_name(format: &str) -> &'static str {
    match format {
        "lrc" => "LRC",
        "srt" => "SRT",
        "ass" => "ASS",
        _ => "Subtitle",
    }
}

fn export_lyrics_content(format: &str, lines: &[LyricLine]) -> String {
    match format {
        "srt" => lyrics_parser::export_srt(lines),
        "ass" => lyrics_parser::export_ass(lines),
        _ => lyrics_parser::export_lrc(lines),
    }
}

/// 儲存歌詞為 LRC 格式
#[tauri::command]
pub fn save_lyrics_as_lrc(
    app: tauri::AppHandle,
    lines: Vec<LyricLine>,
    default_file_name: Option<String>,
) -> Result<Option<String>, AppError> {
    save_lyrics_as_subtitle(app, lines, "lrc".into(), default_file_name)
}

/// 儲存歌詞為 LRC / SRT / ASS 字幕格式
#[tauri::command]
pub fn save_lyrics_as_subtitle(
    app: tauri::AppHandle,
    lines: Vec<LyricLine>,
    format: String,
    default_file_name: Option<String>,
) -> Result<Option<String>, AppError> {
    let format = normalize_export_format(&format)?;
    let Some(selected_path) = app
        .dialog()
        .file()
        .set_title(format!("Export {}", export_filter_name(format)))
        .add_filter(export_filter_name(format), &[format])
        .set_file_name(subtitle_default_file_name(default_file_name, format))
        .blocking_save_file()
    else {
        return Ok(None);
    };

    let output_path = selected_path
        .into_path()
        .map_err(|_| AppError::Audio("字幕匯出路徑無效".into()))?;
    let output_path = normalize_subtitle_output_path(output_path, format)?;
    let content = export_lyrics_content(format, &lines);
    std::fs::write(&output_path, content)
        .map_err(|e| AppError::Audio(format!("無法寫入字幕檔案：{}", e)))?;
    Ok(Some(output_path.to_string_lossy().to_string()))
}

/// 用人聲能量分段，把純文字或無時間戳歌詞自動對齊到音訊。
#[tauri::command]
pub fn auto_align_lyrics_to_audio(
    audio_path: String,
    lines: Vec<LyricLine>,
) -> Result<LyricsAlignmentResult, AppError> {
    security::validate_path_safe(&audio_path)?;
    lyrics_aligner::align_lyrics_to_audio(&audio_path, &lines)
}

/// 提取指定字幕軌到 SRT 檔案，回傳檔案路徑
#[tauri::command]
pub fn extract_embedded_subtitle(
    video_path: String,
    stream_index: usize,
    output_dir: Option<String>,
) -> Result<String, AppError> {
    security::validate_path_safe(&video_path)?;
    let output_dir = match output_dir {
        Some(dir) => {
            security::validate_path_safe(&dir)?;
            Some(dir)
        }
        None => Some(
            portable_paths::ensure_dir("subtitles")?
                .to_string_lossy()
                .to_string(),
        ),
    };
    let out =
        subtitle_extractor::extract_subtitle(&video_path, stream_index, output_dir.as_deref())?;
    Ok(out.to_string_lossy().to_string())
}

#[tauri::command]
pub fn align_lyrics_to_timed_transcript(
    transcript_path: String,
    lines: Vec<LyricLine>,
    audio_path: Option<String>,
) -> Result<LyricsTranscriptAlignmentResult, AppError> {
    security::validate_path_safe(&transcript_path)?;
    let first_vocal_onset_ms = match audio_path {
        Some(path) => {
            security::validate_path_safe(&path)?;
            lyrics_aligner::detect_first_vocal_onset_ms(&path)?
        }
        None => None,
    };
    let result = lyrics_forced_aligner::align_lyrics_to_timed_transcript_with_intro(
        &transcript_path,
        &lines,
        first_vocal_onset_ms,
    );
    if let Err(err) =
        whisper_engine::cleanup_generated_transcript_artifacts(&PathBuf::from(&transcript_path))
    {
        log::warn!("[lyrics] failed to clean Whisper transcript cache: {}", err);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_default_file_name_enforces_lrc_extension() {
        assert_eq!(
            subtitle_default_file_name(Some("song.txt".into()), "lrc"),
            "song.lrc"
        );
        assert_eq!(
            subtitle_default_file_name(Some("../bad:name".into()), "lrc"),
            "badname.lrc"
        );
    }

    #[test]
    fn subtitle_default_file_name_enforces_requested_extension() {
        assert_eq!(
            subtitle_default_file_name(Some("song.lrc".into()), "srt"),
            "song.srt"
        );
        assert_eq!(
            subtitle_default_file_name(Some("song.txt".into()), "ass"),
            "song.ass"
        );
    }

    #[test]
    fn normalize_lrc_output_path_adds_missing_extension() {
        let output = normalize_subtitle_output_path(
            std::env::temp_dir().join("vocalsync-test-export"),
            "lrc",
        )
        .expect("temp path should be valid");

        assert_eq!(output.extension().and_then(|ext| ext.to_str()), Some("lrc"));
    }

    #[test]
    fn normalize_lrc_output_path_rejects_non_lrc_extension() {
        let output =
            normalize_subtitle_output_path(std::env::temp_dir().join("vocalsync-test.txt"), "lrc");

        assert!(output.is_err());
    }
}
