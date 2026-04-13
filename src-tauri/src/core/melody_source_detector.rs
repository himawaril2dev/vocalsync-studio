//! 依伴奏檔案路徑偵測旁邊是否有可自動掛載的目標旋律來源。
//!
//! 目前不再自動偵測任何旋律檔，統一由使用者手動載入 MIDI 或匯入人聲軌。

use std::path::Path;

/// 自動偵測出的目標旋律來源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedSource {
    None,
}

/// 對 `backing_path` 偵測同資料夾的目標旋律來源檔。
pub fn detect_melody_source(_backing_path: &Path) -> DetectedSource {
    DetectedSource::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn always_returns_none_for_audio_file() {
        let detected = detect_melody_source(&PathBuf::from("song.mp3"));
        assert_eq!(detected, DetectedSource::None);
    }

    #[test]
    fn returns_none_for_path_without_parent() {
        let detected = detect_melody_source(Path::new("song.mp3"));
        assert_eq!(detected, DetectedSource::None);
    }
}
