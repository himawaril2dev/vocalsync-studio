//! 目標旋律軌（Melody Track）
//!
//! 統一三種音符來源的共用資料模型：
//! - UltraStar `.txt`（人工標注的卡拉 OK 歌曲包）
//! - MIDI `.mid`（結構化音符事件）
//! - 本地人聲分離（UVR5 模式，cache 為 `.vsmelody.json`）
//!
//! Phase 1 只實作 UltraStar 路徑；MIDI 與分離模式在 Phase 2 / 3 加入，
//! 但共用的 [`MelodyTrack`] 結構在 Phase 1 就定稿，後續只加 source 變體。

use crate::core::pitch_data::{freq_to_note, midi_to_freq, PitchSample, PitchTrack};
use serde::{Deserialize, Serialize};

/// 單一音符（discrete note）。
///
/// 一個 MelodyNote 代表一個有明確起訖時間與音高的音符，
/// 對應 UltraStar 裡的一行 `: start length pitch syllable`、
/// MIDI 的一對 note-on / note-off，
/// 或人聲分離後群聚出的連續穩定音高段。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MelodyNote {
    /// 音符起始時間（秒，從歌曲開頭起算）
    pub start_secs: f64,
    /// 音符持續時間（秒）
    pub duration_secs: f64,
    /// MIDI 音高編號（60 = C4）
    pub midi_pitch: u8,
    /// 預先算好的頻率（Hz），避免前端重複做 midi_to_freq
    pub freq_hz: f64,
    /// 對應的歌詞音節（UltraStar 才有）
    pub lyric: Option<String>,
    /// 黃金音符（UltraStar `*` 開頭，得分加倍）
    pub is_golden: bool,
    /// 自由音符（UltraStar `F` 開頭，任意音高都算對）
    pub is_freestyle: bool,
}

impl MelodyNote {
    /// 從 MIDI 音高建立音符並自動計算 freq_hz。
    pub fn from_midi(
        start_secs: f64,
        duration_secs: f64,
        midi_pitch: u8,
        lyric: Option<String>,
        is_golden: bool,
        is_freestyle: bool,
    ) -> Self {
        Self {
            start_secs,
            duration_secs,
            midi_pitch,
            freq_hz: midi_to_freq(midi_pitch as f64),
            lyric,
            is_golden,
            is_freestyle,
        }
    }

    /// 音符結束時間（秒）
    pub fn end_secs(&self) -> f64 {
        self.start_secs + self.duration_secs
    }
}

/// Melody 來源元資料（各來源的 provenance 與檔案路徑）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MelodySource {
    UltraStar {
        txt_path: String,
        title: Option<String>,
        artist: Option<String>,
        bpm: f32,
    },
    Midi {
        mid_path: String,
        track_index: usize,
        track_name: Option<String>,
    },
    VocalSeparation {
        cache_path: String,
        /// 模型名稱，例如 "htdemucs"
        model: String,
        /// 原始檔案 SHA-256，用於 cache 失效偵測
        file_hash: String,
    },
    /// 使用者用外部工具（UVR5 / Moises / Demucs CLI）分離好的人聲軌，
    /// 由本專案的 YIN 分析器提取 pitch 並群聚成 MelodyNote。
    ImportedVocals {
        vocals_path: String,
        /// 偵測到的音符數（用於 UI 顯示與 sanity check）
        note_count: usize,
        /// YIN 偵測的 voiced frame 比例，作為信心指標
        voiced_ratio: f64,
    },
}

/// 完整 melody 軌（給前端的最終單位）。
///
/// 不論來源是 UltraStar / MIDI / 分離，前端都只看到這個結構。
/// Phase 1 透過 [`MelodyTrack::to_pitch_track`] 轉換成 PitchTrack，
/// 讓既有的 PitchTimeline `drawSegmentedLine` 直接消費，不改 UI。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MelodyTrack {
    pub source: MelodySource,
    pub notes: Vec<MelodyNote>,
    pub total_duration_secs: f64,
    /// AI 引擎（CREPE）的原始音高樣本，保留自然的音高曲線。
    /// 若有值，`to_pitch_track` 會直接回傳這些樣本，跳過離散音符展開。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_pitch_track: Option<Vec<PitchSample>>,
}

