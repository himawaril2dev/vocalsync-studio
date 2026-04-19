//! 調性偵測引擎（Chroma + Krumhansl-Schmuckler 演算法）
//!
//! 從 PitchTrack（一系列 PitchSample）中提取 chroma histogram，
//! 再用 Krumhansl-Schmuckler key profiles 比對 24 個調性（12 大調 + 12 小調），
//! 找出相關性最高的調性。
//!
//! # 原理
//!
//! 1. **Chroma histogram**：將每個偵測到的頻率映射到 12 個半音
//!    （C, C#, D, ..., B），以 confidence × duration 作為權重累加。
//!
//! 2. **Krumhansl-Schmuckler profiles**：認知心理學實驗得出的「穩定感」分佈，
//!    大調和小調各有一組 12 維向量。將 chroma histogram 旋轉 12 次，
//!    分別與大調/小調 profile 計算 Pearson 相關係數，最高者即為偵測結果。
//!
//! # 參考
//!
//! - Krumhansl, C.L. (1990). *Cognitive Foundations of Musical Pitch*.
//! - Temperley, D. (1999). *What's Key for Key?*

use crate::core::pitch_data::{freq_to_midi, PitchSample, PitchTrack};
use serde::{Deserialize, Serialize};

// ── 常數 ─────────────────────────────────────────────────────────

/// 12 個音名
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Krumhansl-Schmuckler 大調 profile（從 C 大調開始）
/// 來源：Krumhansl & Kessler (1982), Table 2
const MAJOR_PROFILE: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

/// Krumhansl-Schmuckler 小調 profile（從 C 小調開始）
const MINOR_PROFILE: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

// ── 型別 ─────────────────────────────────────────────────────────

/// 調性偵測結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyResult {
    /// 調性名稱，如 "C major", "A minor"
    pub key: String,
    /// 音名（0=C, 1=C#, ..., 11=B）
    pub tonic: u8,
    /// 大調 = "major", 小調 = "minor"
    pub mode: String,
    /// Pearson 相關係數（-1.0 ~ 1.0，越高越有信心）
    pub correlation: f64,
    /// 所有 24 個調性的相關係數（debug 用）
    pub all_correlations: Vec<KeyCorrelation>,
    /// chroma histogram（12 個半音的權重分佈）
    pub chroma: [f64; 12],
    /// 用了多少個有效 PitchSample
    pub sample_count: usize,
}

/// 單一調性的相關係數
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyCorrelation {
    pub key: String,
    pub correlation: f64,
}

// ── 公開 API ─────────────────────────────────────────────────────

/// 從 PitchTrack 偵測調性。
///
/// 需要至少 30 個有效 PitchSample 才能給出有意義的結果。
pub fn detect_key(track: &PitchTrack) -> Option<KeyResult> {
    detect_key_from_samples(&track.samples)
}

/// 從 PitchSample slice 偵測調性。
pub fn detect_key_from_samples(samples: &[PitchSample]) -> Option<KeyResult> {
    // 至少需要 30 個樣本
    if samples.len() < 30 {
        return None;
    }

    // 1. 建立 chroma histogram
    let chroma = build_chroma_histogram(samples);

    // 如果 chroma 全為 0（沒有有效 sample），回傳 None
    let total_weight: f64 = chroma.iter().sum();
    if total_weight < 1e-6 {
        return None;
    }

    // 2. 對 24 個調性計算 Pearson 相關係數
    let mut all_correlations = Vec::with_capacity(24);
    let mut best_key = String::new();
    let mut best_tonic: u8 = 0;
    let mut best_mode = String::new();
    let mut best_corr = f64::NEG_INFINITY;

    for shift in 0..12_u8 {
        let rotated = rotate_chroma(&chroma, shift as usize);

        // 大調
        let corr_major = pearson_correlation(&rotated, &MAJOR_PROFILE);
        let key_name = format!("{} major", NOTE_NAMES[shift as usize]);
        all_correlations.push(KeyCorrelation {
            key: key_name.clone(),
            correlation: corr_major,
        });
        if corr_major > best_corr {
            best_corr = corr_major;
            best_key = key_name;
            best_tonic = shift;
            best_mode = "major".to_string();
        }

        // 小調
        let corr_minor = pearson_correlation(&rotated, &MINOR_PROFILE);
        let key_name = format!("{} minor", NOTE_NAMES[shift as usize]);
        all_correlations.push(KeyCorrelation {
            key: key_name.clone(),
            correlation: corr_minor,
        });
        if corr_minor > best_corr {
            best_corr = corr_minor;
            best_key = key_name;
            best_tonic = shift;
            best_mode = "minor".to_string();
        }
    }

    // 按相關係數降序排列
    all_correlations.sort_by(|a, b| b.correlation.partial_cmp(&a.correlation).unwrap());

    Some(KeyResult {
        key: best_key,
        tonic: best_tonic,
        mode: best_mode,
        correlation: best_corr,
        all_correlations,
        chroma,
        sample_count: samples.len(),
    })
}

// ── 內部函式 ─────────────────────────────────────────────────────

