//! Local whisper.cpp runner and model management.

use crate::core::{folder_open, lyrics_forced_aligner, portable_paths};
use crate::{error::AppError, security};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const WHISPER_MANIFEST_NAME: &str = "whisper-manifest.json";
const WHISPER_DIR_NAME: &str = "whisper";
const WHISPER_RUNNER_EVENT: &str = "whisper:runner_install_progress";
const WHISPER_MODEL_EVENT: &str = "whisper:model_install_progress";
const WHISPER_TRANSCRIPTION_EVENT: &str = "whisper:transcription_progress";

#[cfg(windows)]
const WHISPER_RUNNER_VERSION: &str = "v1.8.4";
#[cfg(windows)]
const WHISPER_RUNNER_DOWNLOAD_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.4/whisper-bin-x64.zip";
#[cfg(windows)]
const WHISPER_RUNNER_ZIP_SHA256: &str =
    "74f973345cb52ef5ba3ec9e7e7af8e48cc8c71722d1528603b80588a11f82e3e";
#[cfg(windows)]
const WHISPER_RUNNER_EXE_SHA256: &str =
    "d4c598cf97de103f888d1a53b8abddc85bf27ab752f785ca69318cedc8a2cf64";

#[cfg(windows)]
const MAX_WHISPER_RUNNER_DOWNLOAD_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WHISPER_EXTRACTED_BYTES: u64 = 128 * 1024 * 1024;

