//! 目標旋律 Commands
//!
//! 提供前端：
//! - `auto_detect_melody_source`：給一個伴奏路徑，回傳自動偵測結果。
//! - `load_melody_from_path`：讓使用者手動選檔載入 MIDI `.mid`。
//! - `auto_load_melody_for_backing`：保留給 `load_backing` 成功後的自動掛載流程用，
//!   目前固定回傳 `None`。

use crate::core::audio_aligner::{self, AlignmentResult};
use crate::core::melody_extractor;
use crate::core::melody_source_detector::{detect_melody_source, DetectedSource};
use crate::core::melody_track::MelodyTrack;
use crate::core::midi_parser;
use crate::error::AppError;
use crate::security;
use serde::Serialize;
use std::path::PathBuf;

/// 取得 CREPE 模型目錄。
/// dev 模式下在 src-tauri/models/，production 模式下在 resource dir。
fn get_model_dir() -> Option<PathBuf> {
    // Dev 模式：從 exe 所在目錄往上找 src-tauri/models/
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();

    // 嘗試 dev 路徑（exe 在 target/debug/ 下）
    let dev_path = exe_dir
        .parent() // target/
        .and_then(|p| p.parent()) // src-tauri/
        .map(|p| p.join("models"));

    if let Some(ref p) = dev_path {
        if p.join("crepe-tiny.onnx").exists() {
            return dev_path;
        }
    }

    // Production：models/ 在 exe 同目錄
    let prod_path = exe_dir.join("models");
    if prod_path.join("crepe-tiny.onnx").exists() {
        return Some(prod_path);
    }

    // 最後嘗試 exe 同目錄
    if exe_dir.join("crepe-tiny.onnx").exists() {
        return Some(exe_dir);
    }

    None
}

/// 偵測結果（給前端判斷要不要顯示「沒有目標旋律」提示）
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DetectedSourceDto {
    None,
}

impl From<DetectedSource> for DetectedSourceDto {
    fn from(value: DetectedSource) -> Self {
        match value {
            DetectedSource::None => DetectedSourceDto::None,
        }
    }
}

/// 自動偵測同資料夾的目標旋律來源檔。
#[tauri::command]
pub fn auto_detect_melody_source(backing_path: String) -> Result<DetectedSourceDto, AppError> {
    security::validate_path_safe(&backing_path)?;
    let path = PathBuf::from(&backing_path);
    let detected = detect_melody_source(&path);
    Ok(detected.into())
}

/// 從指定路徑載入目標旋律（使用者手動選檔）。
///
/// 支援格式：
/// - `.mid` / `.midi` — MIDI 檔案
/// - `.wav` / `.mp3` / `.flac` 等音訊 — 視為乾淨人聲軌，跑 CREPE/PYIN 提取旋律
#[tauri::command]
pub fn load_melody_from_path(path: String) -> Result<MelodyTrack, AppError> {
    security::validate_path_safe(&path)?;
    let lowered = path.to_lowercase();
    if is_audio_extension(&lowered) {
        melody_extractor::extract_melody_from_vocals(&path, get_model_dir().as_ref())
    } else if lowered.ends_with(".mid") || lowered.ends_with(".midi") {
        midi_parser::load_midi(&path)
    } else {
        Err(AppError::Audio(format!("不支援的旋律檔格式：{path}")))
    }
}

/// 從乾淨的人聲音檔（使用者用 UVR5 / Moises 等工具預先分離好的 vocals.wav）
/// 提取 MelodyTrack。
#[tauri::command]
pub fn load_vocals_and_extract_melody(vocals_path: String) -> Result<MelodyTrack, AppError> {
    security::validate_path_safe(&vocals_path)?;
    melody_extractor::extract_melody_from_vocals(&vocals_path, get_model_dir().as_ref())
}

fn is_audio_extension(lowered_path: &str) -> bool {
    const AUDIO_EXTS: &[&str] = &[".wav", ".mp3", ".flac", ".m4a", ".aac", ".ogg", ".opus"];
    AUDIO_EXTS.iter().any(|ext| lowered_path.ends_with(ext))
}

/// 複合操作：對一個伴奏路徑自動偵測 + 載入。
///
/// 回傳：
/// - `Ok(None)` — 沒找到任何來源
/// - `Err(_)` — 找到了但載入失敗
#[tauri::command]
pub fn auto_load_melody_for_backing(backing_path: String) -> Result<Option<MelodyTrack>, AppError> {
    security::validate_path_safe(&backing_path)?;
    let path = PathBuf::from(&backing_path);
    match detect_melody_source(&path) {
        DetectedSource::None => Ok(None),
    }
}

// ── 雙檔自動對齊 ─────────────────────────────────────────────────

/// 對齊兩個音訊檔案（例如原曲 + 伴奏版），回傳時間偏移與信心指標。
///
/// 使用 FFT-based cross-correlation。`reference_path` 通常是 melody 抽取
/// 來源，`target_path` 是實際播放的伴奏檔。
#[tauri::command]
pub fn align_audio_files(
    reference_path: String,
    target_path: String,
) -> Result<AlignmentResult, AppError> {
    security::validate_path_safe(&reference_path)?;
    security::validate_path_safe(&target_path)?;
    audio_aligner::align_files(&reference_path, &target_path)
}