impl MelodyTrack {
    /// 把離散音符展開成 PitchTrack 的密集樣本。
    ///
    /// 策略：每個音符按 `hop_secs` 間隔產生樣本，音符之間的 rest
    /// 區段不產生樣本。PitchTimeline 內建的「gap > 0.3s 斷段」邏輯
    /// 會自然把音符之間斷開成獨立線段。
    ///
    /// `hop_secs` 建議值：0.05（50 ms），對 drawSegmentedLine 的
    /// quadratic bezier 平滑演算法足夠細緻。
    pub fn to_pitch_track(&self, hop_secs: f64) -> PitchTrack {
        assert!(hop_secs > 0.0, "hop_secs must be positive");

        // 若有原始 AI 音高樣本，直接回傳（保留自然曲線）
        if let Some(raw) = &self.raw_pitch_track {
            return PitchTrack {
                samples: raw.clone(),
            };
        }

        let mut samples: Vec<PitchSample> = Vec::new();

        for note in &self.notes {
            // 自由音符沒有固定音高，不產生樣本（畫布上留空）
            if note.is_freestyle {
                continue;
            }

            let mut t = note.start_secs;
            let end = note.end_secs();

            // 至少產生一個樣本，避免極短音符（< hop_secs）消失
            let mut emitted = false;
            while t < end {
                let (note_name, octave, cent) = freq_to_note(note.freq_hz);
                samples.push(PitchSample {
                    timestamp: t,
                    freq: note.freq_hz,
                    confidence: 1.0,
                    note: note_name,
                    octave,
                    cent,
                });
                t += hop_secs;
                emitted = true;
            }

            // 保底：如果音符過短沒進入迴圈，補一個樣本在起點
            if !emitted {
                let (note_name, octave, cent) = freq_to_note(note.freq_hz);
                samples.push(PitchSample {
                    timestamp: note.start_secs,
                    freq: note.freq_hz,
                    confidence: 1.0,
                    note: note_name,
                    octave,
                    cent,
                });
            }
        }

        PitchTrack { samples }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_note(start: f64, dur: f64, midi: u8) -> MelodyNote {
        MelodyNote::from_midi(start, dur, midi, None, false, false)
    }

    #[test]
    fn from_midi_computes_freq_hz_correctly() {
        // MIDI 69 = A4 = 440 Hz
        let note = MelodyNote::from_midi(0.0, 1.0, 69, None, false, false);
        assert!((note.freq_hz - 440.0).abs() < 1e-6);
        // MIDI 60 = C4 ≈ 261.626 Hz
        let c4 = MelodyNote::from_midi(0.0, 1.0, 60, None, false, false);
        assert!((c4.freq_hz - 261.6255653).abs() < 1e-3);
    }

    #[test]
    fn end_secs_is_start_plus_duration() {
        let note = make_note(1.5, 0.25, 60);
        assert!((note.end_secs() - 1.75).abs() < 1e-9);
    }

    #[test]
    fn to_pitch_track_produces_dense_samples() {
        // 單一音符 C4，持續 0.2 秒，hop 0.05 → 預期 4 個樣本
        let track = MelodyTrack {
            source: MelodySource::UltraStar {
                txt_path: "dummy.txt".to_string(),
                title: None,
                artist: None,
                bpm: 120.0,
            },
            notes: vec![make_note(0.0, 0.2, 60)],
            total_duration_secs: 0.2,
            raw_pitch_track: None,
        };

        let pitch = track.to_pitch_track(0.05);
        assert_eq!(pitch.samples.len(), 4);
        assert!((pitch.samples[0].timestamp - 0.0).abs() < 1e-9);
        assert!((pitch.samples[1].timestamp - 0.05).abs() < 1e-9);
        // 頻率應該全都是 C4
        let c4 = midi_to_freq(60.0);
        for s in &pitch.samples {
            assert!((s.freq - c4).abs() < 1e-6);
            assert_eq!(s.confidence, 1.0);
        }
    }

    #[test]
    fn to_pitch_track_skips_freestyle_notes() {
        let freestyle = MelodyNote::from_midi(0.0, 1.0, 60, None, false, true);
        let normal = MelodyNote::from_midi(1.0, 0.1, 62, None, false, false);
        let track = MelodyTrack {
            source: MelodySource::UltraStar {
                txt_path: "dummy.txt".to_string(),
                title: None,
                artist: None,
                bpm: 120.0,
            },
            notes: vec![freestyle, normal],
            total_duration_secs: 1.1,
            raw_pitch_track: None,
        };
        let pitch = track.to_pitch_track(0.05);
        // freestyle 不產樣本；normal 0.1s / 0.05 = 2 個樣本
        assert_eq!(pitch.samples.len(), 2);
        assert!((pitch.samples[0].timestamp - 1.0).abs() < 1e-9);
    }

    #[test]
    fn to_pitch_track_emits_at_least_one_sample_for_short_notes() {
        // 持續時間 0.01 秒 < hop 0.05，應該至少有一個樣本
        let track = MelodyTrack {
            source: MelodySource::UltraStar {
                txt_path: "dummy.txt".to_string(),
                title: None,
                artist: None,
                bpm: 120.0,
            },
            notes: vec![make_note(0.5, 0.01, 60)],
            total_duration_secs: 0.51,
            raw_pitch_track: None,
        };
        let pitch = track.to_pitch_track(0.05);
        assert_eq!(pitch.samples.len(), 1);
        assert!((pitch.samples[0].timestamp - 0.5).abs() < 1e-9);
    }
}