static WHISPER_STATE_LOCK: Mutex<()> = Mutex::new(());
static WHISPER_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WhisperManifest {
    runner_path: Option<String>,
    runner_sha256: Option<String>,
    model_path: Option<String>,
    model_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalWhisperRunnerCandidate {
    pub runner_path: String,
    pub runner_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalWhisperModelCandidate {
    pub model_path: String,
    pub model_sha256: String,
    pub model_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperToolsStatus {
    pub runner_available: bool,
    pub runner_path: Option<String>,
    pub model_available: bool,
    pub model_path: Option<String>,
    pub model_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperModelOption {
    pub id: String,
    pub label: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperInstallProgress {
    pub percent: f64,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperTranscriptionResult {
    pub transcript_path: String,
    pub subtitle_path: Option<String>,
    pub segment_count: usize,
    pub model_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperTranscriptionProgress {
    pub percent: f64,
    pub status: String,
    pub message: String,
    pub elapsed_seconds: u64,
    pub eta_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct WhisperModelSpec {
    id: &'static str,
    label: &'static str,
    file_name: &'static str,
    url: &'static str,
    sha256: &'static str,
    size_bytes: u64,
    max_bytes: u64,
}

const WHISPER_MODEL_SPECS: &[WhisperModelSpec] = &[
    WhisperModelSpec {
        id: "large-v3-turbo-q5_0",
        label: "Recommended official large-v3-turbo Q5_0 - 547 MB",
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        size_bytes: 574_041_195,
        max_bytes: 700 * 1024 * 1024,
    },
    WhisperModelSpec {
        id: "large-v3-turbo-q8_0",
        label: "Advanced official large-v3-turbo Q8_0 - 834 MB",
        file_name: "ggml-large-v3-turbo-q8_0.bin",
        url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin",
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        size_bytes: 874_188_075,
        max_bytes: 1_000 * 1024 * 1024,
    },
];

fn acquire_whisper_state_lock() -> Result<MutexGuard<'static, ()>, AppError> {
    WHISPER_STATE_LOCK
        .try_lock()
        .map_err(|_| AppError::Internal("Whisper is already updating settings".into()))
}

fn portable_whisper_data_dir() -> Option<PathBuf> {
    portable_paths::ensure_dir(WHISPER_DIR_NAME).ok()
}

fn whisper_data_dir() -> Option<PathBuf> {
    portable_whisper_data_dir()
}

fn whisper_runner_dir() -> Option<PathBuf> {
    whisper_data_dir().map(|dir| dir.join("runner"))
}

fn portable_whisper_models_dir() -> Option<PathBuf> {
    portable_paths::ensure_dir(Path::new("models").join("whisper")).ok()
}

fn whisper_models_dir() -> Option<PathBuf> {
    portable_whisper_models_dir()
}

fn whisper_model_search_dirs() -> Vec<PathBuf> {
    whisper_models_dir().into_iter().collect()
}

fn path_is_inside_dir(path: &Path, dir: &Path) -> bool {
    match (fs::canonicalize(path), fs::canonicalize(dir)) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => false,
    }
}

fn is_portable_runner_path(path: &Path) -> bool {
    whisper_runner_dir()
        .as_deref()
        .is_some_and(|dir| path_is_inside_dir(path, dir))
}

fn is_portable_model_path(path: &Path) -> bool {
    whisper_models_dir()
        .as_deref()
        .is_some_and(|dir| path_is_inside_dir(path, dir))
}

fn whisper_cache_dir() -> Option<PathBuf> {
    portable_paths::ensure_dir(Path::new(WHISPER_DIR_NAME).join("cache")).ok()
}

fn is_owned_whisper_cache_name(name: &str) -> bool {
    name.starts_with("vocalsync-whisper-") || name.starts_with("vocalsync-whisper-input-")
}

fn is_owned_whisper_cache_path(path: &Path) -> bool {
    let Some(cache_dir) = whisper_cache_dir() else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return false;
    };
    let same_parent = match (fs::canonicalize(parent), fs::canonicalize(&cache_dir)) {
        (Ok(parent), Ok(cache_dir)) => parent == cache_dir,
        _ => parent == cache_dir.as_path(),
    };
    same_parent
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .map(is_owned_whisper_cache_name)
            .unwrap_or(false)
}

fn remove_file_if_exists(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::Io(err)),
    }
}

pub fn cleanup_whisper_cache() -> Result<usize, AppError> {
    let Some(cache_dir) = whisper_cache_dir() else {
        return Ok(0);
    };
    let mut removed = 0usize;
    for entry in fs::read_dir(&cache_dir).map_err(AppError::Io)? {
        let entry = entry.map_err(AppError::Io)?;
        let path = entry.path();
        if !path.is_file() || !is_owned_whisper_cache_path(&path) {
            continue;
        }
        remove_file_if_exists(&path)?;
        removed += 1;
    }
    Ok(removed)
}

pub fn cleanup_generated_transcript_artifacts(transcript_path: &Path) -> Result<usize, AppError> {
    if !is_owned_whisper_cache_path(transcript_path) {
        return Ok(0);
    }
    let mut removed = 0usize;
    for path in [
        transcript_path.with_extension("json"),
        transcript_path.with_extension("srt"),
    ] {
        if is_owned_whisper_cache_path(&path) && path.exists() {
            remove_file_if_exists(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn whisper_manifest_path() -> Option<PathBuf> {
    whisper_data_dir().map(|dir| dir.join(WHISPER_MANIFEST_NAME))
}

fn whisper_manifest_search_paths() -> Vec<PathBuf> {
    whisper_manifest_path().into_iter().collect()
}

fn load_whisper_manifest() -> Option<WhisperManifest> {
    for path in whisper_manifest_search_paths() {
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        match serde_json::from_str::<WhisperManifest>(&content) {
            Ok(manifest) => return Some(manifest),
            Err(err) => {
                log::warn!(
                    "[whisper] failed to parse manifest at {}: {}",
                    path.display(),
                    err
                );
            }
        }
    }
    None
}

fn save_whisper_manifest(manifest: &WhisperManifest) -> Result<(), AppError> {
    let path = whisper_manifest_path()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper settings directory".into()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AppError::Io)?;
    }

    let content = serde_json::to_string_pretty(manifest).map_err(|err| {
        AppError::Internal(format!("Failed to serialize Whisper settings: {}", err))
    })?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).map_err(AppError::Io)?;

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path).map_err(AppError::Io)?;
    }

    fs::rename(&tmp_path, &path).map_err(AppError::Io)?;
    Ok(())
}

struct TempFileGuard {
    path: PathBuf,
    disarmed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.disarmed && self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct TempDirGuard {
    path: PathBuf,
    disarmed: bool,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.disarmed && self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn emit_install_progress(
    app: &AppHandle,
    event: &str,
    percent: f64,
    status: &str,
    message: impl Into<String>,
) {
    let _ = app.emit(
        event,
        &WhisperInstallProgress {
            percent,
            status: status.into(),
            message: message.into(),
        },
    );
}

fn emit_transcription_progress(
    app: &AppHandle,
    percent: f64,
    status: &str,
    message: impl Into<String>,
    elapsed_seconds: u64,
    eta_seconds: Option<u64>,
) {
    let _ = app.emit(
        WHISPER_TRANSCRIPTION_EVENT,
        &WhisperTranscriptionProgress {
            percent: percent.clamp(0.0, 100.0),
            status: status.into(),
            message: message.into(),
            elapsed_seconds,
            eta_seconds,
        },
    );
}

fn transcription_eta_seconds(percent: f64, elapsed_seconds: u64) -> Option<u64> {
    if !(1.0..100.0).contains(&percent) || elapsed_seconds == 0 {
        return None;
    }
    let total = elapsed_seconds as f64 * 100.0 / percent;
    Some((total - elapsed_seconds as f64).max(0.0).ceil() as u64)
}

fn parse_whisper_progress_percent(text: &str) -> Option<f64> {
    let mut parsed = None;
    for (percent_index, _) in text.match_indices('%') {
        let before_percent = &text[..percent_index];
        let number_reversed: String = before_percent
            .chars()
            .rev()
            .skip_while(|ch| ch.is_whitespace())
            .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
            .collect();
        let number: String = number_reversed.chars().rev().collect();
        if let Ok(value) = number.parse::<f64>() {
            parsed = Some(value.clamp(0.0, 100.0));
        }
    }
    parsed
}

fn spawn_whisper_output_reader<R>(
    mut reader: R,
    app: AppHandle,
    output: Arc<Mutex<String>>,
    started_at: Instant,
    last_reported_percent: Arc<Mutex<f64>>,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0u8; 2048];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };
            let chunk = String::from_utf8_lossy(&buffer[..read]).to_string();
            if let Ok(mut text) = output.lock() {
                text.push_str(&chunk);
            }
            if let Some(percent) = parse_whisper_progress_percent(&chunk) {
                if let Ok(mut last_percent) = last_reported_percent.lock() {
                    if percent >= 99.0 || percent - *last_percent >= 1.0 {
                        *last_percent = percent;
                        let elapsed_seconds = started_at.elapsed().as_secs();
                        emit_transcription_progress(
                            &app,
                            percent,
                            "transcribing",
                            "Whisper is transcribing vocals.wav",
                            elapsed_seconds,
                            transcription_eta_seconds(percent, elapsed_seconds),
                        );
                    }
                }
            }
        }
    })
}

fn ensure_download_within_limit(
    label: &str,
    downloaded: u64,
    max_bytes: u64,
) -> Result<(), AppError> {
    if downloaded > max_bytes {
        return Err(AppError::Internal(format!(
            "{} download exceeded the {:.0} MB safety limit",
            label,
            max_bytes as f64 / 1_048_576.0
        )));
    }
    Ok(())
}

fn download_to_file(
    app: &AppHandle,
    event: &str,
    label: &str,
    url: &str,
    target_path: &Path,
    max_bytes: u64,
    progress_start: f64,
    progress_end: f64,
) -> Result<u64, AppError> {
    let resp = ureq::get(url)
        .call()
        .map_err(|err| AppError::Internal(format!("Failed to download {}: {}", label, err)))?;

    let content_length = resp
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if content_length > 0 {
        ensure_download_within_limit(label, content_length, max_bytes)?;
    }

    let mut reader = resp.into_reader();
    let mut file = fs::File::create(target_path).map_err(AppError::Io)?;
    let mut buf = [0u8; 65536];
    let mut downloaded = 0u64;
    let mut last_reported_pct = progress_start;
    let progress_span = (progress_end - progress_start).max(1.0);

    loop {
        let n = reader.read(&mut buf).map_err(|err| {
            AppError::Internal(format!("Failed to read {} download: {}", label, err))
        })?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(AppError::Io)?;
        downloaded += n as u64;
        ensure_download_within_limit(label, downloaded, max_bytes)?;

        if content_length > 0 {
            let pct = progress_start + (downloaded as f64 / content_length as f64) * progress_span;
            if pct - last_reported_pct >= 3.0 {
                last_reported_pct = pct;
                emit_install_progress(
                    app,
                    event,
                    pct.min(progress_end),
                    "downloading",
                    format!(
                        "{} downloading... {:.1} MB / {:.1} MB",
                        label,
                        downloaded as f64 / 1_048_576.0,
                        content_length as f64 / 1_048_576.0
                    ),
                );
            }
        }
    }

    Ok(downloaded)
}

