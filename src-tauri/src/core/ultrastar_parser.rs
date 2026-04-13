//! UltraStar `.txt` 歌曲包解析器
//!
//! UltraStar 是全球最大的卡拉 OK 練唱社群格式（10 萬+ 首歌），人工標注的音符 +
//! 歌詞音節時間軸，品質遠勝任何自動偵測演算法。這是我們跳脫「音高偵測抓到 bass
//! 不是人聲」困境的主要出路。
//!
//! # 檔案格式概要
//!
//! ```text
//! #TITLE:櫻花櫻花              ← metadata
//! #ARTIST:日本民謠
//! #MP3:櫻花櫻花.mp3
//! #BPM:120.5                   ← 見下方「BPM 陷阱」
//! #GAP:500                     ← 起始偏移（毫秒）
//! : 0 4 60 さ                  ← 一般音符：start_beat length midi_offset syllable
//! * 4 4 64 く                  ← 黃金音符（得分加倍）
//! F 8 4 67 ら                  ← 自由音符（任何音高都算對）
//! R 12 4 67 ぷ                 ← Rap 音符（視為一般音符處理）
//! - 16                         ← 換頁標記（忽略）
//! E                            ← 結尾
//! ```
//!
//! # BPM 陷阱（很重要）
//!
//! UltraStar 的 `#BPM` 欄位**不是**實際的每分鐘四分音符數，而是
//! 「每分鐘四分音符數 ÷ 4」（即每分鐘的「拍」數除以 4，因為 UltraStar 內部
//! 的 beat 單位是十六分音符。）。因此：
//!
//! ```text
//! secs_per_beat = 60 / (ultrastar_bpm * 4) = 15 / ultrastar_bpm
//! ```
//!
//! 這個換算公式錯一點音符就會全部錯位，務必在 unit test 驗。
//!
//! # MIDI 換算
//!
//! UltraStar 裡的 pitch value 是以 **C4 (MIDI 60)** 為 0 的偏移量：
//!
//! ```text
//! midi_pitch = 60 + ultrastar_pitch_offset
//! ```
//!
//! 所以 `: 0 4 60 さ` 的 `60` 代表 C4 + 60 半音 = C9（通常是無意義的極高音，
//! 但規格就是如此；一般歌曲的 pitch offset 在 [-20, +20] 區間）。

use crate::core::melody_track::{MelodyNote, MelodySource, MelodyTrack};
use crate::error::AppError;
use std::fs;
use std::path::Path;

/// 載入 UltraStar `.txt` 檔並解析成 MelodyTrack。
pub fn load_ultrastar(path: &str) -> Result<MelodyTrack, AppError> {
    let raw_bytes = fs::read(path)
        .map_err(|e| AppError::Audio(format!("無法讀取 UltraStar 檔：{e}")))?;
    let content = decode_text(&raw_bytes);

    parse_ultrastar(&content, path)
}

/// 從字串內容解析（給測試與其他模組用）。
pub fn parse_ultrastar(content: &str, source_path: &str) -> Result<MelodyTrack, AppError> {
    let mut title: Option<String> = None;
    let mut artist: Option<String> = None;
    let mut bpm: Option<f32> = None;
    let mut gap_ms: f64 = 0.0;
    let mut parsed_notes: Vec<ParsedNote> = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // metadata 標頭
        if let Some(rest) = line.strip_prefix('#') {
            if let Some((key, value)) = rest.split_once(':') {
                let key = key.trim().to_uppercase();
                let value = value.trim();
                match key.as_str() {
                    "TITLE" => title = Some(value.to_string()),
                    "ARTIST" => artist = Some(value.to_string()),
                    "BPM" => {
                        // 接受 "120.5" 或 "120,5"（部分歐洲歌曲包使用逗號）
                        let normalized = value.replace(',', ".");
                        bpm = normalized.parse::<f32>().ok();
                    }
                    "GAP" => {
                        let normalized = value.replace(',', ".");
                        gap_ms = normalized.parse::<f64>().unwrap_or(0.0);
                    }
                    _ => {} // 其他 metadata 忽略（#MP3/#COVER/#VIDEO 等）
                }
            }
            continue;
        }

        // 結尾標記：遇到就立刻停止，不管後面有什麼垃圾
        if line.starts_with('E') && line.len() == 1 {
            break;
        }
        // 寬容處理 "E " 或 "E xxx" 也視為結尾
        if line == "E" || line.starts_with("E ") {
            break;
        }

        // 換頁標記：`- 16` 或 `- 16 20`，忽略
        if line.starts_with('-') {
            continue;
        }

        // 音符行：`:`, `*`, `F`, `R`
        if let Some(note) = parse_note_line(line) {
            parsed_notes.push(note);
        }
    }

    let bpm = bpm.ok_or_else(|| AppError::Audio("UltraStar 檔缺少 #BPM".to_string()))?;
    if bpm <= 0.0 {
        return Err(AppError::Audio(format!("UltraStar #BPM 不合法：{bpm}")));
    }
    if parsed_notes.is_empty() {
        return Err(AppError::Audio("UltraStar 檔沒有任何音符".to_string()));
    }

    // beat → 秒換算：secs_per_beat = 15 / #BPM
    let secs_per_beat = 15.0 / bpm as f64;
    let gap_secs = gap_ms / 1000.0;

    // ParsedNote → MelodyNote（套用 BPM 與 GAP 換算）
    let melody_notes: Vec<MelodyNote> = parsed_notes
        .iter()
        .map(|n| {
            let start_secs = gap_secs + n.start_beat as f64 * secs_per_beat;
            let duration_secs = (n.length_beats as f64 * secs_per_beat).max(0.0);
            let midi_pitch = (60 + n.pitch_offset).clamp(0, 127) as u8;
            MelodyNote::from_midi(
                start_secs,
                duration_secs,
                midi_pitch,
                if n.syllable.is_empty() {
                    None
                } else {
                    Some(n.syllable.clone())
                },
                n.is_golden,
                n.is_freestyle,
            )
        })
        .collect();

    let total_duration_secs = melody_notes
        .iter()
        .map(|n| n.end_secs())
        .fold(0.0_f64, f64::max);

    Ok(MelodyTrack {
        source: MelodySource::UltraStar {
            txt_path: source_path.to_string(),
            title,
            artist,
            bpm,
        },
        notes: melody_notes,
        total_duration_secs,
        raw_pitch_track: None,
    })
}

