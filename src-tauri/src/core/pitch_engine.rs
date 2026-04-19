//! YIN 音高偵測演算法（純 Rust 實作）
//!
//! 參考論文：de Cheveigné & Kawahara (2002), "YIN, a fundamental frequency
//! estimator for speech and music"
//!
//! 對人聲特別有效。對單純樂器（無 vibrato）也能良好工作。
//!
//! 設計重點：
//! - 純粹 O(N²) 暴力差分（N=2048 時約 4M ops/frame，<1ms 在現代 CPU）
//! - 不需要 FFT，無外部依賴
//! - RMS gate 預過濾靜音
//! - 拋物線插值取得 sub-sample 精度

use crate::core::pitch_data::{freq_to_note, PitchSample};

pub struct PitchDetector {
    sample_rate: u32,
    /// 偵測視窗大小（typically 2048）
    buf_size: usize,
    /// 對應 f_max 的最小週期（樣本數）
    tau_min: usize,
    /// 對應 f_min 的最大週期（樣本數）
    tau_max: usize,
    /// CMNDF 閾值（典型 0.10 ~ 0.20，越低越嚴格）
    harmonic_threshold: f64,
    /// RMS 靜音閾值
    rms_threshold: f64,
    /// 工作緩衝區（避免每次配置）
    diff_buf: Vec<f64>,
}

impl PitchDetector {
    /// 建立偵測器
    ///
    /// - `sample_rate`：取樣率
    /// - `buf_size`：視窗大小（建議 2048，越大越準確但延遲越高）
    /// - `f_min`：偵測下限頻率（人聲建議 50Hz）
    /// - `f_max`：偵測上限頻率（人聲建議 1000Hz）
    /// - `harmonic_threshold`：CMNDF 閾值（人聲建議 0.15）
    /// - `rms_threshold`：靜音 RMS 閾值（建議 0.01）
    pub fn new(
        sample_rate: u32,
        buf_size: usize,
        f_min: f64,
        f_max: f64,
        harmonic_threshold: f64,
        rms_threshold: f64,
    ) -> Self {
        let sr = sample_rate as f64;
        let tau_min = (sr / f_max).floor() as usize;
        let tau_max = (sr / f_min).ceil() as usize;
        let tau_max = tau_max.min(buf_size / 2);

        Self {
            sample_rate,
            buf_size,
            tau_min: tau_min.max(2),
            tau_max,
            harmonic_threshold,
            rms_threshold,
            diff_buf: vec![0.0; buf_size / 2 + 1],
        }
    }

    /// 偵測單一視窗的音高
    ///
    /// - `samples`：長度應 == buf_size 的單聲道音訊（f32）
    /// - `timestamp`：時間戳（秒），會寫入回傳的 PitchSample
    ///
    /// 回傳 None 代表：靜音、無法偵測、或信心不足
    pub fn detect(&mut self, samples: &[f32], timestamp: f64) -> Option<PitchSample> {
        if samples.len() < self.buf_size {
            return None;
        }

        // ── Step 1：RMS 靜音過濾 ──
        let rms = calc_rms(&samples[..self.buf_size]);
        if rms < self.rms_threshold {
            return None;
        }

        // ── Step 2：差分函數 d(τ) ──
        // d(τ) = Σ_{i=0..W} (x[i] - x[i+τ])²
        // W = buf_size / 2，τ ∈ [1, buf_size/2]
        let half = self.buf_size / 2;
        for tau in 1..=half {
            let mut sum = 0.0_f64;
            for i in 0..half {
                let diff = samples[i] as f64 - samples[i + tau] as f64;
                sum += diff * diff;
            }
            self.diff_buf[tau] = sum;
        }

        // ── Step 3：CMNDF（累積平均正規化差分函數）──
        // d'(τ) = d(τ) / [(1/τ) Σ_{j=1..τ} d(j)]
        self.diff_buf[0] = 1.0; // d'(0) = 1 by definition
        let mut running_sum = 0.0_f64;
        for tau in 1..=half {
            running_sum += self.diff_buf[tau];
            if running_sum > 0.0 {
                self.diff_buf[tau] = self.diff_buf[tau] * (tau as f64) / running_sum;
            } else {
                self.diff_buf[tau] = 1.0;
            }
        }

        // ── Step 4：絕對閾值搜尋 ──
        // 找第一個低於閾值的 τ，再向下找局部最小
        let mut tau_estimate = 0_usize;
        let mut tau = self.tau_min;
        while tau < self.tau_max {
            if self.diff_buf[tau] < self.harmonic_threshold {
                // 跟著向下找直到不再下降，取局部最小
                while tau + 1 < self.tau_max && self.diff_buf[tau + 1] < self.diff_buf[tau] {
                    tau += 1;
                }
                tau_estimate = tau;
                break;
            }
            tau += 1;
        }

        if tau_estimate == 0 {
            return None;
        }

        // ── Step 5：拋物線插值（sub-sample 精度）──
        let better_tau = parabolic_interpolation(&self.diff_buf, tau_estimate);

        // ── Step 6：頻率與信心度 ──
        let freq = self.sample_rate as f64 / better_tau;
        let confidence = 1.0 - self.diff_buf[tau_estimate]; // 越接近 1 越可信

        // 過濾不合理的頻率
        if !freq.is_finite() || freq <= 0.0 {
            return None;
        }

        let (note, octave, cent) = freq_to_note(freq);

        Some(PitchSample {
            timestamp,
            freq,
            confidence,
            note,
            octave,
            cent,
        })
    }
}