fn runner_allowed_file_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["whisper-cli.exe"]
    } else {
        &["whisper-cli"]
    }
}

fn model_extension_allowed(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bin") || ext.eq_ignore_ascii_case("gguf"))
}

fn file_name_matches(path: &Path, allowed_names: &[&str]) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            allowed_names
                .iter()
                .any(|allowed| name.eq_ignore_ascii_case(allowed))
        })
}

fn canonical_existing_file(path: &str, label: &str) -> Result<PathBuf, AppError> {
    security::validate_path_safe(path)?;
    let raw_path = Path::new(path);
    let canonical = raw_path.canonicalize().map_err(AppError::Io)?;
    if !canonical.is_file() {
        return Err(AppError::Audio(format!(
            "{} must point to an existing file",
            label
        )));
    }
    Ok(canonical)
}

#[cfg(windows)]
fn command_arg_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(not(windows))]
fn command_arg_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn compute_sha256(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path).map_err(AppError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(AppError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_sha256(path: &Path, expected_hash: &str) -> Result<(), AppError> {
    let actual = compute_sha256(path)?;
    if actual != expected_hash {
        return Err(AppError::Internal(format!(
            "Whisper file changed after it was trusted. Expected SHA-256 {}, got {}",
            expected_hash, actual
        )));
    }
    Ok(())
}

fn verify_file_size(path: &Path, expected_size: u64) -> Result<(), AppError> {
    let actual = fs::metadata(path).map_err(AppError::Io)?.len();
    if actual != expected_size {
        return Err(AppError::Internal(format!(
            "Whisper file size changed after it was trusted. Expected {} bytes, got {} bytes",
            expected_size, actual
        )));
    }
    Ok(())
}

fn runner_candidate_from_path(path: PathBuf) -> Result<LocalWhisperRunnerCandidate, AppError> {
    if !file_name_matches(&path, runner_allowed_file_names()) {
        return Err(AppError::Audio(
            "Whisper runner must be whisper-cli.exe".into(),
        ));
    }

    Ok(LocalWhisperRunnerCandidate {
        runner_sha256: compute_sha256(&path)?,
        runner_path: path.to_string_lossy().to_string(),
    })
}

fn model_candidate_from_path(path: PathBuf) -> Result<LocalWhisperModelCandidate, AppError> {
    if !model_extension_allowed(&path) {
        return Err(AppError::Audio(
            "Whisper model must be a .bin or .gguf file".into(),
        ));
    }
    let metadata = fs::metadata(&path).map_err(AppError::Io)?;
    if metadata.len() == 0 {
        return Err(AppError::Audio("Whisper model file is empty".into()));
    }

    Ok(LocalWhisperModelCandidate {
        model_sha256: compute_sha256(&path)?,
        model_size_bytes: metadata.len(),
        model_path: path.to_string_lossy().to_string(),
    })
}

fn runner_candidate_from_trust_request(
    candidate: LocalWhisperRunnerCandidate,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    let path = canonical_existing_file(&candidate.runner_path, "Whisper runner")?;
    let refreshed = runner_candidate_from_path(path)?;
    if refreshed.runner_sha256 != candidate.runner_sha256 {
        return Err(AppError::Audio(
            "Whisper runner changed after selection. Please select it again.".into(),
        ));
    }
    Ok(refreshed)
}

fn copy_file_verified(
    source: &Path,
    target: &Path,
    expected_hash: &str,
) -> Result<PathBuf, AppError> {
    let source = source.canonicalize().map_err(AppError::Io)?;
    if let Ok(target_canonical) = target.canonicalize() {
        if target_canonical == source && verify_sha256(&target_canonical, expected_hash).is_ok() {
            return Ok(target_canonical);
        }
        if verify_sha256(&target_canonical, expected_hash).is_ok() {
            return Ok(target_canonical);
        }
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(AppError::Io)?;
    }
    let tmp_path = target.with_extension(format!("tmp-{}", whisper_unique_suffix()));
    let _ = remove_file_if_exists(&tmp_path);
    fs::copy(&source, &tmp_path).map_err(AppError::Io)?;
    verify_sha256(&tmp_path, expected_hash)?;

    #[cfg(windows)]
    if target.exists() {
        fs::remove_file(target).map_err(AppError::Io)?;
    }
    fs::rename(&tmp_path, target).map_err(AppError::Io)?;
    target.canonicalize().map_err(AppError::Io)
}

fn import_runner_candidate_to_portable(
    candidate: LocalWhisperRunnerCandidate,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    let source = PathBuf::from(&candidate.runner_path);
    if is_portable_runner_path(&source) {
        return Ok(candidate);
    }

    let runner_root = whisper_runner_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper runner directory".into()))?;
    let import_dir = runner_root.join("manual");
    fs::create_dir_all(&import_dir).map_err(AppError::Io)?;

    if let Some(parent) = source.parent() {
        for entry in fs::read_dir(parent).map_err(AppError::Io)? {
            let entry = entry.map_err(AppError::Io)?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".dll") {
                fs::copy(&path, import_dir.join(name)).map_err(AppError::Io)?;
            }
        }
    }

    let runner_name = runner_allowed_file_names()
        .first()
        .copied()
        .ok_or_else(|| AppError::Internal("No Whisper runner file name is allowed".into()))?;
    let imported_path = copy_file_verified(
        &source,
        &import_dir.join(runner_name),
        &candidate.runner_sha256,
    )?;
    let imported = runner_candidate_from_path(imported_path)?;
    if imported.runner_sha256 != candidate.runner_sha256 {
        return Err(AppError::Audio(
            "Whisper runner changed during portable import".into(),
        ));
    }
    Ok(imported)
}

fn model_candidate_from_trust_request(
    candidate: LocalWhisperModelCandidate,
) -> Result<LocalWhisperModelCandidate, AppError> {
    let path = canonical_existing_file(&candidate.model_path, "Whisper model")?;
    let refreshed = model_candidate_from_path(path)?;
    if refreshed.model_sha256 != candidate.model_sha256
        || refreshed.model_size_bytes != candidate.model_size_bytes
    {
        return Err(AppError::Audio(
            "Whisper model changed after selection. Please select it again.".into(),
        ));
    }
    Ok(refreshed)
}

fn portable_model_import_target(
    source: &Path,
    sha256: &str,
    size_bytes: u64,
) -> Result<PathBuf, AppError> {
    let models_dir = whisper_models_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper model directory".into()))?;
    fs::create_dir_all(&models_dir).map_err(AppError::Io)?;

    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Audio("Whisper model file name is invalid".into()))?;
    security::validate_filename_prefix(file_name)?;

    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("whisper-model");
    let ext = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("bin");
    let suffix = sha256.chars().take(12).collect::<String>();

    for attempt in 0..100 {
        let candidate_name = if attempt == 0 {
            file_name.to_string()
        } else if attempt == 1 {
            format!("{}-{}.{}", stem, suffix, ext)
        } else {
            format!("{}-{}-{}.{}", stem, suffix, attempt, ext)
        };
        let target = models_dir.join(candidate_name);
        if !target.exists() {
            return Ok(target);
        }
        if target.is_file()
            && fs::metadata(&target)
                .map(|metadata| metadata.len() == size_bytes)
                .unwrap_or(false)
            && verify_sha256(&target, sha256).is_ok()
        {
            return Ok(target);
        }
    }

    Err(AppError::Audio(
        "Could not choose a portable Whisper model file name".into(),
    ))
}

