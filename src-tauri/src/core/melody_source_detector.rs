//! 依伴奏檔案路徑自動偵測旁邊是否有目標旋律來源檔。
//!
//! 偵測順序（優先度由高到低）：
//! 1. 同資料夾同檔名 `.txt`（UltraStar）
//! 2. 寬鬆 fallback：同資料夾若**只有一個** `.txt`，自動當 UltraStar 候選
//!    （給常見的 `Song [Karaoke].mp3` vs `Song.txt` 這類命名差異）
//!
//! MIDI / 人聲軌等其他格式請使用者手動載入（SetupTab 的「載入標註檔 / MIDI」按鈕）。
//!
//! 若全都找不到，回傳 [`DetectedSource::None`]，前端顯示「沒有目標旋律」提示。

use std::path::{Path, PathBuf};

/// 自動偵測出的目標旋律來源
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedSource {
    UltraStar(PathBuf),
    None,
}

/// 對 `backing_path`（伴奏音檔）偵測同資料夾的目標旋律來源檔。
///
/// 如果 `backing_path` 本身不存在或沒有父資料夾，直接回傳 None。
pub fn detect_melody_source(backing_path: &Path) -> DetectedSource {
    let parent = match backing_path.parent() {
        Some(p) => p,
        None => return DetectedSource::None,
    };

    let stem = match backing_path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return DetectedSource::None,
    };

    // 1. 同檔名 .txt
    let txt_exact = parent.join(format!("{stem}.txt"));
    if txt_exact.is_file() {
        return DetectedSource::UltraStar(txt_exact);
    }

    // 2. 寬鬆 fallback：同資料夾若只有「唯一一個」.txt，拿來當 UltraStar 候選
    if let Some(unique_txt) = find_unique_txt_in_dir(parent) {
        return DetectedSource::UltraStar(unique_txt);
    }

    DetectedSource::None
}

/// 掃描資料夾找「唯一一個」`.txt` 檔；多於一個回傳 None（避免猜錯）。
fn find_unique_txt_in_dir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut found: Option<PathBuf> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_txt = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("txt"))
            .unwrap_or(false);
        if !is_txt {
            continue;
        }
        if found.is_some() {
            // 多於一個，放棄自動選
            return None;
        }
        found = Some(path);
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    /// 建立一個暫存資料夾並回傳路徑；呼叫端負責清理。
    fn make_tempdir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "vocalsync_melody_detect_{}_{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(path).unwrap();
        f.write_all(b"dummy").unwrap();
    }

    #[test]
    fn detects_ultrastar_txt_by_exact_stem() {
        let dir = make_tempdir("ultrastar_exact");
        let mp3 = dir.join("song.mp3");
        let txt = dir.join("song.txt");
        touch(&mp3);
        touch(&txt);

        let detected = detect_melody_source(&mp3);
        assert_eq!(detected, DetectedSource::UltraStar(txt));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallbacks_to_unique_txt_when_stem_mismatch() {
        let dir = make_tempdir("ultrastar_fallback");
        let mp3 = dir.join("song [Karaoke].mp3");
        let txt = dir.join("song.txt");
        touch(&mp3);
        touch(&txt);

        let detected = detect_melody_source(&mp3);
        assert_eq!(detected, DetectedSource::UltraStar(txt));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn returns_none_when_multiple_txt_and_no_stem_match() {
        let dir = make_tempdir("ultrastar_ambiguous");
        let mp3 = dir.join("song [Karaoke].mp3");
        let txt_a = dir.join("a.txt");
        let txt_b = dir.join("b.txt");
        touch(&mp3);
        touch(&txt_a);
        touch(&txt_b);

        let detected = detect_melody_source(&mp3);
        assert_eq!(detected, DetectedSource::None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn midi_in_same_folder_is_not_auto_detected() {
        let dir = make_tempdir("midi_no_auto");
        let mp3 = dir.join("tune.mp3");
        let _mid = dir.join("tune.mid");
        touch(&mp3);
        touch(&_mid);

        // MIDI 不再自動偵測，應回傳 None
        let detected = detect_melody_source(&mp3);
        assert_eq!(detected, DetectedSource::None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn returns_none_for_lonely_audio_file() {
        let dir = make_tempdir("lonely");
        let mp3 = dir.join("alone.mp3");
        touch(&mp3);

        let detected = detect_melody_source(&mp3);
        assert_eq!(detected, DetectedSource::None);

        fs::remove_dir_all(&dir).ok();
    }
}
