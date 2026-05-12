//! 設定讀寫 Commands

use crate::core::portable_paths;
use crate::core::settings::AppSettings;
use crate::error::AppError;
use crate::security;
use serde_json::{Map, Number, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const PROJECT_SESSION_FILE: &str = "project-session.json";
const MAX_PROJECT_SESSION_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SESSION_STRING_CHARS: usize = 4096;
const MAX_PATH_CHARS: usize = 2048;
const MAX_LYRIC_LINES: usize = 5000;
const MAX_LYRIC_TEXT_CHARS: usize = 2000;
const MAX_MELODY_NOTES: usize = 50_000;
const MAX_RAW_PITCH_SAMPLES: usize = 300_000;
const MAX_DURATION_SECS: f64 = 24.0 * 60.0 * 60.0;
const MAX_FINE_TUNE_MS: f64 = 10.0 * 60.0 * 1000.0;
const MAX_RESTORED_MEDIA_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_ALIGNMENT_CORRELATION: f64 = 1.0e18;
const MAX_ALIGNMENT_RATIO: f64 = 1.0e12;

const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "flac", "m4a", "aac", "ogg", "opus", "mp4", "mov", "mkv", "webm",
];
const GUIDE_VOCAL_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus"];
const MELODY_SOURCE_EXTENSIONS: &[&str] = &[
    "mid", "midi", "wav", "mp3", "flac", "m4a", "aac", "ogg", "opus",
];

#[derive(Debug, Clone, Copy)]
enum SessionPathMode {
    Runtime,
    Persisted,
}

#[tauri::command]
pub fn load_settings(settings: State<'_, Mutex<AppSettings>>) -> Result<AppSettings, AppError> {
    let settings = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(settings.clone())
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

#[tauri::command]
pub fn update_show_startup_guide(
    show: bool,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    current.show_startup_guide = show;
    current
        .save()
        .map_err(|e| AppError::Settings(e.to_string()))
}

fn ratio_to_percent(value: f32, max_percent: u16) -> u16 {
    if !value.is_finite() {
        return 0;
    }
    let percent = (value * 100.0).round();
    percent.clamp(0.0, max_percent as f32) as u16
}

#[tauri::command]
pub fn update_mixer_settings(
    backing: f32,
    mic: f32,
    guide: f32,
    auto_balance: bool,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    current.backing_volume = ratio_to_percent(backing, 100);
    current.mic_gain = ratio_to_percent(mic, 300);
    current.guide_volume = ratio_to_percent(guide, 100);
    current.auto_balance = auto_balance;
    current.mixer_settings_version = 1;
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

#[tauri::command]
pub fn load_project_session() -> Result<Option<String>, AppError> {
    let path = portable_paths::path(PROJECT_SESSION_FILE);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(AppError::Io(err)),
    };
    if metadata.len() > MAX_PROJECT_SESSION_BYTES {
        quarantine_project_session(&path)?;
        return Ok(None);
    }

    match fs::read_to_string(&path) {
        Ok(contents) => {
            match parse_and_sanitize_project_session(&contents, SessionPathMode::Runtime) {
                Ok(runtime_session) => {
                    let persisted_session =
                        parse_and_sanitize_project_session(&contents, SessionPathMode::Persisted)?;
                    if persisted_session != contents {
                        write_project_session(&path, &persisted_session)?;
                    }
                    Ok(Some(runtime_session))
                }
                Err(_) => {
                    quarantine_project_session(&path)?;
                    Ok(None)
                }
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(AppError::Io(err)),
    }
}

#[tauri::command]
pub fn save_project_session(session_json: String) -> Result<(), AppError> {
    if session_json.len() as u64 > MAX_PROJECT_SESSION_BYTES {
        return Err(AppError::Settings("Project session is too large".into()));
    }
    let sanitized = parse_and_sanitize_project_session(&session_json, SessionPathMode::Persisted)?;
    if sanitized.len() as u64 > MAX_PROJECT_SESSION_BYTES {
        return Err(AppError::Settings("Project session is too large".into()));
    }
    let path = portable_paths::path(PROJECT_SESSION_FILE);
    write_project_session(&path, &sanitized)
}

#[tauri::command]
pub fn clear_project_session() -> Result<(), AppError> {
    let path = portable_paths::path(PROJECT_SESSION_FILE);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::Io(err)),
    }
}

fn parse_and_sanitize_project_session(
    session_json: &str,
    path_mode: SessionPathMode,
) -> Result<String, AppError> {
    let value = serde_json::from_str::<Value>(session_json)
        .map_err(|e| AppError::Settings(format!("Invalid project session JSON: {e}")))?;
    let sanitized = sanitize_project_session_value_for_mode(&value, path_mode)?;
    serde_json::to_string(&sanitized)
        .map_err(|e| AppError::Settings(format!("Could not serialize project session: {e}")))
}

fn write_project_session(path: &Path, contents: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AppError::Io)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, contents).map_err(AppError::Io)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(AppError::Io)?;
    }
    fs::rename(&tmp_path, path).map_err(AppError::Io)
}

