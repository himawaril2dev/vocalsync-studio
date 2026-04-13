//! 音訊操作 Commands

use crate::core::audio_engine::{AudioEngine, ExportPaths, LoadResult};
use crate::core::pitch_data::PitchTrack;
use crate::error::AppError;
use std::sync::Mutex;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn load_backing(
    _app: AppHandle,
    path: String,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<LoadResult, AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut result = engine.load_backing(&path)?;
    result.melody_source = None;

    Ok(result)
}

#[tauri::command]
pub fn start_recording(
    app: AppHandle,
    input_device: Option<usize>,
    output_device: Option<usize>,
    engine: State<'_, Mutex<AudioEngine>>,
    settings: State<'_, Mutex<crate::core::settings::AppSettings>>,
) -> Result<(), AppError> {
    let pitch_engine = settings
        .lock()
        .map(|s| s.pitch_engine.clone())
        .unwrap_or_else(|_| "auto".to_string());
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.start_recording(app, input_device, output_device, &pitch_engine)
}

#[tauri::command]
pub fn stop_recording(engine: State<'_, Mutex<AudioEngine>>) -> Result<(), AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.stop();
    Ok(())
}

#[tauri::command]
pub fn start_preview(
    app: AppHandle,
    start_frame: Option<u64>,
    output_device: Option<usize>,
    input_device: Option<usize>,
    engine: State<'_, Mutex<AudioEngine>>,
    settings: State<'_, Mutex<crate::core::settings::AppSettings>>,
) -> Result<(), AppError> {
    let pitch_engine = settings
        .lock()
        .map(|s| s.pitch_engine.clone())
        .unwrap_or_else(|_| "auto".to_string());
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.start_preview(app, start_frame, output_device, input_device, &pitch_engine)
}

#[tauri::command]
pub fn start_playback(
    app: AppHandle,
    start_frame: Option<u64>,
    output_device: Option<usize>,
    latency_ms: f64,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<(), AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.start_playback(app, start_frame, output_device, latency_ms)
}

#[tauri::command]
pub fn pause_playback(engine: State<'_, Mutex<AudioEngine>>) -> Result<u64, AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(engine.pause())
}

#[tauri::command]
pub fn seek(seconds: f64, engine: State<'_, Mutex<AudioEngine>>) -> Result<(), AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.seek(seconds);
    Ok(())
}

#[tauri::command]
pub fn set_volume(
    backing: f32,
    mic: f32,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<(), AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.set_volume(backing, mic);
    Ok(())
}

#[tauri::command]
pub fn export_audio(
    dir: String,
    prefix: String,
    auto_balance: bool,
    latency_ms: f64,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<ExportPaths, AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.export(&dir, &prefix, auto_balance, latency_ms)
}

#[tauri::command]
pub fn calibrate_latency(
    app: AppHandle,
    input_device: Option<usize>,
    output_device: Option<usize>,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<u64, AppError> {
    let mut engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.calibrate_latency(app, input_device, output_device)
}

#[tauri::command]
pub fn get_pitch_track(
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<PitchTrack, AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(engine.get_pitch_track())
}

#[tauri::command]
pub fn get_backing_pitch_track(
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<Option<PitchTrack>, AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(engine.get_backing_pitch_track())
}

#[tauri::command]
pub fn set_loop_points(
    a_secs: f64,
    b_secs: f64,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<(), AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.set_loop_points(a_secs, b_secs);
    Ok(())
}

#[tauri::command]
pub fn clear_loop(engine: State<'_, Mutex<AudioEngine>>) -> Result<(), AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.clear_loop();
    Ok(())
}

#[tauri::command]
pub fn get_loop_points(
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<Option<(f64, f64)>, AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(engine.get_loop_points())
}

#[tauri::command]
pub fn set_speed(
    speed: f64,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<(), AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.set_speed(speed);
    Ok(())
}

#[tauri::command]
pub fn get_speed(
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<f64, AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(engine.get_speed())
}

#[tauri::command]
pub fn set_pitch_semitones(
    semitones: i32,
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<(), AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    engine.set_pitch_semitones(semitones);
    Ok(())
}

#[tauri::command]
pub fn get_pitch_semitones(
    engine: State<'_, Mutex<AudioEngine>>,
) -> Result<i32, AppError> {
    let engine = engine
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(engine.get_pitch_semitones())
}