// ── 內部結構與 helpers ─────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ParsedNote {
    start_beat: u32,
    length_beats: u32,
    pitch_offset: i32,
    syllable: String,
    is_golden: bool,
    is_freestyle: bool,
}

/// 解析一行音符：`<kind> <start> <length> <pitch> <syllable...>`
///
/// - `kind` 是 `:`（一般）/ `*`（黃金）/ `F`（自由）/ `R`（Rap，視為一般）
/// - syllable 可能包含空格，所以 split 後剩下的全部串起來
fn parse_note_line(line: &str) -> Option<ParsedNote> {
    let mut chars = line.chars();
    let kind = chars.next()?;

    let (is_golden, is_freestyle) = match kind {
        ':' | 'R' => (false, false),
        '*' => (true, false),
        'F' => (false, true),
        _ => return None,
    };

    // 剩下的部分 tokenize
    let rest: &str = chars.as_str();
    // 用 split_whitespace 拿前 3 個數字 token；syllable 是剩下全部
    let mut iter = rest.split_whitespace();
    let start_beat: u32 = iter.next()?.parse().ok()?;
    let length_beats: u32 = iter.next()?.parse().ok()?;
    let pitch_offset: i32 = iter.next()?.parse().ok()?;

    // syllable 可能含空格，join 回去但要保留第一個 token 前的空白行為：
    // 因為 split_whitespace 已經去掉前導空白，這裡直接剩餘 join
    let syllable_tokens: Vec<&str> = iter.collect();
    let syllable = syllable_tokens.join(" ");

    Some(ParsedNote {
        start_beat,
        length_beats,
        pitch_offset,
        syllable,
        is_golden,
        is_freestyle,
    })
}

/// 解碼位元組串成字串，處理 UTF-8 BOM 並 fallback 到 lossy。
///
/// 注意：UltraStar 歌曲包的 `.txt` 編碼很亂，常見 UTF-8 / UTF-8 with BOM /
/// Windows-1252 / Big5。這裡先處理 BOM，非法 UTF-8 則用 lossy，
/// 歌詞變成 `�` 但 metadata 與音符數字通常都是 ASCII 不受影響。
fn decode_text(bytes: &[u8]) -> String {
    let stripped = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };

    if let Ok(s) = std::str::from_utf8(stripped) {
        return s.to_string();
    }
    String::from_utf8_lossy(stripped).to_string()
}