fn quarantine_project_session(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let quarantine = path.with_file_name(format!("project-session.invalid-{suffix}.json"));
    match fs::rename(path, quarantine) {
        Ok(()) => Ok(()),
        Err(_) => fs::remove_file(path).map_err(AppError::Io),
    }
}

#[cfg(test)]
fn sanitize_project_session_value(value: &Value) -> Result<Value, AppError> {
    sanitize_project_session_value_for_mode(value, SessionPathMode::Persisted)
}

fn sanitize_project_session_value_for_mode(
    value: &Value,
    path_mode: SessionPathMode,
) -> Result<Value, AppError> {
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::Settings("Project session must be a JSON object".into()))?;

    let mut out = Map::new();
    out.insert("version".into(), Value::Number(Number::from(1)));
    out.insert(
        "backingPath".into(),
        sanitize_restore_path_value_for_mode(obj.get("backingPath"), AUDIO_EXTENSIONS, path_mode),
    );
    out.insert(
        "lyricsFileName".into(),
        sanitize_string_value(obj.get("lyricsFileName"), 255)
            .map(Value::String)
            .unwrap_or_else(|| Value::String(String::new())),
    );
    out.insert(
        "lyricsLines".into(),
        Value::Array(sanitize_lyrics_lines(obj.get("lyricsLines"))),
    );
    out.insert(
        "melody".into(),
        sanitize_melody(obj.get("melody"), path_mode).unwrap_or(Value::Null),
    );
    out.insert(
        "melodySourcePath".into(),
        sanitize_restore_path_value_for_mode(
            obj.get("melodySourcePath"),
            MELODY_SOURCE_EXTENSIONS,
            path_mode,
        ),
    );
    out.insert(
        "guideVocalPath".into(),
        sanitize_restore_path_value_for_mode(
            obj.get("guideVocalPath"),
            GUIDE_VOCAL_EXTENSIONS,
            path_mode,
        ),
    );
    out.insert(
        "alignmentFineTuneMs".into(),
        finite_number_value(
            obj.get("alignmentFineTuneMs"),
            -MAX_FINE_TUNE_MS,
            MAX_FINE_TUNE_MS,
        )
        .unwrap_or_else(|| Value::Number(Number::from(0))),
    );
    out.insert(
        "alignmentResult".into(),
        sanitize_alignment_result(obj.get("alignmentResult")).unwrap_or(Value::Null),
    );

    Ok(Value::Object(out))
}

