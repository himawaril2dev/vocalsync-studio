//! 歌詞解析 Commands

use crate::core::lyrics_parser::{self, LyricLine};
use crate::core::subtitle_extractor::{self, SubtitleStream};
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

fn lrc_default_file_name(default_file_name: Option<String>) -> String {
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
    path.set_extension("lrc");
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && security::validate_filename_prefix(name).is_ok())
        .unwrap_or("lyrics_synced.lrc")
        .to_string()
}

fn normalize_lrc_output_path(mut path: PathBuf) -> Result<PathBuf, AppError> {
    if path.extension().is_none() {
        path.set_extension("lrc");
    }

    let path_str = path.to_string_lossy();
    security::validate_path_safe(&path_str)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Audio("LRC 檔名無效".into()))?;
    security::validate_filename_prefix(file_name)?;

    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if !extension.eq_ignore_ascii_case("lrc") {
        return Err(AppError::Audio("LRC 匯出路徑必須使用 .lrc 副檔名".into()));
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Audio("LRC 匯出資料夾無效".into()))?;
    let parent = parent.canonicalize()?;
    if !parent.is_dir() {
        return Err(AppError::Audio("LRC 匯出資料夾不存在".into()));
    }

    Ok(parent.join(file_name))
}

/// 儲存歌詞為 LRC 格式
#[tauri::command]
pub fn save_lyrics_as_lrc(
    app: tauri::AppHandle,
    lines: Vec<LyricLine>,
    default_file_name: Option<String>,
) -> Result<Option<String>, AppError> {
    let Some(selected_path) = app
        .dialog()
        .file()
        .set_title("Export LRC")
        .add_filter("LRC", &["lrc"])
        .set_file_name(lrc_default_file_name(default_file_name))
        .blocking_save_file()
    else {
        return Ok(None);
    };

    let output_path = selected_path
        .into_path()
        .map_err(|_| AppError::Audio("LRC 匯出路徑無效".into()))?;
    let output_path = normalize_lrc_output_path(output_path)?;
    let lrc_content = lyrics_parser::export_lrc(&lines);
    std::fs::write(&output_path, lrc_content)
        .map_err(|e| AppError::Audio(format!("無法寫入 LRC 檔案：{}", e)))?;
    Ok(Some(output_path.to_string_lossy().to_string()))
}

/// 提取指定字幕軌到 SRT 檔案，回傳檔案路徑
#[tauri::command]
pub fn extract_embedded_subtitle(
    video_path: String,
    stream_index: usize,
    output_dir: Option<String>,
) -> Result<String, AppError> {
    security::validate_path_safe(&video_path)?;
    if let Some(ref dir) = output_dir {
        security::validate_path_safe(dir)?;
    }
    let out =
        subtitle_extractor::extract_subtitle(&video_path, stream_index, output_dir.as_deref())?;
    Ok(out.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_default_file_name_enforces_lrc_extension() {
        assert_eq!(lrc_default_file_name(Some("song.txt".into())), "song.lrc");
        assert_eq!(
            lrc_default_file_name(Some("../bad:name".into())),
            "badname.lrc"
        );
    }

    #[test]
    fn normalize_lrc_output_path_adds_missing_extension() {
        let output = normalize_lrc_output_path(std::env::temp_dir().join("vocalsync-test-export"))
            .expect("temp path should be valid");

        assert_eq!(output.extension().and_then(|ext| ext.to_str()), Some("lrc"));
    }

    #[test]
    fn normalize_lrc_output_path_rejects_non_lrc_extension() {
        let output = normalize_lrc_output_path(std::env::temp_dir().join("vocalsync-test.txt"));

        assert!(output.is_err());
    }
}
