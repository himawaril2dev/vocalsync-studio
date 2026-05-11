//! Tauri commands for local Whisper transcription.

use crate::core::whisper_engine::{
    self, LocalWhisperModelCandidate, LocalWhisperRunnerCandidate, WhisperModelOption,
    WhisperToolsStatus, WhisperTranscriptionResult,
};
use crate::error::AppError;
use tauri::AppHandle;

#[tauri::command]
pub fn check_whisper_tools() -> WhisperToolsStatus {
    whisper_engine::check_whisper_tools()
}

#[tauri::command]
pub fn list_whisper_model_options() -> Vec<WhisperModelOption> {
    whisper_engine::list_whisper_model_options()
}

#[tauri::command]
pub fn inspect_local_whisper_runner_path(
    path: String,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    whisper_engine::inspect_local_whisper_runner_path(path)
}

#[tauri::command]
pub fn trust_local_whisper_runner(
    candidate: LocalWhisperRunnerCandidate,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    whisper_engine::trust_local_whisper_runner_candidate(candidate)
}

#[tauri::command]
pub fn inspect_local_whisper_model_path(
    path: String,
) -> Result<LocalWhisperModelCandidate, AppError> {
    whisper_engine::inspect_local_whisper_model_path(path)
}

#[tauri::command]
pub fn trust_local_whisper_model(
    candidate: LocalWhisperModelCandidate,
) -> Result<LocalWhisperModelCandidate, AppError> {
    whisper_engine::trust_local_whisper_model_candidate(candidate)
}

#[tauri::command]
pub fn open_whisper_model_folder() -> Result<Vec<String>, AppError> {
    whisper_engine::open_whisper_model_folder()
}

#[tauri::command]
pub async fn install_whisper_runner(
    app: AppHandle,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    tauri::async_runtime::spawn_blocking(move || whisper_engine::install_whisper_runner(&app))
        .await
        .map_err(|err| AppError::Internal(format!("Whisper runner install task failed: {}", err)))?
}

#[tauri::command]
pub async fn install_whisper_model(
    app: AppHandle,
    model_id: String,
) -> Result<LocalWhisperModelCandidate, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        whisper_engine::install_whisper_model(&app, model_id)
    })
    .await
    .map_err(|err| AppError::Internal(format!("Whisper model install task failed: {}", err)))?
}

#[tauri::command]
pub async fn activate_installed_whisper_model(
    model_id: String,
) -> Result<LocalWhisperModelCandidate, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        whisper_engine::activate_installed_whisper_model(model_id)
    })
    .await
    .map_err(|err| AppError::Internal(format!("Whisper model activation task failed: {}", err)))?
}

#[tauri::command]
pub async fn transcribe_vocals_with_whisper(
    app: AppHandle,
    audio_path: String,
    language: Option<String>,
) -> Result<WhisperTranscriptionResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        whisper_engine::transcribe_vocals_with_whisper(&app, audio_path, language)
    })
    .await
    .map_err(|err| AppError::Internal(format!("Whisper task failed: {}", err)))?
}