fn import_model_candidate_to_portable(
    candidate: LocalWhisperModelCandidate,
) -> Result<LocalWhisperModelCandidate, AppError> {
    let source = PathBuf::from(&candidate.model_path);
    if is_portable_model_path(&source) {
        return Ok(candidate);
    }

    let target =
        portable_model_import_target(&source, &candidate.model_sha256, candidate.model_size_bytes)?;
    if !target.is_file() {
        let tmp_path = target.with_file_name(format!(
            "{}.tmp-{}",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("whisper-model"),
            whisper_unique_suffix()
        ));
        let _ = remove_file_if_exists(&tmp_path);
        let copied = fs::copy(&source, &tmp_path).map_err(AppError::Io)?;
        if copied != candidate.model_size_bytes {
            let _ = remove_file_if_exists(&tmp_path);
            return Err(AppError::Audio(format!(
                "Whisper model copy size mismatch. Expected {} bytes, copied {} bytes",
                candidate.model_size_bytes, copied
            )));
        }
        if let Err(err) = verify_sha256(&tmp_path, &candidate.model_sha256) {
            let _ = remove_file_if_exists(&tmp_path);
            return Err(err);
        }
        fs::rename(&tmp_path, &target).map_err(AppError::Io)?;
    }

    let imported = model_candidate_from_path(target.canonicalize().map_err(AppError::Io)?)?;
    if imported.model_sha256 != candidate.model_sha256
        || imported.model_size_bytes != candidate.model_size_bytes
    {
        return Err(AppError::Audio(
            "Portable Whisper model changed during import".into(),
        ));
    }
    Ok(imported)
}

fn trusted_runner_path_from_manifest(manifest: &WhisperManifest) -> Option<PathBuf> {
    let path = PathBuf::from(manifest.runner_path.as_deref()?);
    let expected_hash = manifest.runner_sha256.as_deref()?;
    let canonical = path.canonicalize().ok()?;
    if !is_portable_runner_path(&canonical) {
        return None;
    }
    verify_sha256(&canonical, expected_hash).ok()?;
    if !file_name_matches(&canonical, runner_allowed_file_names()) {
        return None;
    }
    Some(canonical)
}

#[cfg(windows)]
fn trusted_bundled_runner_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let runner = exe_dir.join("whisper").join("whisper-cli.exe");
    if !runner.is_file() {
        return None;
    }
    verify_sha256(&runner, WHISPER_RUNNER_EXE_SHA256).ok()?;
    Some(runner)
}

#[cfg(not(windows))]
fn trusted_bundled_runner_path() -> Option<PathBuf> {
    None
}

fn trusted_model_path_from_manifest(manifest: &WhisperManifest) -> Option<PathBuf> {
    let path = PathBuf::from(manifest.model_path.as_deref()?);
    let expected_hash = manifest.model_sha256.as_deref()?;
    let canonical = path.canonicalize().ok()?;
    if !is_portable_model_path(&canonical) {
        return None;
    }
    verify_sha256(&canonical, expected_hash).ok()?;
    if !model_extension_allowed(&canonical) {
        return None;
    }
    Some(canonical)
}

pub fn find_whisper_runner() -> Option<PathBuf> {
    trusted_bundled_runner_path().or_else(|| {
        load_whisper_manifest()
            .as_ref()
            .and_then(trusted_runner_path_from_manifest)
    })
}

pub fn find_whisper_model() -> Option<PathBuf> {
    load_whisper_manifest()
        .as_ref()
        .and_then(trusted_model_path_from_manifest)
}