fn sanitize_restore_path_value_for_mode(
    value: Option<&Value>,
    allowed_extensions: &[&str],
    path_mode: SessionPathMode,
) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(|path| sanitize_restore_path(path, allowed_extensions, path_mode).ok())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn sanitize_restore_path(
    path: &str,
    allowed_extensions: &[&str],
    path_mode: SessionPathMode,
) -> Result<String, AppError> {
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(AppError::Settings("Restored path is too long".into()));
    }
    let raw = portable_paths::resolve_stored_path_text(path)
        .ok_or_else(|| AppError::Settings("Restored path is invalid".into()))?;
    let raw_text = portable_paths::display_path(&raw);
    security::validate_local_path_safe(&raw_text)?;
    let canonical = raw.canonicalize().map_err(AppError::Io)?;
    let canonical_text = portable_paths::display_path(&canonical);
    security::validate_local_path_safe(&canonical_text)?;
    if !canonical.is_file() {
        return Err(AppError::Settings("Restored path must be a file".into()));
    }
    let extension_allowed = canonical
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            allowed_extensions
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        });
    if !extension_allowed {
        return Err(AppError::Settings(
            "Restored path has an unsupported extension".into(),
        ));
    }
    let len = fs::metadata(&canonical).map_err(AppError::Io)?.len();
    if len == 0 || len > MAX_RESTORED_MEDIA_BYTES {
        return Err(AppError::Settings(
            "Restored file size is outside the allowed range".into(),
        ));
    }
    Ok(match path_mode {
        SessionPathMode::Runtime => canonical_text,
        SessionPathMode::Persisted => portable_paths::encode_path_for_storage(&canonical),
    })
}

fn sanitize_local_path_string(
    value: Option<&Value>,
    allowed_extensions: &[&str],
    path_mode: SessionPathMode,
) -> Option<String> {
    let path = value?.as_str()?;
    if path.chars().count() > MAX_PATH_CHARS {
        return None;
    }
    let resolved = portable_paths::resolve_stored_path_text(path)?;
    let resolved_text = portable_paths::display_path(&resolved);
    security::validate_local_path_safe(&resolved_text).ok()?;
    let extension_allowed = resolved
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            allowed_extensions
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        });
    extension_allowed.then(|| match path_mode {
        SessionPathMode::Runtime => resolved_text,
        SessionPathMode::Persisted => portable_paths::encode_path_for_storage(&resolved),
    })
}

fn sanitize_lyrics_lines(value: Option<&Value>) -> Vec<Value> {
    let Some(lines) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    lines
        .iter()
        .take(MAX_LYRIC_LINES)
        .filter_map(|line| {
            let obj = line.as_object()?;
            let start_ms = finite_number(obj.get("start_ms"), 0.0, MAX_DURATION_SECS * 1000.0)?;
            let end_ms = finite_number(obj.get("end_ms"), start_ms, MAX_DURATION_SECS * 1000.0)?;
            let text = sanitize_string_value(obj.get("text"), MAX_LYRIC_TEXT_CHARS)?;
            let mut out = Map::new();
            out.insert("start_ms".into(), number_from_f64(start_ms)?);
            out.insert("end_ms".into(), number_from_f64(end_ms)?);
            out.insert("text".into(), Value::String(text));
            if let Some(translation) =
                sanitize_string_value(obj.get("translation"), MAX_LYRIC_TEXT_CHARS)
            {
                out.insert("translation".into(), Value::String(translation));
            }
            Some(Value::Object(out))
        })
        .collect()
}

fn sanitize_melody(value: Option<&Value>, path_mode: SessionPathMode) -> Option<Value> {
    let obj = value?.as_object()?;
    let source = sanitize_melody_source(obj.get("source"), path_mode)?;
    let notes = obj.get("notes")?.as_array()?;
    if notes.len() > MAX_MELODY_NOTES {
        return None;
    }

    let mut out = Map::new();
    out.insert("source".into(), source);
    out.insert(
        "notes".into(),
        Value::Array(notes.iter().filter_map(sanitize_melody_note).collect()),
    );
    out.insert(
        "total_duration_secs".into(),
        finite_number_value(obj.get("total_duration_secs"), 0.0, MAX_DURATION_SECS)?,
    );
    if let Some(raw_pitch_track) = sanitize_raw_pitch_track(obj.get("raw_pitch_track")) {
        out.insert("raw_pitch_track".into(), raw_pitch_track);
    }
    Some(Value::Object(out))
}

