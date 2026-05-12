use crate::core::portable_paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: String,
    pub download_folder: String,
    pub last_backing_path: String,
    pub backing_volume: u16,
    pub mic_gain: u16,
    #[serde(default = "default_guide_volume")]
    pub guide_volume: u16,
    pub export_volume: u16,
    pub export_prefix: String,
    pub auto_balance: bool,
    #[serde(default)]
    pub mixer_settings_version: u8,
    pub playback_speed: f32,
    pub transpose_semitones: i8,
    pub window_geometry: Option<String>,
    pub calibrated_latency_ms: Option<f64>,
    pub manual_offset_ms: i32,
    #[serde(default = "default_pitch_engine")]
    pub pitch_engine: String,
    #[serde(default = "default_show_startup_guide")]
    pub show_startup_guide: bool,
}

fn default_pitch_engine() -> String {
    "auto".to_string()
}

fn default_show_startup_guide() -> bool {
    true
}

fn default_guide_volume() -> u16 {
    25
}

fn default_download_folder() -> String {
    portable_paths::ensure_dir("downloads")
        .unwrap_or_else(|_| portable_paths::path("downloads"))
        .to_string_lossy()
        .to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "light".to_string(),
            download_folder: default_download_folder(),
            last_backing_path: String::new(),
            backing_volume: 10,
            mic_gain: 100,
            guide_volume: default_guide_volume(),
            export_volume: 50,
            export_prefix: String::new(),
            auto_balance: true,
            mixer_settings_version: 0,
            playback_speed: 1.0,
            transpose_semitones: 0,
            window_geometry: None,
            calibrated_latency_ms: None,
            manual_offset_ms: 0,
            pitch_engine: default_pitch_engine(),
            show_startup_guide: default_show_startup_guide(),
        }
    }
}

impl AppSettings {
    fn settings_path() -> PathBuf {
        portable_paths::settings_path()
    }

    fn load_from_path(path: &Path) -> Option<Self> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) => {
                log::error!("[settings] failed to read settings at {:?}: {}", path, err);
                return None;
            }
        };

        match serde_json::from_str::<Self>(&content) {
            Ok(settings) => Some(settings.into_runtime_paths()),
            Err(err) => {
                let backup = path.with_extension("json.bak");
                if let Err(backup_err) = std::fs::copy(path, &backup) {
                    log::error!(
                        "[settings] failed to back up invalid settings {:?} to {:?}: {}",
                        path,
                        backup,
                        backup_err
                    );
                } else {
                    log::warn!(
                        "[settings] invalid settings at {:?}: {}; backed up to {:?}",
                        path,
                        err,
                        backup
                    );
                }
                None
            }
        }
    }

    pub fn load_or_default() -> Self {
        let path = Self::settings_path();
        if path.exists() {
            if let Some(settings) = Self::load_from_path(&path) {
                let _ = settings.save();
                return settings;
            }
        }

        Self::default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let persisted = self.to_persisted_paths();
        let content = serde_json::to_string_pretty(&persisted)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, content)
    }

    fn into_runtime_paths(mut self) -> Self {
        self.download_folder =
            resolve_settings_path(&self.download_folder).unwrap_or_else(default_download_folder);
        if let Some(path) = resolve_settings_path(&self.last_backing_path) {
            self.last_backing_path = path;
        }
        self
    }

    fn to_persisted_paths(&self) -> Self {
        let mut persisted = self.clone();
        persisted.download_folder = encode_settings_path(&persisted.download_folder);
        persisted.last_backing_path = encode_settings_path(&persisted.last_backing_path);
        persisted
    }
}

fn resolve_settings_path(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return Some(String::new());
    }
    portable_paths::resolve_stored_path_text(value).map(|path| {
        let path = if path.is_absolute() {
            path
        } else {
            portable_paths::path(path)
        };
        portable_paths::display_path(&path)
    })
}

fn encode_settings_path(value: &str) -> String {
    if value.trim().is_empty() {
        return String::new();
    }
    portable_paths::encode_path_for_storage(Path::new(value))
}