/// 從 PitchSample 建立 chroma histogram。
///
/// 每個 sample 的頻率轉為 MIDI → 取 mod 12 得到 pitch class，
/// 以 confidence 作為權重累加（confidence 越高的 sample 對調性判斷貢獻越大）。
fn build_chroma_histogram(samples: &[PitchSample]) -> [f64; 12] {
    let mut chroma = [0.0_f64; 12];

    for (i, sample) in samples.iter().enumerate() {
        if sample.freq <= 0.0 || sample.confidence <= 0.0 {
            continue;
        }

        let midi = freq_to_midi(sample.freq);
        if midi < 0.0 || midi > 127.0 {
            continue;
        }

        // pitch class = MIDI mod 12（四捨五入到最近的半音）
        let pc = ((midi.round() as i32 % 12) + 12) % 12;

        // 權重 = confidence × duration_weight
        // duration_weight：用相鄰 sample 的時間差估算
        let duration_weight = if i > 0 && i + 1 < samples.len() {
            let dt_prev = sample.timestamp - samples[i - 1].timestamp;
            let dt_next = samples[i + 1].timestamp - sample.timestamp;
            ((dt_prev + dt_next) / 2.0).clamp(0.001, 0.5)
        } else {
            0.01 // 邊界 sample 給最小權重
        };

        chroma[pc as usize] += sample.confidence * duration_weight;
    }

    chroma
}

/// 旋轉 chroma histogram：shift=0 保持原樣，shift=1 往右移一格（C→C#）。
///
/// 這等同於「假設 tonic 在 NOTE_NAMES[shift] 上」。
fn rotate_chroma(chroma: &[f64; 12], shift: usize) -> [f64; 12] {
    let mut rotated = [0.0; 12];
    for (i, &val) in chroma.iter().enumerate() {
        let new_idx = (i + 12 - shift) % 12;
        rotated[new_idx] = val;
    }
    rotated
}

/// Pearson 相關係數（兩個長度 12 的向量）。
///
/// r = Σ((x_i - x̄)(y_i - ȳ)) / sqrt(Σ(x_i - x̄)² × Σ(y_i - ȳ)²)
fn pearson_correlation(x: &[f64; 12], y: &[f64; 12]) -> f64 {
    let n = 12.0;
    let mean_x: f64 = x.iter().sum::<f64>() / n;
    let mean_y: f64 = y.iter().sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;

    for i in 0..12 {
        let dx = x[i] - mean_x;
        let dy = y[i] - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }

    let denom = (var_x * var_y).sqrt();
    if denom < 1e-12 {
        return 0.0;
    }

    cov / denom
}

// ── 測試 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pitch_data::freq_to_note;

    /// 建立測試用的 PitchSample
    fn make_sample(freq: f64, timestamp: f64, confidence: f64) -> PitchSample {
        let (note, octave, cent) = freq_to_note(freq);
        PitchSample {
            timestamp,
            freq,
            confidence,
            note,
            octave,
            cent,
        }
    }

    #[test]
    fn detects_c_major_from_scale() {
        // C 大調音階：C4 D4 E4 F4 G4 A4 B4
        let freqs = [261.63, 293.66, 329.63, 349.23, 392.00, 440.00, 493.88];
        let mut samples = Vec::new();
        for (i, &freq) in freqs.iter().enumerate() {
            // 每個音符持續 0.5 秒
            for j in 0..50 {
                let t = (i * 50 + j) as f64 * 0.01;
                samples.push(make_sample(freq, t, 0.9));
            }
        }

        let result = detect_key_from_samples(&samples);
        assert!(result.is_some(), "應該能偵測到調性");

        let result = result.unwrap();
        // C major 應該排名前幾
        assert!(
            result.key == "C major" || result.all_correlations[0].key == "C major",
            "預期 C major，得到 {}（top: {}）",
            result.key,
            result.all_correlations[0].key
        );
    }

    #[test]
    fn detects_a_minor_from_scale() {
        // A 小調音階：A4 B4 C5 D5 E5 F5 G5
        let freqs = [440.00, 493.88, 523.25, 587.33, 659.26, 698.46, 783.99];
        let mut samples = Vec::new();
        for (i, &freq) in freqs.iter().enumerate() {
            for j in 0..50 {
                let t = (i * 50 + j) as f64 * 0.01;
                samples.push(make_sample(freq, t, 0.9));
            }
        }

        let result = detect_key_from_samples(&samples);
        assert!(result.is_some());

        let result = result.unwrap();
        // A minor 應該有很高的相關係數
        let a_minor_corr = result
            .all_correlations
            .iter()
            .find(|c| c.key == "A minor")
            .map(|c| c.correlation)
            .unwrap_or(0.0);
        assert!(
            a_minor_corr > 0.5,
            "A minor 相關係數應 > 0.5，得到 {}",
            a_minor_corr
        );
    }

    #[test]
    fn returns_none_for_too_few_samples() {
        let samples: Vec<PitchSample> = (0..10)
            .map(|i| make_sample(440.0, i as f64 * 0.01, 0.8))
            .collect();
        assert!(detect_key_from_samples(&samples).is_none());
    }

    #[test]
    fn pearson_perfect_correlation() {
        let x = [
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
        ];
        let y = [
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
        ];
        let r = pearson_correlation(&x, &y);
        assert!((r - 1.0).abs() < 1e-10, "完美正相關應為 1.0，得到 {}", r);
    }

    #[test]
    fn chroma_rotation_preserves_sum() {
        let chroma = [
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
        ];
        let original_sum: f64 = chroma.iter().sum();
        for shift in 0..12 {
            let rotated = rotate_chroma(&chroma, shift);
            let rotated_sum: f64 = rotated.iter().sum();
            assert!(
                (original_sum - rotated_sum).abs() < 1e-10,
                "旋轉後總和應不變"
            );
        }
    }
}
