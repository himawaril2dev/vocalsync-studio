//! 應用程式設定管理

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: String,
    pub download_folder: String,
    pub last_backing_path: String,
    pub backing_volume: u16, // 百分比 0~200
    pub mic_gain: u16,       // 百分比 0~500
    pub export_volume: u16,  // 百分比 0~200
    pub export_prefix: String,
    pub auto_balance: bool,
    pub playback_speed: f32,     // 0.5~2.0
    pub transpose_semitones: i8, // -12~+12
    pub window_geometry: Option<String>,
    pub calibrated_latency_ms: Option<f64>,
    pub manual_offset_ms: i32,
    /// 音高偵測引擎偏好："auto" | "crepe" | "yin"
    #[serde(default = "default_pitch_engine")]
    pub pitch_engine: String,
}

fn default_pitch_engine() -> String {
    "auto".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "light".to_string(),
            download_folder: dirs_next::download_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            last_backing_path: String::new(),
            backing_volume: 5,
            mic_gain: 100,
            export_volume: 50,
            export_prefix: String::new(),
            auto_balance: true,
            playback_speed: 1.0,
            transpose_semitones: 0,
            window_geometry: None,
            calibrated_latency_ms: None,
            manual_offset_ms: 0,
            pitch_engine: default_pitch_engine(),
        }
    }
}

impl AppSettings {
    fn settings_path() -> PathBuf {
        let mut path = dirs_next::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("VocalSyncStudio");
        std::fs::create_dir_all(&path).ok();
        path.push("settings.json");
        path
    }

    pub fn load_or_default() -> Self {
        let path = Self::settings_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(settings) => settings,
                    Err(e) => {
                        // 設定檔損壞：備份壞檔再用預設值
                        let backup = path.with_extension("json.bak");
                        if let Err(be) = std::fs::copy(&path, &backup) {
                            log::error!(
                                "[settings] 備份損壞設定檔失敗: {:?} → {:?}: {}",
                                path,
                                backup,
                                be
                            );
                        } else {
                            log::warn!(
                                "[settings] 設定檔解析失敗（{}），已備份至 {:?}，使用預設值",
                                e,
                                backup
                            );
                        }
                        Self::default()
                    }
                },
                Err(e) => {
                    log::error!("[settings] 無法讀取設定檔: {}", e);
                    Self::default()
                }
            }
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::settings_path();
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, content)
    }
}
