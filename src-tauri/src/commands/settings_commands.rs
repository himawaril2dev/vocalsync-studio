//! 設定讀寫 Commands

use crate::core::portable_paths;
use crate::core::settings::{AppSettings, LatencyCalibrationProfile};
use crate::error::AppError;
use crate::security;
use serde::Serialize;
use serde_json::{Map, Number, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const PROJECT_SESSION_FILE: &str = "project-session.json";
const SONG_LIBRARY_FILE: &str = "song-library.json";
const SONG_LIBRARY_BACKUP_PREFIX: &str = "song-library.backup-";
const MAX_SONG_LIBRARY_BACKUPS: usize = 7;
const MAX_PROJECT_SESSION_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SONG_LIBRARY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SONG_PROFILES: usize = 500;
const MAX_SONG_ID_CHARS: usize = 80;
const MAX_SONG_TITLE_CHARS: usize = 160;
const MAX_SONG_ARTIST_CHARS: usize = 160;
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
const MAX_DEVICE_NAME_CHARS: usize = 160;
const MAX_LATENCY_CALIBRATION_PROFILES: usize = 100;

static SESSION_FILE_LOCK: Mutex<()> = Mutex::new(());

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

#[derive(Debug, Clone, Serialize)]
struct SongProfileRecord {
    id: String,
    title: String,
    artist: Option<String>,
    created_at_unix: u64,
    updated_at_unix: u64,
    last_opened_at_unix: u64,
    session: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongProfileStatus {
    backing_set: bool,
    backing_exists: bool,
    guide_vocal_set: bool,
    guide_vocal_exists: bool,
    lyrics_count: usize,
    timed_lyrics_count: usize,
    melody_present: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongProfileSummary {
    id: String,
    title: String,
    artist: Option<String>,
    created_at_unix: u64,
    updated_at_unix: u64,
    last_opened_at_unix: u64,
    status: SongProfileStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongProfilePayload {
    profile: SongProfileSummary,
    session_json: String,
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

fn normalize_export_naming_mode(value: &str) -> String {
    match value {
        "auto" => "auto".to_string(),
        _ => "manual".to_string(),
    }
}

fn normalize_auto_balance_vocal_preset(value: &str) -> String {
    match value {
        "natural" => "natural".to_string(),
        "clear" => "clear".to_string(),
        "forward" => "forward".to_string(),
        _ => "natural".to_string(),
    }
}

#[tauri::command]
pub fn update_mixer_settings(
    backing: f32,
    mic: f32,
    guide: f32,
    auto_balance: bool,
    auto_balance_vocal_preset: String,
    export_naming_mode: String,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    current.backing_volume = ratio_to_percent(backing, 100);
    current.mic_gain = ratio_to_percent(mic, 300);
    current.guide_volume = ratio_to_percent(guide, 100);
    current.auto_balance = auto_balance;
    current.auto_balance_vocal_preset =
        normalize_auto_balance_vocal_preset(&auto_balance_vocal_preset);
    current.export_naming_mode = normalize_export_naming_mode(&export_naming_mode);
    current.mixer_settings_version = 7;
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
    input_device_name: Option<String>,
    output_device_name: Option<String>,
    sample_rate: Option<u32>,
    confidence: Option<String>,
    settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), AppError> {
    if !latency_ms.is_finite() || !(0.0..=5000.0).contains(&latency_ms) {
        return Err(AppError::Settings(
            "Latency value is outside the allowed range".into(),
        ));
    }

    let mut current = settings
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    current.calibrated_latency_ms = Some(latency_ms);

    let input_device = sanitize_profile_text(input_device_name.as_deref());
    let output_device = sanitize_profile_text(output_device_name.as_deref());
    if let (Some(input_device), Some(output_device), Some(sample_rate)) =
        (input_device, output_device, sample_rate)
    {
        if (8_000..=384_000).contains(&sample_rate) {
            let key = latency_profile_key(&input_device, &output_device, sample_rate);
            let updated_at_unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            let profile = LatencyCalibrationProfile {
                key: key.clone(),
                input_device,
                output_device,
                sample_rate,
                latency_ms,
                confidence: sanitize_profile_text(confidence.as_deref())
                    .unwrap_or_else(|| "manual".to_string()),
                updated_at_unix,
            };

            if let Some(existing) = current
                .calibrated_latency_profiles
                .iter_mut()
                .find(|existing| existing.key == key)
            {
                *existing = profile;
            } else {
                current.calibrated_latency_profiles.push(profile);
            }
            trim_latency_profiles(&mut current.calibrated_latency_profiles);
        }
    }

    current
        .save()
        .map_err(|e| AppError::Settings(e.to_string()))
}

fn sanitize_profile_text(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() || raw.chars().any(|ch| ch == '\0') {
        return None;
    }
    let mut out = String::new();
    for ch in raw.chars().filter(|ch| !ch.is_control()) {
        if out.chars().count() >= MAX_DEVICE_NAME_CHARS {
            break;
        }
        out.push(ch);
    }
    (!out.is_empty()).then_some(out)
}

fn latency_profile_key(input_device: &str, output_device: &str, sample_rate: u32) -> String {
    format!(
        "{}|{}|{}",
        input_device.trim().to_lowercase(),
        output_device.trim().to_lowercase(),
        sample_rate
    )
}

fn acquire_session_file_lock() -> Result<MutexGuard<'static, ()>, AppError> {
    SESSION_FILE_LOCK
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))
}

fn trim_latency_profiles(profiles: &mut Vec<LatencyCalibrationProfile>) {
    profiles.sort_by(|a, b| {
        b.updated_at_unix
            .cmp(&a.updated_at_unix)
            .then_with(|| a.key.cmp(&b.key))
    });
    profiles.truncate(MAX_LATENCY_CALIBRATION_PROFILES);
}

#[tauri::command]
pub fn load_project_session() -> Result<Option<String>, AppError> {
    let _file_guard = acquire_session_file_lock()?;
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
    let _file_guard = acquire_session_file_lock()?;
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
    let _file_guard = acquire_session_file_lock()?;
    let path = portable_paths::path(PROJECT_SESSION_FILE);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::Io(err)),
    }
}