fn sanitize_melody_source(value: Option<&Value>, path_mode: SessionPathMode) -> Option<Value> {
    let obj = value?.as_object()?;
    let source_type = obj.get("type")?.as_str()?;
    let mut out = Map::new();
    out.insert("type".into(), Value::String(source_type.to_string()));
    match source_type {
        "midi" => {
            out.insert(
                "mid_path".into(),
                sanitize_local_path_string(obj.get("mid_path"), &["mid", "midi"], path_mode)
                    .map(Value::String)
                    .unwrap_or_else(|| Value::String(String::new())),
            );
            out.insert(
                "track_index".into(),
                finite_number_value(obj.get("track_index"), 0.0, 1024.0)?,
            );
            out.insert(
                "track_name".into(),
                sanitize_string_value(obj.get("track_name"), 255)
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            );
        }
        "vocal_separation" => {
            out.insert(
                "cache_path".into(),
                sanitize_local_path_string(obj.get("cache_path"), &["json"], path_mode)
                    .map(Value::String)
                    .unwrap_or_else(|| Value::String(String::new())),
            );
            out.insert(
                "model".into(),
                sanitize_string_value(obj.get("model"), MAX_SESSION_STRING_CHARS)
                    .map(Value::String)
                    .unwrap_or_else(|| Value::String(String::new())),
            );
            out.insert(
                "file_hash".into(),
                sanitize_string_value(obj.get("file_hash"), 128)
                    .map(Value::String)
                    .unwrap_or_else(|| Value::String(String::new())),
            );
        }
        "imported_vocals" => {
            out.insert(
                "vocals_path".into(),
                sanitize_local_path_string(
                    obj.get("vocals_path"),
                    GUIDE_VOCAL_EXTENSIONS,
                    path_mode,
                )
                .map(Value::String)
                .unwrap_or_else(|| Value::String(String::new())),
            );
            out.insert(
                "note_count".into(),
                finite_number_value(obj.get("note_count"), 0.0, MAX_MELODY_NOTES as f64)?,
            );
            out.insert(
                "voiced_ratio".into(),
                finite_number_value(obj.get("voiced_ratio"), 0.0, 1.0)?,
            );
        }
        _ => return None,
    }
    Some(Value::Object(out))
}