// ── Helper 函式 ────────────────────────────────────────────────────

fn calc_rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum_sq = 0.0_f64;
    for &s in samples {
        sum_sq += (s as f64) * (s as f64);
    }
    (sum_sq / samples.len() as f64).sqrt()
}

/// 拋物線插值：在 d_buf[tau-1], d_buf[tau], d_buf[tau+1] 三點擬合拋物線，
/// 回傳極值對應的 sub-sample tau
fn parabolic_interpolation(d_buf: &[f64], tau: usize) -> f64 {
    if tau == 0 || tau + 1 >= d_buf.len() {
        return tau as f64;
    }
    let s0 = d_buf[tau - 1];
    let s1 = d_buf[tau];
    let s2 = d_buf[tau + 1];
    let denom = 2.0 * (2.0 * s1 - s2 - s0);
    if denom.abs() < 1e-9 {
        return tau as f64;
    }
    tau as f64 + (s2 - s0) / denom
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// 產生純正弦波測試信號
    fn sine_wave(freq: f64, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32 * 0.5
            })
            .collect()
    }

    #[test]
    fn detects_440hz_a4() {
        let mut det = PitchDetector::new(44100, 2048, 50.0, 1000.0, 0.15, 0.01);
        let samples = sine_wave(440.0, 44100, 2048);
        let result = det.detect(&samples, 0.0).expect("應該偵測到");
        assert!((result.freq - 440.0).abs() < 1.0);
        assert_eq!(result.note, "A");
        assert_eq!(result.octave, 4);
    }

    #[test]
    fn detects_220hz_a3() {
        let mut det = PitchDetector::new(44100, 2048, 50.0, 1000.0, 0.15, 0.01);
        let samples = sine_wave(220.0, 44100, 2048);
        let result = det.detect(&samples, 0.0).expect("應該偵測到");
        assert!((result.freq - 220.0).abs() < 1.0);
        assert_eq!(result.note, "A");
        assert_eq!(result.octave, 3);
    }

    #[test]
    fn rejects_silence() {
        let mut det = PitchDetector::new(44100, 2048, 50.0, 1000.0, 0.15, 0.01);
        let silence = vec![0.0_f32; 2048];
        assert!(det.detect(&silence, 0.0).is_none());
    }

    #[test]
    fn detects_c5_523hz() {
        let mut det = PitchDetector::new(44100, 2048, 50.0, 1000.0, 0.15, 0.01);
        let samples = sine_wave(523.25, 44100, 2048);
        let result = det.detect(&samples, 0.0).expect("應該偵測到");
        assert!((result.freq - 523.25).abs() < 2.0);
        assert_eq!(result.note, "C");
        assert_eq!(result.octave, 5);
    }
}