/// Helper 給其他模組：判斷路徑的副檔名是否為 `.txt`（無視大小寫）。
pub fn has_txt_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("txt"))
        .unwrap_or(false)
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "#TITLE:Test Song\n\
                           #ARTIST:Tester\n\
                           #BPM:60\n\
                           #GAP:0\n\
                           : 0 4 0 Hel\n\
                           : 4 4 2 lo\n\
                           E\n";

    #[test]
    fn parses_basic_ultrastar() {
        let track = parse_ultrastar(MINIMAL, "test.txt").unwrap();
        assert_eq!(track.notes.len(), 2);
        // 第一個音符：start_beat=0, length=4, pitch_offset=0, syllable="Hel"
        assert_eq!(track.notes[0].midi_pitch, 60); // 60 + 0 = C4
        assert_eq!(track.notes[0].lyric.as_deref(), Some("Hel"));
        assert!(!track.notes[0].is_golden);
        assert!(!track.notes[0].is_freestyle);
    }

    #[test]
    fn parses_metadata_correctly() {
        let track = parse_ultrastar(MINIMAL, "test.txt").unwrap();
        match &track.source {
            MelodySource::UltraStar {
                title, artist, bpm, ..
            } => {
                assert_eq!(title.as_deref(), Some("Test Song"));
                assert_eq!(artist.as_deref(), Some("Tester"));
                assert!((bpm - 60.0).abs() < 1e-6);
            }
            _ => panic!("expected UltraStar source"),
        }
    }

    #[test]
    fn parses_golden_notes() {
        let src = "#BPM:60\n#GAP:0\n* 0 4 5 gold\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert_eq!(track.notes.len(), 1);
        assert!(track.notes[0].is_golden);
        assert!(!track.notes[0].is_freestyle);
        assert_eq!(track.notes[0].midi_pitch, 65); // 60 + 5
    }

    #[test]
    fn parses_freestyle_notes() {
        let src = "#BPM:60\n#GAP:0\nF 0 4 0 yo\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert_eq!(track.notes.len(), 1);
        assert!(track.notes[0].is_freestyle);
        assert!(!track.notes[0].is_golden);
    }

    #[test]
    fn handles_page_break_lines() {
        let src = "#BPM:60\n#GAP:0\n: 0 4 0 a\n- 4\n: 8 4 2 b\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert_eq!(track.notes.len(), 2);
    }

    #[test]
    fn handles_end_marker_and_ignores_trailing_junk() {
        let src = "#BPM:60\n#GAP:0\n: 0 4 0 a\nE\n: 99 4 99 junk\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert_eq!(track.notes.len(), 1);
    }

    #[test]
    fn converts_beat_to_seconds_with_bpm() {
        // BPM 60 → secs_per_beat = 15 / 60 = 0.25
        // 第一個音符 start_beat=0 → 0 秒；length=4 → 1.0 秒
        // 第二個音符 start_beat=4 → 1.0 秒
        let track = parse_ultrastar(MINIMAL, "t.txt").unwrap();
        assert!((track.notes[0].start_secs - 0.0).abs() < 1e-9);
        assert!((track.notes[0].duration_secs - 1.0).abs() < 1e-9);
        assert!((track.notes[1].start_secs - 1.0).abs() < 1e-9);
    }

    #[test]
    fn applies_gap_offset() {
        // GAP 500ms → 所有音符的 start_secs 平移 +0.5 秒
        let src = "#BPM:60\n#GAP:500\n: 0 4 0 a\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert!((track.notes[0].start_secs - 0.5).abs() < 1e-9);
    }

    #[test]
    fn converts_pitch_offset_to_midi() {
        // UltraStar pitch offset 0 → MIDI 60 (C4)；-12 → MIDI 48 (C3)
        let src = "#BPM:60\n#GAP:0\n: 0 4 0 a\n: 4 4 -12 b\n: 8 4 12 c\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert_eq!(track.notes[0].midi_pitch, 60);
        assert_eq!(track.notes[1].midi_pitch, 48);
        assert_eq!(track.notes[2].midi_pitch, 72);
    }

    #[test]
    fn handles_utf8_bom() {
        // 加 UTF-8 BOM 前綴
        let mut bytes: Vec<u8> = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(MINIMAL.as_bytes());
        let decoded = decode_text(&bytes);
        assert!(!decoded.starts_with('\u{FEFF}'));
        let track = parse_ultrastar(&decoded, "t.txt").unwrap();
        assert_eq!(track.notes.len(), 2);
    }

    #[test]
    fn rejects_empty_or_invalid() {
        assert!(parse_ultrastar("", "t.txt").is_err());
        // 有 BPM 沒有音符
        assert!(parse_ultrastar("#BPM:120\nE\n", "t.txt").is_err());
        // 有音符沒有 BPM
        assert!(parse_ultrastar(": 0 4 0 a\nE\n", "t.txt").is_err());
    }

    #[test]
    fn melody_track_to_pitch_track_dense_samples() {
        // BPM 60、一個音符 length=4（1 秒）、pitch 0（C4 = 261.626 Hz）
        let src = "#BPM:60\n#GAP:0\n: 0 4 0 a\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        let pitch = track.to_pitch_track(0.05);
        // 1 秒 / 0.05 = 20 個樣本
        assert_eq!(pitch.samples.len(), 20);
        // 頻率應該是 C4 ≈ 261.626 Hz
        for s in &pitch.samples {
            assert!((s.freq - 261.6255653).abs() < 0.01);
        }
    }

    #[test]
    fn parses_rap_notes_as_normal() {
        let src = "#BPM:60\n#GAP:0\nR 0 4 0 yo\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        assert_eq!(track.notes.len(), 1);
        // Rap 視為一般音符：既非 golden 也非 freestyle
        assert!(!track.notes[0].is_golden);
        assert!(!track.notes[0].is_freestyle);
    }

    #[test]
    fn parses_bpm_with_comma_decimal() {
        // 歐洲歌曲包常用逗號當小數點
        let src = "#BPM:120,5\n#GAP:0\n: 0 4 0 a\nE\n";
        let track = parse_ultrastar(src, "t.txt").unwrap();
        match &track.source {
            MelodySource::UltraStar { bpm, .. } => {
                assert!((bpm - 120.5).abs() < 1e-3);
            }
            _ => panic!("expected UltraStar"),
        }
    }
}