pub fn check_whisper_tools() -> WhisperToolsStatus {
    let manifest = load_whisper_manifest().unwrap_or_default();
    let runner_path =
        trusted_bundled_runner_path().or_else(|| trusted_runner_path_from_manifest(&manifest));
    let model_path = trusted_model_path_from_manifest(&manifest);
    let model_size_bytes = model_path
        .as_deref()
        .and_then(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len());

    WhisperToolsStatus {
        runner_available: runner_path.is_some(),
        runner_path: runner_path.map(|path| path.to_string_lossy().to_string()),
        model_available: model_path.is_some(),
        model_path: model_path.map(|path| path.to_string_lossy().to_string()),
        model_size_bytes,
    }
}

pub fn inspect_local_whisper_runner_path(
    path: String,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    runner_candidate_from_path(canonical_existing_file(&path, "Whisper runner")?)
}

pub fn trust_local_whisper_runner_candidate(
    candidate: LocalWhisperRunnerCandidate,
) -> Result<LocalWhisperRunnerCandidate, AppError> {
    let _guard = acquire_whisper_state_lock()?;
    let trusted =
        import_runner_candidate_to_portable(runner_candidate_from_trust_request(candidate)?)?;
    let mut manifest = load_whisper_manifest().unwrap_or_default();
    manifest.runner_path = Some(trusted.runner_path.clone());
    manifest.runner_sha256 = Some(trusted.runner_sha256.clone());
    save_whisper_manifest(&manifest)?;
    Ok(trusted)
}

pub fn inspect_local_whisper_model_path(
    path: String,
) -> Result<LocalWhisperModelCandidate, AppError> {
    model_candidate_from_path(canonical_existing_file(&path, "Whisper model")?)
}

pub fn trust_local_whisper_model_candidate(
    candidate: LocalWhisperModelCandidate,
) -> Result<LocalWhisperModelCandidate, AppError> {
    let _guard = acquire_whisper_state_lock()?;
    let trusted =
        import_model_candidate_to_portable(model_candidate_from_trust_request(candidate)?)?;
    let mut manifest = load_whisper_manifest().unwrap_or_default();
    manifest.model_path = Some(trusted.model_path.clone());
    manifest.model_sha256 = Some(trusted.model_sha256.clone());
    save_whisper_manifest(&manifest)?;
    Ok(trusted)
}

pub fn list_whisper_model_options() -> Vec<WhisperModelOption> {
    WHISPER_MODEL_SPECS
        .iter()
        .map(|spec| WhisperModelOption {
            id: spec.id.into(),
            label: spec.label.into(),
            file_name: spec.file_name.into(),
            size_bytes: spec.size_bytes,
            installed: installed_official_model_path(spec).is_some(),
        })
        .collect()
}

fn official_model_target_path(spec: &WhisperModelSpec) -> Result<PathBuf, AppError> {
    Ok(whisper_models_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper model directory".into()))?
        .join(spec.file_name))
}

fn installed_official_model_path(spec: &WhisperModelSpec) -> Option<PathBuf> {
    for dir in whisper_model_search_dirs() {
        let path = dir.join(spec.file_name);
        if path.is_file()
            && verify_file_size(&path, spec.size_bytes).is_ok()
            && verify_sha256(&path, spec.sha256).is_ok()
        {
            return path.canonicalize().ok();
        }
    }
    None
}

fn trust_model_candidate(candidate: &LocalWhisperModelCandidate) -> Result<(), AppError> {
    let mut manifest = load_whisper_manifest().unwrap_or_default();
    manifest.model_path = Some(candidate.model_path.clone());
    manifest.model_sha256 = Some(candidate.model_sha256.clone());
    save_whisper_manifest(&manifest)
}

pub fn open_whisper_model_folder() -> Result<Vec<String>, AppError> {
    let primary_dir = whisper_models_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper model directory".into()))?;
    fs::create_dir_all(&primary_dir).map_err(AppError::Io)?;

    let dirs = vec![primary_dir];

    for dir in &dirs {
        folder_open::open_folder(dir)?;
    }

    Ok(dirs
        .into_iter()
        .map(|dir| dir.to_string_lossy().to_string())
        .collect())
}

pub fn activate_installed_whisper_model(
    model_id: String,
) -> Result<LocalWhisperModelCandidate, AppError> {
    let _guard = acquire_whisper_state_lock()?;
    let spec = model_spec_by_id(model_id.trim())?;
    let target_path = installed_official_model_path(&spec).ok_or_else(|| {
        AppError::Audio("Selected official Whisper model has not been downloaded".into())
    })?;
    let candidate = model_candidate_from_path(target_path)?;
    trust_model_candidate(&candidate)?;
    Ok(candidate)
}

fn model_spec_by_id(id: &str) -> Result<WhisperModelSpec, AppError> {
    WHISPER_MODEL_SPECS
        .iter()
        .copied()
        .find(|spec| spec.id == id)
        .ok_or_else(|| AppError::Audio("Unknown Whisper model option".into()))
}

fn safe_zip_entry_path(path: &Path) -> Option<PathBuf> {
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    if path
        .to_string_lossy()
        .chars()
        .any(|ch| ch == '\0' || ch.is_control())
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn find_file_named(dir: &Path, allowed_names: &[&str]) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && file_name_matches(&path, allowed_names) {
            return path.canonicalize().ok();
        }
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, allowed_names) {
                return Some(found);
            }
        }
    }
    None
}

fn manifest_trusts_runner_candidate(
    manifest: &WhisperManifest,
    candidate: &LocalWhisperRunnerCandidate,
) -> bool {
    if manifest.runner_sha256.as_deref() != Some(candidate.runner_sha256.as_str()) {
        return false;
    }
    let Some(path) = manifest.runner_path.as_deref() else {
        return false;
    };
    let Ok(canonical) = Path::new(path).canonicalize() else {
        return false;
    };
    canonical == PathBuf::from(&candidate.runner_path)
        && verify_sha256(&canonical, &candidate.runner_sha256).is_ok()
}

fn safe_extraction_file_path(base_dir: &Path, relative: &Path) -> Result<PathBuf, AppError> {
    let base = base_dir.canonicalize().map_err(AppError::Io)?;
    let out_path = base_dir.join(relative);
    let parent = out_path
        .parent()
        .ok_or_else(|| AppError::Internal("Zip entry has no parent directory".into()))?;
    fs::create_dir_all(parent).map_err(AppError::Io)?;
    let parent = parent.canonicalize().map_err(AppError::Io)?;
    if !parent.starts_with(&base) {
        return Err(AppError::Internal(
            "Zip entry escaped the temporary extraction directory".into(),
        ));
    }
    Ok(out_path)
}