fn sanitize_melody_note(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut out = Map::new();
    out.insert(
        "start_secs".into(),
        finite_number_value(obj.get("start_secs"), -MAX_DURATION_SECS, MAX_DURATION_SECS)?,
    );
    out.insert(
        "duration_secs".into(),
        finite_number_value(obj.get("duration_secs"), 0.0, MAX_DURATION_SECS)?,
    );
    out.insert(
        "midi_pitch".into(),
        finite_number_value(obj.get("midi_pitch"), 0.0, 127.0)?,
    );
    out.insert(
        "freq_hz".into(),
        finite_number_value(obj.get("freq_hz"), 1.0, 25_000.0)?,
    );
    out.insert(
        "lyric".into(),
        sanitize_string_value(obj.get("lyric"), MAX_SESSION_STRING_CHARS)
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    out.insert(
        "is_golden".into(),
        Value::Bool(
            obj.get("is_golden")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
    );
    out.insert(
        "is_freestyle".into(),
        Value::Bool(
            obj.get("is_freestyle")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
    );
    Some(Value::Object(out))
}

fn sanitize_raw_pitch_track(value: Option<&Value>) -> Option<Value> {
    let samples = value?.as_array()?;
    if samples.len() > MAX_RAW_PITCH_SAMPLES {
        return None;
    }
    let out = samples
        .iter()
        .filter_map(|sample| {
            let obj = sample.as_object()?;
            let mut sample_out = Map::new();
            sample_out.insert(
                "timestamp".into(),
                finite_number_value(obj.get("timestamp"), -MAX_DURATION_SECS, MAX_DURATION_SECS)?,
            );
            sample_out.insert(
                "freq".into(),
                finite_number_value(obj.get("freq"), 0.0, 25_000.0)?,
            );
            sample_out.insert(
                "confidence".into(),
                finite_number_value(obj.get("confidence"), 0.0, 1.0)?,
            );
            if let Some(note) = sanitize_string_value(obj.get("note"), 16) {
                sample_out.insert("note".into(), Value::String(note));
            }
            if let Some(octave) = finite_number_value(obj.get("octave"), -2.0, 10.0) {
                sample_out.insert("octave".into(), octave);
            }
            if let Some(cent) = finite_number_value(obj.get("cent"), -1200.0, 1200.0) {
                sample_out.insert("cent".into(), cent);
            }
            Some(Value::Object(sample_out))
        })
        .collect();
    Some(Value::Array(out))
}

fn sanitize_alignment_result(value: Option<&Value>) -> Option<Value> {
    let obj = value?.as_object()?;
    let mut out = Map::new();
    out.insert(
        "offset_secs".into(),
        finite_number_value(
            obj.get("offset_secs"),
            -MAX_DURATION_SECS,
            MAX_DURATION_SECS,
        )?,
    );
    out.insert(
        "peak_correlation".into(),
        finite_number_value(
            obj.get("peak_correlation"),
            -MAX_ALIGNMENT_CORRELATION,
            MAX_ALIGNMENT_CORRELATION,
        )?,
    );
    out.insert(
        "peak_to_mean_ratio".into(),
        finite_number_value(obj.get("peak_to_mean_ratio"), 0.0, MAX_ALIGNMENT_RATIO)?,
    );
    out.insert(
        "sample_rate".into(),
        finite_number_value(obj.get("sample_rate"), 1.0, 384_000.0)?,
    );
    out.insert(
        "reference_duration_secs".into(),
        finite_number_value(obj.get("reference_duration_secs"), 0.0, MAX_DURATION_SECS)?,
    );
    out.insert(
        "target_duration_secs".into(),
        finite_number_value(obj.get("target_duration_secs"), 0.0, MAX_DURATION_SECS)?,
    );
    Some(Value::Object(out))
}

fn sanitize_string_value(value: Option<&Value>, max_chars: usize) -> Option<String> {
    let raw = value?.as_str()?;
    if raw.chars().any(|ch| ch == '\0') {
        return None;
    }
    let mut text = String::new();
    for ch in raw.chars().filter(|ch| !ch.is_control()) {
        if text.chars().count() >= max_chars {
            break;
        }
        text.push(ch);
    }
    Some(text)
}

fn finite_number_value(value: Option<&Value>, min: f64, max: f64) -> Option<Value> {
    number_from_f64(finite_number(value, min, max)?)
}

fn finite_number(value: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    let number = value?.as_f64()?;
    (number.is_finite() && number >= min && number <= max).then_some(number)
}

fn number_from_f64(value: f64) -> Option<Value> {
    Number::from_f64(value).map(Value::Number)
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn session_sanitizer_caps_large_arrays() {
        let lines = (0..(MAX_LYRIC_LINES + 10))
            .map(|i| {
                serde_json::json!({
                    "start_ms": i,
                    "end_ms": i + 1,
                    "text": "line"
                })
            })
            .collect::<Vec<_>>();
        let sanitized = sanitize_project_session_value(&serde_json::json!({
            "version": 1,
            "lyricsLines": lines,
            "alignmentFineTuneMs": 0
        }))
        .unwrap();
        assert_eq!(
            sanitized["lyricsLines"].as_array().unwrap().len(),
            MAX_LYRIC_LINES
        );
    }

    #[test]
    fn session_sanitizer_rejects_non_objects() {
        assert!(sanitize_project_session_value(&serde_json::json!([])).is_err());
    }

    #[test]
    fn session_sanitizer_preserves_alignment_with_large_correlation() {
        let sanitized = sanitize_project_session_value(&serde_json::json!({
            "version": 1,
            "alignmentFineTuneMs": 123,
            "alignmentResult": {
                "offset_secs": 1.25,
                "peak_correlation": 123456789.0,
                "peak_to_mean_ratio": 25000.0,
                "sample_rate": 11025,
                "reference_duration_secs": 180.0,
                "target_duration_secs": 180.0
            }
        }))
        .unwrap();

        assert_eq!(sanitized["alignmentFineTuneMs"].as_f64(), Some(123.0));
        assert!(sanitized["alignmentResult"].is_object());
        assert_eq!(
            sanitized["alignmentResult"]["peak_correlation"].as_f64(),
            Some(123456789.0)
        );
    }
}