#[tauri::command]
pub fn list_song_profiles() -> Result<Vec<SongProfileSummary>, AppError> {
    let _file_guard = acquire_session_file_lock()?;
    let mut songs = load_song_library_records()?;
    songs.sort_by(|a, b| {
        b.last_opened_at_unix
            .cmp(&a.last_opened_at_unix)
            .then_with(|| b.updated_at_unix.cmp(&a.updated_at_unix))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    Ok(songs.iter().map(song_profile_summary).collect())
}

#[tauri::command]
pub fn save_song_profile(
    profile_id: Option<String>,
    title: String,
    artist: Option<String>,
    session_json: String,
) -> Result<SongProfileSummary, AppError> {
    let _file_guard = acquire_session_file_lock()?;
    if session_json.len() as u64 > MAX_PROJECT_SESSION_BYTES {
        return Err(AppError::Settings(
            "Song profile session is too large".into(),
        ));
    }
    let session = parse_and_sanitize_song_session(&session_json, SessionPathMode::Persisted)?;
    let title = sanitize_text_input(&title, MAX_SONG_TITLE_CHARS)
        .ok_or_else(|| AppError::Settings("Song title is required".into()))?;
    let artist = artist
        .as_deref()
        .and_then(|value| sanitize_text_input(value, MAX_SONG_ARTIST_CHARS));
    let now = current_unix_secs();
    let mut songs = load_song_library_records()?;

    let summary = if let Some(raw_id) = profile_id {
        let id = sanitize_song_id(&raw_id)
            .ok_or_else(|| AppError::Settings("Song profile id is invalid".into()))?;
        let Some(existing) = songs.iter_mut().find(|song| song.id == id) else {
            return Err(AppError::Settings("Song profile was not found".into()));
        };
        existing.title = title;
        existing.artist = artist;
        existing.updated_at_unix = now;
        existing.last_opened_at_unix = now;
        existing.session = session;
        song_profile_summary(existing)
    } else {
        if songs.len() >= MAX_SONG_PROFILES {
            return Err(AppError::Settings(format!(
                "Song library can store up to {MAX_SONG_PROFILES} profiles"
            )));
        }
        let record = SongProfileRecord {
            id: generate_song_profile_id(&songs, now),
            title,
            artist,
            created_at_unix: now,
            updated_at_unix: now,
            last_opened_at_unix: now,
            session,
        };
        let summary = song_profile_summary(&record);
        songs.push(record);
        summary
    };

    write_song_library_records(&songs)?;
    Ok(summary)
}

#[tauri::command]
pub fn load_song_profile(profile_id: String) -> Result<SongProfilePayload, AppError> {
    let _file_guard = acquire_session_file_lock()?;
    let id = sanitize_song_id(&profile_id)
        .ok_or_else(|| AppError::Settings("Song profile id is invalid".into()))?;
    let mut songs = load_song_library_records()?;
    let Some(index) = songs.iter().position(|song| song.id == id) else {
        return Err(AppError::Settings("Song profile was not found".into()));
    };

    songs[index].last_opened_at_unix = current_unix_secs();
    let runtime_session =
        sanitize_song_session_value_for_mode(&songs[index].session, SessionPathMode::Runtime)?;
    let session_json = serde_json::to_string(&runtime_session)
        .map_err(|e| AppError::Settings(format!("Could not serialize song session: {e}")))?;
    let profile = song_profile_summary(&songs[index]);
    write_song_library_records(&songs)?;

    Ok(SongProfilePayload {
        profile,
        session_json,
    })
}

#[tauri::command]
pub fn rename_song_profile(
    profile_id: String,
    title: String,
    artist: Option<String>,
) -> Result<SongProfileSummary, AppError> {
    let _file_guard = acquire_session_file_lock()?;
    let id = sanitize_song_id(&profile_id)
        .ok_or_else(|| AppError::Settings("Song profile id is invalid".into()))?;
    let title = sanitize_text_input(&title, MAX_SONG_TITLE_CHARS)
        .ok_or_else(|| AppError::Settings("Song title is required".into()))?;
    let artist = artist
        .as_deref()
        .and_then(|value| sanitize_text_input(value, MAX_SONG_ARTIST_CHARS));
    let mut songs = load_song_library_records()?;
    let Some(existing) = songs.iter_mut().find(|song| song.id == id) else {
        return Err(AppError::Settings("Song profile was not found".into()));
    };
    existing.title = title;
    existing.artist = artist;
    existing.updated_at_unix = current_unix_secs();
    let summary = song_profile_summary(existing);
    write_song_library_records(&songs)?;
    Ok(summary)
}

#[tauri::command]
pub fn delete_song_profile(profile_id: String) -> Result<(), AppError> {
    let _file_guard = acquire_session_file_lock()?;
    let id = sanitize_song_id(&profile_id)
        .ok_or_else(|| AppError::Settings("Song profile id is invalid".into()))?;
    let mut songs = load_song_library_records()?;
    let original_len = songs.len();
    songs.retain(|song| song.id != id);
    if songs.len() == original_len {
        return Err(AppError::Settings("Song profile was not found".into()));
    }
    write_song_library_records(&songs)
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

fn load_song_library_records() -> Result<Vec<SongProfileRecord>, AppError> {
    let path = portable_paths::path(SONG_LIBRARY_FILE);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(AppError::Io(err)),
    };
    if metadata.len() > MAX_SONG_LIBRARY_BYTES {
        quarantine_song_library(&path)?;
        return Ok(Vec::new());
    }

    match fs::read_to_string(&path) {
        Ok(contents) => match parse_song_library_records(&contents, SessionPathMode::Persisted) {
            Ok(songs) => {
                let normalized = serialize_song_library_records(&songs)?;
                if normalized != contents {
                    write_song_library_records(&songs)?;
                }
                Ok(songs)
            }
            Err(_) => {
                quarantine_song_library(&path)?;
                Ok(Vec::new())
            }
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(AppError::Io(err)),
    }
}

fn write_song_library_records(songs: &[SongProfileRecord]) -> Result<(), AppError> {
    let path = portable_paths::path(SONG_LIBRARY_FILE);
    let contents = serialize_song_library_records(songs)?;
    if contents.len() as u64 > MAX_SONG_LIBRARY_BYTES {
        return Err(AppError::Settings("Song library is too large".into()));
    }
    backup_song_library_before_write(&path);
    write_project_session(&path, &contents)
}

/// UTC 民用日期換算（Howard Hinnant civil_from_days），避免引入日期時間相依套件
fn unix_secs_to_utc_ymd(secs: u64) -> (i64, u32, u32) {
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// 每天第一次寫入歌單前，把現有檔案複製為當日備份；備份失敗不阻擋正常儲存
fn backup_song_library_before_write(path: &Path) {
    if !path.is_file() {
        return;
    }
    let (year, month, day) = unix_secs_to_utc_ymd(current_unix_secs());
    let backup_name = format!("{SONG_LIBRARY_BACKUP_PREFIX}{year:04}{month:02}{day:02}.json");
    let backup_path = path.with_file_name(&backup_name);
    if backup_path.exists() {
        return;
    }
    if let Err(err) = fs::copy(path, &backup_path) {
        eprintln!("[song-library] daily backup failed: {err}");
        return;
    }
    prune_song_library_backups(path);
}

fn prune_song_library_backups(path: &Path) {
    let Some(dir) = path.parent() else { return };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(SONG_LIBRARY_BACKUP_PREFIX) && name.ends_with(".json")
                })
        })
        .collect();
    if backups.len() <= MAX_SONG_LIBRARY_BACKUPS {
        return;
    }
    // 檔名內含 YYYYMMDD，字典序即時間序；移除最舊的多餘備份
    backups.sort();
    let excess = backups.len() - MAX_SONG_LIBRARY_BACKUPS;
    for stale in backups.into_iter().take(excess) {
        if let Err(err) = fs::remove_file(&stale) {
            eprintln!("[song-library] backup prune failed: {err}");
        }
    }
}