#[cfg(windows)]
pub fn install_whisper_runner(app: &AppHandle) -> Result<LocalWhisperRunnerCandidate, AppError> {
    let _guard = acquire_whisper_state_lock()?;
    let runner_root = whisper_runner_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper runner directory".into()))?;
    fs::create_dir_all(&runner_root).map_err(AppError::Io)?;
    let install_dir = runner_root.join(format!("whisper-bin-x64-{}", WHISPER_RUNNER_VERSION));
    let mut manifest = load_whisper_manifest().unwrap_or_default();

    if let Some(existing_runner) = find_file_named(&install_dir, runner_allowed_file_names()) {
        let candidate = runner_candidate_from_path(existing_runner)?;
        if manifest_trusts_runner_candidate(&manifest, &candidate) {
            emit_install_progress(
                app,
                WHISPER_RUNNER_EVENT,
                100.0,
                "finished",
                "Whisper runner is ready",
            );
            return Ok(candidate);
        }
    }

    emit_install_progress(
        app,
        WHISPER_RUNNER_EVENT,
        0.0,
        "downloading",
        "Downloading Whisper runner...",
    );

    let unique = whisper_unique_suffix();
    let tmp_dir = runner_root.join(format!(
        "whisper-bin-x64-{}-extract-{}.tmp",
        WHISPER_RUNNER_VERSION, unique
    ));
    if tmp_dir.exists() {
        fs::remove_dir_all(&tmp_dir).map_err(AppError::Io)?;
    }
    fs::create_dir_all(&tmp_dir).map_err(AppError::Io)?;
    let mut tmp_dir_guard = TempDirGuard::new(tmp_dir.clone());

    let zip_path = runner_root.join(format!(
        "whisper-runner-download-{}.zip.tmp",
        whisper_unique_suffix()
    ));
    let mut zip_guard = TempFileGuard::new(zip_path.clone());
    download_to_file(
        app,
        WHISPER_RUNNER_EVENT,
        "Whisper runner",
        WHISPER_RUNNER_DOWNLOAD_URL,
        &zip_path,
        MAX_WHISPER_RUNNER_DOWNLOAD_BYTES,
        0.0,
        70.0,
    )?;

    emit_install_progress(
        app,
        WHISPER_RUNNER_EVENT,
        75.0,
        "verifying",
        "Verifying Whisper runner...",
    );
    verify_sha256(&zip_path, WHISPER_RUNNER_ZIP_SHA256)?;

    emit_install_progress(
        app,
        WHISPER_RUNNER_EVENT,
        82.0,
        "extracting",
        "Extracting Whisper runner...",
    );
    let zip_file = fs::File::open(&zip_path).map_err(AppError::Io)?;
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|err| AppError::Internal(format!("Failed to open Whisper runner zip: {}", err)))?;
    let mut extracted_bytes = 0u64;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| AppError::Internal(format!("Failed to read zip entry: {}", err)))?;
        if entry.name().ends_with('/') {
            continue;
        }
        let Some(enclosed_raw) = entry.enclosed_name() else {
            continue;
        };
        let Some(enclosed) = safe_zip_entry_path(&enclosed_raw) else {
            continue;
        };
        extracted_bytes += entry.size();
        if extracted_bytes > MAX_WHISPER_EXTRACTED_BYTES {
            return Err(AppError::Internal(
                "Whisper runner zip exceeded extraction safety limit".into(),
            ));
        }

        let out_path = safe_extraction_file_path(&tmp_dir, &enclosed)?;
        let mut out_file = fs::File::create(&out_path).map_err(AppError::Io)?;
        std::io::copy(&mut entry, &mut out_file).map_err(AppError::Io)?;
    }
    zip_guard.disarm();
    let _ = fs::remove_file(&zip_path);

    let runner = find_file_named(&tmp_dir, runner_allowed_file_names()).ok_or_else(|| {
        AppError::Internal("Whisper runner zip did not contain whisper-cli.exe".into())
    })?;
    let extracted_candidate = runner_candidate_from_path(runner)?;
    if extracted_candidate.runner_sha256 != WHISPER_RUNNER_EXE_SHA256 {
        return Err(AppError::Internal(format!(
            "Whisper runner checksum mismatch. Expected SHA-256 {}, got {}",
            WHISPER_RUNNER_EXE_SHA256, extracted_candidate.runner_sha256
        )));
    }

    if install_dir.exists() {
        fs::remove_dir_all(&install_dir).map_err(AppError::Io)?;
    }
    fs::rename(&tmp_dir, &install_dir).map_err(AppError::Io)?;
    tmp_dir_guard.disarm();

    let runner = find_file_named(&install_dir, runner_allowed_file_names()).ok_or_else(|| {
        AppError::Internal("Whisper runner install did not contain whisper-cli.exe".into())
    })?;
    let candidate = runner_candidate_from_path(runner)?;
    manifest.runner_path = Some(candidate.runner_path.clone());
    manifest.runner_sha256 = Some(candidate.runner_sha256.clone());
    save_whisper_manifest(&manifest)?;

    emit_install_progress(
        app,
        WHISPER_RUNNER_EVENT,
        100.0,
        "finished",
        "Whisper runner installed",
    );
    Ok(candidate)
}

#[cfg(not(windows))]
pub fn install_whisper_runner(_app: &AppHandle) -> Result<LocalWhisperRunnerCandidate, AppError> {
    Err(AppError::Internal(
        "Automatic Whisper runner download is currently available for Windows builds".into(),
    ))
}

