//! 歌詞解析 Commands

use crate::core::lyrics_parser::{self, LyricLine};
use crate::core::subtitle_extractor::{self, SubtitleStream};
use crate::error::AppError;

#[tauri::command]
pub fn load_lyrics(path: String) -> Result<Vec<LyricLine>, AppError> {
    lyrics_parser::load_lyrics(&path)
}

/// 掃描目錄中的字幕檔案（.srt, .vtt, .lrc）
#[tauri::command]
pub fn find_subtitle_files(dir: String) -> Vec<String> {
    lyrics_parser::find_subtitle_files(&dir)
}

/// 偵測影片中的內嵌字幕軌
#[tauri::command]
pub fn probe_embedded_subtitles(
    video_path: String,
) -> Result<Vec<SubtitleStream>, AppError> {
    subtitle_extractor::probe_subtitles(&video_path)
}

/// 儲存歌詞為 LRC 格式
#[tauri::command]
pub fn save_lyrics_as_lrc(
    lines: Vec<LyricLine>,
    output_path: String,
) -> Result<(), AppError> {
    let lrc_content = lyrics_parser::export_lrc(&lines);
    std::fs::write(&output_path, lrc_content)
        .map_err(|e| AppError::Audio(format!("無法寫入 LRC 檔案：{}", e)))?;
    Ok(())
}

/// 提取指定字幕軌到 SRT 檔案，回傳檔案路徑
#[tauri::command]
pub fn extract_embedded_subtitle(
    video_path: String,
    stream_index: usize,
    output_dir: Option<String>,
) -> Result<String, AppError> {
    let out = subtitle_extractor::extract_subtitle(
        &video_path,
        stream_index,
        output_dir.as_deref(),
    )?;
    Ok(out.to_string_lossy().to_string())
}
