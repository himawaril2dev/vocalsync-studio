//! 音高資料結構（移植自 Python 版 pitch_data.py）

use serde::{Deserialize, Serialize};

/// 單一音高樣本
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchSample {
    pub timestamp: f64,
    pub freq: f64,
    pub confidence: f64,
    pub note: String,
    pub octave: i32,
    pub cent: f64,
}

/// 音高軌跡（一系列樣本）
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PitchTrack {
    pub samples: Vec<PitchSample>,
}

impl PitchTrack {
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    pub fn append(&mut self, sample: PitchSample) {
        self.samples.push(sample);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// 保留 timestamp < `cutoff_secs` 的樣本，捨棄之後的。
    /// 用於續錄向前 seek 時同步截斷 pitch 軌跡，避免後段殘留。
    pub fn truncate_after(&mut self, cutoff_secs: f64) {
        self.samples.retain(|s| s.timestamp < cutoff_secs);
    }
}

/// MIDI 音符編號轉頻率
pub fn midi_to_freq(midi: f64) -> f64 {
    440.0 * 2.0_f64.powf((midi - 69.0) / 12.0)
}

/// 頻率轉 MIDI 音符編號
pub fn freq_to_midi(freq: f64) -> f64 {
    if freq <= 0.0 {
        return 0.0;
    }
    69.0 + 12.0 * (freq / 440.0).log2()
}

/// 頻率轉音符名稱 + 八度
pub fn freq_to_note(freq: f64) -> (String, i32, f64) {
    let midi = freq_to_midi(freq);
    let note_names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let rounded = midi.round() as i32;
    let note_idx = ((rounded % 12) + 12) % 12;
    let octave = (rounded / 12) - 1;
    let cent = (midi - rounded as f64) * 100.0;
    (note_names[note_idx as usize].to_string(), octave, cent)
}