pub fn install_whisper_model(
    app: &AppHandle,
    model_id: String,
) -> Result<LocalWhisperModelCandidate, AppError> {
    let _guard = acquire_whisper_state_lock()?;
    let spec = model_spec_by_id(model_id.trim())?;
    let models_dir = whisper_models_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper model directory".into()))?;
    fs::create_dir_all(&models_dir).map_err(AppError::Io)?;
    let target_path = official_model_target_path(&spec)?;

    if target_path.is_file() {
        verify_file_size(&target_path, spec.size_bytes)?;
        verify_sha256(&target_path, spec.sha256)?;
        let candidate =
            model_candidate_from_path(target_path.canonicalize().map_err(AppError::Io)?)?;
        trust_model_candidate(&candidate)?;
        emit_install_progress(
            app,
            WHISPER_MODEL_EVENT,
            100.0,
            "finished",
            format!("Whisper model is ready: {}", spec.label),
        );
        return Ok(candidate);
    }

    emit_install_progress(
        app,
        WHISPER_MODEL_EVENT,
        0.0,
        "downloading",
        format!("Downloading Whisper model: {}", spec.label),
    );

    let tmp_path = target_path.with_extension("bin.tmp");
    let mut tmp_guard = TempFileGuard::new(tmp_path.clone());
    download_to_file(
        app,
        WHISPER_MODEL_EVENT,
        "Whisper model",
        spec.url,
        &tmp_path,
        spec.max_bytes,
        0.0,
        92.0,
    )?;

    emit_install_progress(
        app,
        WHISPER_MODEL_EVENT,
        94.0,
        "verifying",
        "Verifying Whisper model...",
    );
    verify_file_size(&tmp_path, spec.size_bytes)?;
    verify_sha256(&tmp_path, spec.sha256)?;

    if target_path.exists() {
        fs::remove_file(&target_path).map_err(AppError::Io)?;
    }
    fs::rename(&tmp_path, &target_path).map_err(AppError::Io)?;
    tmp_guard.disarm();

    let candidate = model_candidate_from_path(target_path.canonicalize().map_err(AppError::Io)?)?;
    trust_model_candidate(&candidate)?;

    emit_install_progress(
        app,
        WHISPER_MODEL_EVENT,
        100.0,
        "finished",
        format!("Whisper model installed: {}", spec.label),
    );
    Ok(candidate)
}

fn normalize_language(language: Option<String>) -> &'static str {
    match language
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "zh" | "zh-tw" | "zh-cn" | "chinese" => "zh",
        "en" | "english" => "en",
        "ja" | "jp" | "japanese" => "ja",
        "ko" | "kr" | "korean" => "ko",
        _ => "auto",
    }
}

fn whisper_unique_suffix() -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = WHISPER_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", std::process::id(), now_ms, sequence)
}

fn unique_output_stem(output_dir: &Path) -> PathBuf {
    output_dir.join(format!("vocalsync-whisper-{}", whisper_unique_suffix()))
}

fn whisper_audio_extension(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "flac" => "flac",
        "mp3" => "mp3",
        "ogg" => "ogg",
        _ => "wav",
    }
}

fn stage_audio_for_whisper_cli(
    source: &Path,
    output_dir: &Path,
) -> Result<(PathBuf, TempFileGuard), AppError> {
    let staged_path = output_dir.join(format!(
        "vocalsync-whisper-input-{}.{}",
        whisper_unique_suffix(),
        whisper_audio_extension(source)
    ));
    let guard = TempFileGuard::new(staged_path.clone());

    if fs::hard_link(source, &staged_path).is_err() {
        fs::copy(source, &staged_path).map_err(AppError::Io)?;
    }

    Ok((staged_path, guard))
}

fn generated_transcript_path(stem: &Path) -> Result<PathBuf, AppError> {
    let json_path = stem.with_extension("json");
    let srt_path = stem.with_extension("srt");
    if json_path.is_file() {
        return Ok(json_path);
    }
    if srt_path.is_file() {
        return Ok(srt_path);
    }
    Err(AppError::Audio(
        "Whisper did not produce a JSON or SRT timed transcript".into(),
    ))
}

fn clean_whisper_error_message(message: String) -> String {
    let trimmed = message.trim();
    let usage_markers = ["\r\nusage:", "\nusage:", " usage:"];
    let end = usage_markers
        .iter()
        .filter_map(|marker| trimmed.find(marker))
        .min()
        .unwrap_or(trimmed.len());
    let mut cleaned = trimmed[..end].trim().to_string();
    const MAX_ERROR_LEN: usize = 900;
    if cleaned.chars().count() > MAX_ERROR_LEN {
        cleaned = cleaned.chars().take(MAX_ERROR_LEN).collect();
        cleaned.push_str("...");
    }
    cleaned
}