fn serialize_song_library_records(songs: &[SongProfileRecord]) -> Result<String, AppError> {
    let root = serde_json::json!({
        "version": 1,
        "songs": songs,
    });
    serde_json::to_string_pretty(&root)
        .map_err(|e| AppError::Settings(format!("Could not serialize song library: {e}")))
}

fn quarantine_song_library(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    let suffix = current_unix_secs();
    let quarantine = path.with_file_name(format!("song-library.invalid-{suffix}.json"));
    match fs::rename(path, quarantine) {
        Ok(()) => Ok(()),
        Err(_) => fs::remove_file(path).map_err(AppError::Io),
    }
}

fn parse_song_library_records(
    library_json: &str,
    path_mode: SessionPathMode,
) -> Result<Vec<SongProfileRecord>, AppError> {
    let value = serde_json::from_str::<Value>(library_json)
        .map_err(|e| AppError::Settings(format!("Invalid song library JSON: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::Settings("Song library must be a JSON object".into()))?;
    let songs = obj
        .get("songs")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Settings("Song library songs must be an array".into()))?;
    if songs.len() > MAX_SONG_PROFILES {
        return Err(AppError::Settings(format!(
            "Song library can store up to {MAX_SONG_PROFILES} profiles"
        )));
    }

    Ok(songs
        .iter()
        .filter_map(|song| sanitize_song_profile_record(song, path_mode).ok())
        .collect())
}

fn sanitize_song_profile_record(
    value: &Value,
    path_mode: SessionPathMode,
) -> Result<SongProfileRecord, AppError> {
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::Settings("Song profile must be a JSON object".into()))?;
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .and_then(sanitize_song_id)
        .ok_or_else(|| AppError::Settings("Song profile id is invalid".into()))?;
    let title = obj
        .get("title")
        .and_then(Value::as_str)
        .and_then(|value| sanitize_text_input(value, MAX_SONG_TITLE_CHARS))
        .ok_or_else(|| AppError::Settings("Song profile title is required".into()))?;
    let artist = obj
        .get("artist")
        .and_then(Value::as_str)
        .and_then(|value| sanitize_text_input(value, MAX_SONG_ARTIST_CHARS));
    let session = sanitize_song_session_value_for_mode(
        obj.get("session").unwrap_or(&Value::Null),
        path_mode,
    )?;

    Ok(SongProfileRecord {
        id,
        title,
        artist,
        created_at_unix: unsigned_integer(obj.get("created_at_unix")).unwrap_or(0),
        updated_at_unix: unsigned_integer(obj.get("updated_at_unix")).unwrap_or(0),
        last_opened_at_unix: unsigned_integer(obj.get("last_opened_at_unix")).unwrap_or(0),
        session,
    })
}

fn sanitize_song_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_SONG_ID_CHARS {
        return None;
    }
    let id: String = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    (id == trimmed).then_some(id)
}

