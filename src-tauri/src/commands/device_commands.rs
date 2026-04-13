//! 裝置列舉 Commands

use crate::core::audio_engine::{AudioEngine, DeviceList};
use crate::error::AppError;

#[tauri::command]
pub fn list_devices() -> Result<DeviceList, AppError> {
    AudioEngine::list_devices()
}
