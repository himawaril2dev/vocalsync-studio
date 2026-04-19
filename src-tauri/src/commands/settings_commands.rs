//! 設定讀寫 Commands

use crate::core::settings::AppSettings;
use crate::error::AppError;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn load_settings(settings: State<'_, Mutex<AppSettings>>) -> Result<AppSettings, AppError> {
    let settings = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(settings.clone())
}

#[tauri::command]
pub fn save_settings(
    new_settings: AppSettings,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    *current = new_settings;
    current
        .save()
        .map_err(|e| AppError::Settings(e.to_string()))
}

/// 部分更新：只寫入音高偵測引擎偏好並立即持久化。
#[tauri::command]
pub fn update_pitch_engine(
    engine: String,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    current.pitch_engine = engine;
    current
        .save()
        .map_err(|e| AppError::Settings(e.to_string()))
}

/// 部分更新：只寫入校準延遲值並立即持久化。
///
/// 用途：校準完成後前端只想更新這一欄位，不想 round-trip 整個 AppSettings。
#[tauri::command]
pub fn update_calibrated_latency(
    latency_ms: f64,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    current.calibrated_latency_ms = Some(latency_ms);
    current
        .save()
        .map_err(|e| AppError::Settings(e.to_string()))
}