fn sanitize_text_input(value: &str, max_chars: usize) -> Option<String> {
    if value.chars().any(|ch| ch == '\0') {
        return None;
    }
    let mut text = String::new();
    for ch in value.trim().chars().filter(|ch| !ch.is_control()) {
        if text.chars().count() >= max_chars {
            break;
        }
        text.push(ch);
    }
    (!text.is_empty()).then_some(text)
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn generate_song_profile_id(songs: &[SongProfileRecord], now: u64) -> String {
    for suffix in 0..10_000 {
        let id = if suffix == 0 {
            format!("song-{now}")
        } else {
            format!("song-{now}-{suffix}")
        };
        if songs.iter().all(|song| song.id != id) {
            return id;
        }
    }
    format!("song-{now}-fallback")
}

fn song_profile_summary(song: &SongProfileRecord) -> SongProfileSummary {
    SongProfileSummary {
        id: song.id.clone(),
        title: song.title.clone(),
        artist: song.artist.clone(),
        created_at_unix: song.created_at_unix,
        updated_at_unix: song.updated_at_unix,
        last_opened_at_unix: song.last_opened_at_unix,
        status: song_profile_status(&song.session),
    }
}

fn song_profile_status(session: &Value) -> SongProfileStatus {
    let obj = session.as_object();
    let backing_path = obj
        .and_then(|obj| obj.get("backingPath"))
        .and_then(Value::as_str);
    let guide_vocal_path = obj
        .and_then(|obj| obj.get("guideVocalPath"))
        .and_then(Value::as_str);
    let lyrics = obj
        .and_then(|obj| obj.get("lyricsLines"))
        .and_then(Value::as_array);
    let lyrics_count = lyrics.map(|lines| lines.len()).unwrap_or(0);
    let timed_lyrics_count = lyrics
        .map(|lines| {
            lines
                .iter()
                .filter(|line| {
                    let obj = line.as_object();
                    let start = obj
                        .and_then(|obj| obj.get("start_ms"))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    let end = obj
                        .and_then(|obj| obj.get("end_ms"))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    end > start
                })
                .count()
        })
        .unwrap_or(0);

    SongProfileStatus {
        backing_set: backing_path.is_some_and(|path| !path.trim().is_empty()),
        backing_exists: backing_path.is_some_and(stored_path_exists),
        guide_vocal_set: guide_vocal_path.is_some_and(|path| !path.trim().is_empty()),
        guide_vocal_exists: guide_vocal_path.is_some_and(stored_path_exists),
        lyrics_count,
        timed_lyrics_count,
        melody_present: obj
            .and_then(|obj| obj.get("melody"))
            .is_some_and(Value::is_object),
    }
}

fn stored_path_exists(path: &str) -> bool {
    let Some(resolved) = portable_paths::resolve_stored_path_text(path) else {
        return false;
    };
    let resolved_text = portable_paths::display_path(&resolved);
    if security::validate_local_path_safe(&resolved_text).is_err() {
        return false;
    }
    resolved.is_file()
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
        "guideVocalEnabled".into(),
        Value::Bool(
            obj.get("guideVocalEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
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

fn parse_and_sanitize_song_session(
    session_json: &str,
    path_mode: SessionPathMode,
) -> Result<Value, AppError> {
    let value = serde_json::from_str::<Value>(session_json)
        .map_err(|e| AppError::Settings(format!("Invalid song session JSON: {e}")))?;
    sanitize_song_session_value_for_mode(&value, path_mode)
}

fn sanitize_song_session_value_for_mode(
    value: &Value,
    path_mode: SessionPathMode,
) -> Result<Value, AppError> {
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::Settings("Song session must be a JSON object".into()))?;
    let project = sanitize_project_session_value_for_mode(value, path_mode)?;
    let mut out = project
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Settings("Song session project data is invalid".into()))?;

    if let Some(mixer) = sanitize_song_mixer(obj.get("mixer")) {
        out.insert("mixer".into(), mixer);
    }
    if let Some(practice) = sanitize_song_practice(obj.get("practice")) {
        out.insert("practice".into(), practice);
    }
    Ok(Value::Object(out))
}

fn sanitize_song_mixer(value: Option<&Value>) -> Option<Value> {
    let obj = value?.as_object()?;
    let mut out = Map::new();
    out.insert(
        "backingVolume".into(),
        finite_number_value(obj.get("backingVolume"), 0.0, 1.0)?,
    );
    out.insert(
        "micGain".into(),
        finite_number_value(obj.get("micGain"), 0.0, 3.0)?,
    );
    out.insert(
        "guideVolume".into(),
        finite_number_value(obj.get("guideVolume"), 0.0, 1.0)?,
    );
    out.insert(
        "autoBalanceMixin".into(),
        Value::Bool(
            obj.get("autoBalanceMixin")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ),
    );
    out.insert(
        "autoBalanceVocalPreset".into(),
        Value::String(normalize_auto_balance_vocal_preset(
            obj.get("autoBalanceVocalPreset")
                .and_then(Value::as_str)
                .unwrap_or("forward"),
        )),
    );
    out.insert(
        "exportNamingMode".into(),
        Value::String(normalize_export_naming_mode(
            obj.get("exportNamingMode")
                .and_then(Value::as_str)
                .unwrap_or("auto"),
        )),
    );
    Some(Value::Object(out))
}

fn sanitize_song_practice(value: Option<&Value>) -> Option<Value> {
    let obj = value?.as_object()?;
    let mut out = Map::new();
    if let Some(loop_a) = finite_number_value(obj.get("loopA"), 0.0, MAX_DURATION_SECS) {
        out.insert("loopA".into(), loop_a);
    }
    if let Some(loop_b) = finite_number_value(obj.get("loopB"), 0.0, MAX_DURATION_SECS) {
        out.insert("loopB".into(), loop_b);
    }
    out.insert(
        "speed".into(),
        finite_number_value(obj.get("speed"), 0.25, 4.0)
            .unwrap_or_else(|| Value::Number(Number::from(1))),
    );
    out.insert(
        "pitchSemitones".into(),
        finite_number_value(obj.get("pitchSemitones"), -7.0, 7.0)
            .unwrap_or_else(|| Value::Number(Number::from(0))),
    );
    Some(Value::Object(out))
}

fn sanitize_restore_path_value_for_mode(
    value: Option<&Value>,
    allowed_extensions: &[&str],
    path_mode: SessionPathMode,
) -> Value {
    match path_mode {
        SessionPathMode::Runtime => value
            .and_then(Value::as_str)
            .and_then(|path| sanitize_restore_path(path, allowed_extensions, path_mode).ok())
            .map(Value::String)
            .unwrap_or(Value::Null),
        SessionPathMode::Persisted => {
            sanitize_local_path_string(value, allowed_extensions, path_mode)
                .map(Value::String)
                .unwrap_or(Value::Null)
        }
    }
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

fn unsigned_integer(value: Option<&Value>) -> Option<u64> {
    value?.as_u64()
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
            "guideVocalEnabled": true,
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
        assert_eq!(sanitized["guideVocalEnabled"].as_bool(), Some(true));
        assert!(sanitized["alignmentResult"].is_object());
        assert_eq!(
            sanitized["alignmentResult"]["peak_correlation"].as_f64(),
            Some(123456789.0)
        );
    }

    #[test]
    fn song_session_sanitizer_preserves_mixer_and_practice_settings() {
        let sanitized = sanitize_song_session_value_for_mode(
            &serde_json::json!({
                "version": 1,
                "guideVocalEnabled": true,
                "mixer": {
                    "backingVolume": 0.25,
                    "micGain": 1.75,
                    "guideVolume": 0.4,
                    "autoBalanceMixin": true,
                    "autoBalanceVocalPreset": "clear",
                    "exportNamingMode": "auto"
                },
                "practice": {
                    "loopA": 12.5,
                    "loopB": 18.0,
                    "speed": 0.9,
                    "pitchSemitones": -2
                }
            }),
            SessionPathMode::Persisted,
        )
        .unwrap();

        assert_eq!(sanitized["guideVocalEnabled"].as_bool(), Some(true));
        assert_eq!(sanitized["mixer"]["backingVolume"].as_f64(), Some(0.25));
        assert_eq!(
            sanitized["mixer"]["autoBalanceVocalPreset"].as_str(),
            Some("clear")
        );
        assert_eq!(sanitized["practice"]["loopA"].as_f64(), Some(12.5));
        assert_eq!(sanitized["practice"]["pitchSemitones"].as_f64(), Some(-2.0));
    }

    #[test]
    fn song_library_parser_skips_invalid_records() {
        let library = serde_json::json!({
            "version": 1,
            "songs": [
                {
                    "id": "song-1",
                    "title": "Song A",
                    "artist": "Singer",
                    "created_at_unix": 1,
                    "updated_at_unix": 2,
                    "last_opened_at_unix": 3,
                    "session": { "version": 1, "lyricsLines": [] }
                },
                {
                    "id": "../bad",
                    "title": "Bad",
                    "session": { "version": 1 }
                }
            ]
        })
        .to_string();

        let records = parse_song_library_records(&library, SessionPathMode::Persisted).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "song-1");
        assert_eq!(records[0].artist.as_deref(), Some("Singer"));
    }

    #[test]
    fn song_library_parser_rejects_over_profile_limit() {
        let songs = (0..=MAX_SONG_PROFILES)
            .map(|index| {
                serde_json::json!({
                    "id": format!("song-{index}"),
                    "title": format!("Song {index}"),
                    "created_at_unix": 1,
                    "updated_at_unix": 2,
                    "last_opened_at_unix": 3,
                    "session": { "version": 1, "lyricsLines": [] }
                })
            })
            .collect::<Vec<_>>();
        let library = serde_json::json!({
            "version": 1,
            "songs": songs
        })
        .to_string();

        assert!(parse_song_library_records(&library, SessionPathMode::Persisted).is_err());
    }

    #[test]
    fn trim_latency_profiles_keeps_latest_profiles() {
        let mut profiles = (0..(MAX_LATENCY_CALIBRATION_PROFILES + 10))
            .map(|index| LatencyCalibrationProfile {
                key: format!("profile-{index}"),
                input_device: format!("input-{index}"),
                output_device: format!("output-{index}"),
                sample_rate: 44_100,
                latency_ms: index as f64,
                confidence: "manual".to_string(),
                updated_at_unix: index as u64,
            })
            .collect::<Vec<_>>();

        trim_latency_profiles(&mut profiles);

        assert_eq!(profiles.len(), MAX_LATENCY_CALIBRATION_PROFILES);
        assert_eq!(profiles[0].updated_at_unix, 109);
        assert_eq!(profiles.last().unwrap().updated_at_unix, 10);
    }
}