pub fn transcribe_vocals_with_whisper(
    app: &AppHandle,
    audio_path: String,
    language: Option<String>,
) -> Result<WhisperTranscriptionResult, AppError> {
    let started_at = Instant::now();
    emit_transcription_progress(app, 0.0, "preparing", "Preparing vocals.wav", 0, None);

    security::validate_path_safe(&audio_path)?;
    let audio_path = canonical_existing_file(&audio_path, "Audio")?;
    let runner = find_whisper_runner()
        .ok_or_else(|| AppError::Audio("Whisper runner is not configured".into()))?;
    let model = find_whisper_model()
        .ok_or_else(|| AppError::Audio("Whisper model is not configured".into()))?;

    if let Err(err) = cleanup_whisper_cache() {
        log::warn!("[whisper] failed to clean stale cache before transcription: {err}");
    }

    let output_dir = whisper_cache_dir()
        .ok_or_else(|| AppError::Internal("Could not locate Whisper cache directory".into()))?;
    fs::create_dir_all(&output_dir).map_err(AppError::Io)?;
    let output_stem = unique_output_stem(&output_dir);
    let (staged_audio_path, _staged_audio_guard) =
        stage_audio_for_whisper_cli(&audio_path, &output_dir)?;
    let lang = normalize_language(language);
    let model_arg = command_arg_path(&model);
    let audio_arg = command_arg_path(&staged_audio_path);
    let output_stem_arg = command_arg_path(&output_stem);

    let mut command = Command::new(&runner);
    if let Some(runner_dir) = runner.parent() {
        command.current_dir(runner_dir);
    }
    command
        .arg("-m")
        .arg(&model_arg)
        .arg("-f")
        .arg(&audio_arg)
        .arg("-l")
        .arg(lang)
        .arg("-ojf")
        .arg("-osrt")
        .arg("-pp")
        .arg("-of")
        .arg(&output_stem_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(AppError::Io)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_text = Arc::new(Mutex::new(String::new()));
    let stderr_text = Arc::new(Mutex::new(String::new()));
    let last_reported_percent = Arc::new(Mutex::new(0.0));
    let mut readers = Vec::new();

    if let Some(stdout) = stdout {
        readers.push(spawn_whisper_output_reader(
            stdout,
            app.clone(),
            Arc::clone(&stdout_text),
            started_at,
            Arc::clone(&last_reported_percent),
        ));
    }
    if let Some(stderr) = stderr {
        readers.push(spawn_whisper_output_reader(
            stderr,
            app.clone(),
            Arc::clone(&stderr_text),
            started_at,
            Arc::clone(&last_reported_percent),
        ));
    }

    let status = child.wait().map_err(AppError::Io)?;
    for reader in readers {
        let _ = reader.join();
    }

    if !status.success() {
        let stderr = stderr_text
            .lock()
            .map(|text| text.trim().to_string())
            .unwrap_or_default();
        let stdout = stdout_text
            .lock()
            .map(|text| text.trim().to_string())
            .unwrap_or_default();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Whisper exited with status {}", status)
        };
        let message = clean_whisper_error_message(message);
        return Err(AppError::Audio(format!(
            "Whisper transcription failed: {}",
            message
        )));
    }

    let transcript_path = generated_transcript_path(&output_stem)?;
    emit_transcription_progress(
        app,
        100.0,
        "finished",
        "Whisper transcription finished",
        started_at.elapsed().as_secs(),
        Some(0),
    );
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    let segments = lyrics_forced_aligner::load_timed_transcript(&transcript_path_str)?;
    let subtitle_path = output_stem.with_extension("srt");

    Ok(WhisperTranscriptionResult {
        transcript_path: transcript_path_str,
        subtitle_path: subtitle_path
            .is_file()
            .then(|| subtitle_path.to_string_lossy().to_string()),
        segment_count: segments.len(),
        model_path: model.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_languages() {
        assert_eq!(normalize_language(Some("zh-TW".into())), "zh");
        assert_eq!(normalize_language(Some("english".into())), "en");
        assert_eq!(normalize_language(Some("ja".into())), "ja");
        assert_eq!(normalize_language(Some("unknown".into())), "auto");
    }

    #[test]
    fn accepts_common_whisper_model_extensions() {
        assert!(model_extension_allowed(Path::new("ggml-base.bin")));
        assert!(model_extension_allowed(Path::new("ggml-large-v3.gguf")));
        assert!(!model_extension_allowed(Path::new("model.txt")));
    }

    #[test]
    fn trims_whisper_usage_from_error_message() {
        let message = clean_whisper_error_message(
            "error: input file not found 'song.wav' error: no input files specified usage: whisper-cli.exe [options]".into(),
        );
        assert_eq!(
            message,
            "error: input file not found 'song.wav' error: no input files specified"
        );
    }

    #[test]
    fn parses_whisper_cli_progress_percent() {
        assert_eq!(
            parse_whisper_progress_percent("whisper_print_progress_callback: progress =  42%"),
            Some(42.0)
        );
        assert_eq!(
            parse_whisper_progress_percent("\rprogress = 99.5%"),
            Some(99.5)
        );
        assert_eq!(parse_whisper_progress_percent("no progress"), None);
    }

    #[test]
    fn estimates_transcription_eta_from_progress() {
        assert_eq!(transcription_eta_seconds(25.0, 10), Some(30));
        assert_eq!(transcription_eta_seconds(0.0, 10), None);
        assert_eq!(transcription_eta_seconds(100.0, 10), None);
    }

    #[test]
    fn chooses_whisper_audio_extension_for_staged_input() {
        assert_eq!(whisper_audio_extension(Path::new("song.wav")), "wav");
        assert_eq!(whisper_audio_extension(Path::new("song.FLAC")), "flac");
        assert_eq!(whisper_audio_extension(Path::new("song.mp3")), "mp3");
        assert_eq!(whisper_audio_extension(Path::new("song.ogg")), "ogg");
        assert_eq!(whisper_audio_extension(Path::new("song.m4a")), "wav");
    }

    #[test]
    fn stages_audio_to_ascii_whisper_cli_path() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("whisper-stage-{}", whisper_unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("ReoNa 新曲.wav");
        fs::write(&source, b"audio").unwrap();

        let (staged, guard) = stage_audio_for_whisper_cli(&source, &dir).unwrap();
        assert!(staged
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .is_ascii());
        assert_eq!(fs::read(&staged).unwrap(), b"audio");

        drop(guard);
        assert!(!staged.exists());
        assert!(source.exists());

        let _ = fs::remove_file(source);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_generated_transcript_artifacts_removes_owned_cache_files() {
        let dir = whisper_cache_dir().unwrap();
        let stem = dir.join(format!(
            "vocalsync-whisper-cleanup-test-{}",
            whisper_unique_suffix()
        ));
        let json = stem.with_extension("json");
        let srt = stem.with_extension("srt");
        fs::write(&json, "{}").unwrap();
        fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nhello").unwrap();

        let removed = cleanup_generated_transcript_artifacts(&json).unwrap();

        assert_eq!(removed, 2);
        assert!(!json.exists());
        assert!(!srt.exists());
    }

    #[cfg(windows)]
    #[test]
    fn strips_windows_verbatim_prefix_for_whisper_cli_args() {
        assert_eq!(
            command_arg_path(Path::new(r"\\?\C:\Users\example\song.wav")),
            PathBuf::from(r"C:\Users\example\song.wav")
        );
        assert_eq!(
            command_arg_path(Path::new(r"\\?\UNC\server\share\song.wav")),
            PathBuf::from(r"\\server\share\song.wav")
        );
    }
}
