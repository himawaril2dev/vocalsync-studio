//! 事件名稱常數與推送輔助函式。
//! 後端透過 Tauri Events 以 ~20Hz 頻率推送即時資料給前端。

use serde::Serialize;
use tauri::{AppHandle, Emitter};

// ── 事件名稱 ──

pub const AUDIO_PROGRESS: &str = "audio:progress";
pub const AUDIO_RMS: &str = "audio:rms";
pub const AUDIO_PITCH: &str = "audio:pitch";
pub const AUDIO_STATE_CHANGED: &str = "audio:state_changed";
pub const AUDIO_FINISHED: &str = "audio:finished";
pub const AUDIO_ERROR: &str = "audio:error";
pub const BACKING_PITCH_ANALYZING: &str = "backing_pitch:analyzing";
pub const BACKING_PITCH_READY: &str = "backing_pitch:ready";
pub const BACKING_PITCH_NOT_DETECTED: &str = "backing_pitch:not_detected";
pub const CALIBRATION_STARTED: &str = "calibration:started";
pub const CALIBRATION_BEAT_DETECTED: &str = "calibration:beat_detected";
pub const CALIBRATION_COMPLETE: &str = "calibration:complete";
pub const CALIBRATION_FAILED: &str = "calibration:failed";

// ── Payload 結構 ──

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

/// 伴奏旋律分析「啟動中」的提示（給前端顯示分析中橫幅）
#[derive(Clone, Serialize)]
pub struct BackingPitchAnalyzingPayload {
    /// 音訊長度（秒）
    pub duration: f64,
}

/// 伴奏旋律分析的品質摘要
#[derive(Clone, Serialize)]
pub struct BackingPitchQualityPayload {
    pub total_frames: usize,
    pub voiced_frames: usize,
    pub voiced_ratio: f64,
    pub mean_confidence: f64,
    /// 分析耗時（秒），便於 UI 顯示效能參考
    pub elapsed_secs: f64,
}

/// 伴奏無法偵測到主旋律時的提示資訊（給前端切自由模式）
#[derive(Clone, Serialize)]
pub struct BackingPitchNotDetectedPayload {
    pub voiced_ratio: f64,
    pub mean_confidence: f64,
    pub elapsed_secs: f64,
    pub reason: String,
}

/// 校準流程啟動：通知前端時間軸開始計時
#[derive(Clone, Serialize)]
pub struct CalibrationStartedPayload {
    pub bpm: f32,
    pub warmup_beats: u8,
    pub measurement_beats: u8,
    pub prep_ms: u32,
    pub beat_interval_ms: u32,
}

/// 單一拍偵測結果（在錄音結束後逐拍 emit，給前端動畫補上即時回饋）
#[derive(Clone, Serialize)]
pub struct CalibrationBeatPayload {
    /// 0-based 全域拍號（含暖身拍）
    pub beat_idx: u8,
    /// 此拍是否為暖身拍（不納入統計）
    pub is_warmup: bool,
    /// 是否成功偵測到 onset
    pub detected: bool,
    /// 是否被納入最終統計（暖身拍與離群值會被排除）
    pub accepted: bool,
    /// 相對於期望時刻的偏差，正值代表晚於、負值代表早於
    pub offset_ms: f64,
}

/// 校準成功完成
#[derive(Clone, Serialize)]
pub struct CalibrationCompletePayload {
    pub latency_ms: u64,
    /// 納入統計的有效拍數
    pub valid_beats: u8,
    /// 量測拍總數（不含暖身）
    pub measurement_beats: u8,
    /// 有效拍偏差的標準差，可作為信心指標
    pub std_dev_ms: f64,
}

/// 校準失敗
#[derive(Clone, Serialize)]
pub struct CalibrationFailedPayload {
    pub reason: String,
}


// ── 推送輔助 ──

pub fn emit_progress(app: &AppHandle, elapsed: f64, duration: f64) {
    let _ = app.emit(AUDIO_PROGRESS, ProgressPayload { elapsed, duration });
}

pub fn emit_rms(app: &AppHandle, backing_rms: f32, mic_rms: f32) {
    let _ = app.emit(AUDIO_RMS, RmsPayload { backing_rms, mic_rms });
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

/// 清空音高（前端把指示器歸零）
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

pub fn emit_backing_pitch_not_detected(
    app: &AppHandle,
    payload: BackingPitchNotDetectedPayload,
) {
    let _ = app.emit(BACKING_PITCH_NOT_DETECTED, payload);
}

pub fn emit_calibration_started(app: &AppHandle, payload: CalibrationStartedPayload) {
    let _ = app.emit(CALIBRATION_STARTED, payload);
}

pub fn emit_calibration_beat(app: &AppHandle, payload: CalibrationBeatPayload) {
    let _ = app.emit(CALIBRATION_BEAT_DETECTED, payload);
}

pub fn emit_calibration_complete(app: &AppHandle, payload: CalibrationCompletePayload) {
    let _ = app.emit(CALIBRATION_COMPLETE, payload);
}

pub fn emit_calibration_failed(app: &AppHandle, payload: CalibrationFailedPayload) {
    let _ = app.emit(CALIBRATION_FAILED, payload);
}

