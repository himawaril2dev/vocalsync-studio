use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const AUDIO_PROGRESS: &str = "audio:progress";
pub const AUDIO_RMS: &str = "audio:rms";
pub const AUDIO_PITCH: &str = "audio:pitch";
pub const AUDIO_STATE_CHANGED: &str = "audio:state_changed";
pub const AUDIO_FINISHED: &str = "audio:finished";
pub const AUDIO_ERROR: &str = "audio:error";
pub const BACKING_PITCH_ANALYZING: &str = "backing_pitch:analyzing";
pub const BACKING_PITCH_READY: &str = "backing_pitch:ready";
pub const BACKING_PITCH_NOT_DETECTED: &str = "backing_pitch:not_detected";

#[derive(Clone, Serialize)]
pub struct ProgressPayload {
    pub elapsed: f64,
    pub duration: f64,
}

#[derive(Clone, Serialize)]
pub struct RmsPayload {
    pub backing_rms: f32,
    pub mic_rms: f32,
}

#[derive(Clone, Serialize)]
pub struct PitchPayload {
    pub freq: f64,
    pub note: String,
    pub octave: i32,
    pub cent: f64,
    pub confidence: f64,
}

#[derive(Clone, Serialize)]
pub struct StatePayload {
    pub state: String,
}

#[derive(Clone, Serialize)]
pub struct ErrorPayload {
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct BackingPitchAnalyzingPayload {
    pub duration: f64,
}

#[derive(Clone, Serialize)]
pub struct BackingPitchQualityPayload {
    pub total_frames: usize,
    pub voiced_frames: usize,
    pub voiced_ratio: f64,
    pub mean_confidence: f64,
    pub elapsed_secs: f64,
}

#[derive(Clone, Serialize)]
pub struct BackingPitchNotDetectedPayload {
    pub voiced_ratio: f64,
    pub mean_confidence: f64,
    pub elapsed_secs: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationCompletePayload {
    pub mode: String,
    pub latency_ms: u64,
    pub confidence: String,
    pub rounds_used: u8,
    pub valid_beats: u8,
    pub measurement_beats: u8,
    pub std_dev_ms: f64,
    pub round_spread_ms: f64,
    pub applied_recommended: bool,
    pub diagnostic: String,
}

pub fn emit_progress(app: &AppHandle, elapsed: f64, duration: f64) {
    let _ = app.emit(AUDIO_PROGRESS, ProgressPayload { elapsed, duration });
}

pub fn emit_rms(app: &AppHandle, backing_rms: f32, mic_rms: f32) {
    let _ = app.emit(
        AUDIO_RMS,
        RmsPayload {
            backing_rms,
            mic_rms,
        },
    );
}

pub fn emit_state(app: &AppHandle, state: &str) {
    let _ = app.emit(
        AUDIO_STATE_CHANGED,
        StatePayload {
            state: state.to_string(),
        },
    );
}

pub fn emit_pitch(app: &AppHandle, payload: PitchPayload) {
    let _ = app.emit(AUDIO_PITCH, payload);
}

pub fn emit_pitch_none(app: &AppHandle) {
    let _ = app.emit(AUDIO_PITCH, serde_json::Value::Null);
}

pub fn emit_finished(app: &AppHandle) {
    let _ = app.emit(AUDIO_FINISHED, ());
}

pub fn emit_error(app: &AppHandle, message: &str) {
    let _ = app.emit(
        AUDIO_ERROR,
        ErrorPayload {
            message: message.to_string(),
        },
    );
}

pub fn emit_backing_pitch_analyzing(app: &AppHandle, payload: BackingPitchAnalyzingPayload) {
    let _ = app.emit(BACKING_PITCH_ANALYZING, payload);
}

pub fn emit_backing_pitch_ready(app: &AppHandle, payload: BackingPitchQualityPayload) {
    let _ = app.emit(BACKING_PITCH_READY, payload);
}

pub fn emit_backing_pitch_not_detected(app: &AppHandle, payload: BackingPitchNotDetectedPayload) {
    let _ = app.emit(BACKING_PITCH_NOT_DETECTED, payload);
}
