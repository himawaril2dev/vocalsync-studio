//! 音訊自動對齊（FFT-based cross-correlation）
//!
//! # 解決的問題
//!
//! 使用者從「原曲」提取 melody pitch 時間軸，但練唱時播放的是「伴奏版」
//! (off vocal / instrumental)。這兩個檔案**幾乎不會有相同的 t=0**：
//!
//! - 原曲可能有 3 秒 intro silence，伴奏版剪掉了
//! - 伴奏版可能加了 4 拍 count-in
//! - 兩者的 intro 長度可能因為 remaster 不同
//!
//! 如果不對齊，melody 線會在 PitchTimeline 上飄掉，產品體驗直接崩潰。
//!
//! # 演算法
//!
//! 經典的 FFT-accelerated cross-correlation：
//!
//! ```text
//! 1. 兩個檔案各自：
//!    a. symphonia 解碼成 interleaved f32 stereo
//!    b. down-mix 成 mono (L+R)/2
//!    c. boxcar down-sample 到 ~11 kHz（加速計算，精度仍足夠）
//! 2. 計算線性 cross-correlation：
//!    a. zero-pad 兩段到 next_power_of_2(N + M)
//!    b. FFT(a), FFT(b)
//!    c. A * conj(B) element-wise
//!    d. IFFT → correlation 函數
//! 3. 找最大 peak 的 index → unwrap 成 lag samples
//! 4. 換算成秒 + 回傳對齊資訊
//! ```
//!
//! # 符號定義（很重要，不要搞混）
//!
//! `AlignmentResult::offset_secs` 代表「**target 的 t=0 對應到 reference 的哪個時間位置**」：
//!
//! - **正值**：target 跳過了 reference 前 `offset_secs` 秒的內容
//!   - 典型情境：原曲（reference）有 3 秒 intro silence，伴奏版（target）剪掉了
//!   - `offset_secs = +3.0` → target 的開頭對應原曲的 3 秒處
//!
//! - **負值**：target 多了 reference 沒有的開頭（count-in / silence padding）
//!   - 典型情境：伴奏版加了 4 拍 count-in，原曲直接從音樂開始
//!   - `offset_secs = -2.5` → target 的開頭比原曲早了 2.5 秒
//!
//! # 套用到 melody track
//!
//! Melody track 的 timestamps 都是相對於 reference 的時間軸。實際播放時用
//! target，所以每個 timestamp 要這樣換算：
//!
//! ```text
//! melody_time_in_target = melody_time_in_reference - offset_secs
//! ```
//!
//! 例 1：reference 有 3 秒 intro 但 target 剪掉了 → `offset_secs = +3.0`
//!      melody 的 `t = 5.0`（相對原曲）→ target 播放 `5.0 - 3.0 = 2.0` 秒時應該開始唱
//!
//! 例 2：target 有 2 秒 count-in → `offset_secs = -2.0`
//!      melody 的 `t = 0.0`（原曲一開始就有人聲）→ target 播放到 `0.0 - (-2.0) = 2.0` 秒
//!      才開始唱
//!
//! # 精度
//!
//! Down-sample 到 11 kHz 後，單一 sample 的精度約 91 微秒，聽覺上完全可忽略。
//! 最大偵測範圍取決於檔案長度，對 4 分鐘歌曲可偵測 ±3 分鐘以上的位移。
//!
//! # 邊界條件（未處理）
//!
//! - BPM 不同（需要 time-stretching）
//! - 結構不同（例如伴奏版少了 8 小節 intro，需要 DTW）
//! - Key 偏移（需要 chroma 對齊）
//!
//! 這些 case 在 UI 層應顯示「信心度低，請手動微調」，並提供 offset slider。

use crate::core::media_loader::load_media;
use crate::core::resampler;
use crate::error::AppError;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use serde::Serialize;

/// 對齊計算時的目標取樣率（Hz）。降到這個值大幅加速 FFT，精度仍足夠。
const TARGET_SAMPLE_RATE: u32 = 11_025;
pub(crate) const ENVELOPE_SAMPLE_RATE: u32 = 100;
const ENVELOPE_WINDOW_MS: f32 = 30.0;
const ENVELOPE_HOP_MS: f32 = 10.0;

/// 對齊結果。
#[derive(Debug, Clone, Serialize)]
pub struct AlignmentResult {
    /// target 的 t=0 對應 reference 的時間位置（秒），詳見模組 doc。
    /// 套用到 melody：`aligned_time = melody_time - offset_secs`
    pub offset_secs: f64,
    /// 偵測到的最大 correlation 值，可作為信心指標（值越大越有信心）。
    pub peak_correlation: f32,
    /// Peak 相對於「平均 correlation」的倍數，> 5 算高信心，< 2 要警示。
    pub peak_to_mean_ratio: f32,
    /// 實際用於計算的有效取樣率（Hz）。
    pub sample_rate: u32,
    /// Reference 檔的原始秒數。
    pub reference_duration_secs: f64,
    /// Target 檔的原始秒數。
    pub target_duration_secs: f64,
}

/// 對齊兩個音訊檔案。
///
/// `reference_path` 通常是「原曲」（melody 抽取來源），
/// `target_path` 是「伴奏版」（實際播放的檔案）。
pub fn align_files(reference_path: &str, target_path: &str) -> Result<AlignmentResult, AppError> {
    let reference = load_and_prepare(reference_path)?;
    let target = load_and_prepare(target_path)?;

    let effective_sr = reference.sample_rate;
    if target.sample_rate != effective_sr {
        return Err(AppError::Audio(format!(
            "取樣率不一致：reference={} target={}（對齊前應重採樣）",
            effective_sr, target.sample_rate
        )));
    }

    let reference_wave = normalize_for_correlation(&reference.mono);
    let target_wave = normalize_for_correlation(&target.mono);
    let waveform = correlation_candidate(&reference_wave, &target_wave, effective_sr);

    let reference_envelope = build_alignment_envelope(&reference.mono, effective_sr);
    let target_envelope = build_alignment_envelope(&target.mono, effective_sr);
    let envelope =
        correlation_candidate(&reference_envelope, &target_envelope, ENVELOPE_SAMPLE_RATE);
    let selected = select_alignment_candidate(waveform, envelope);

    Ok(AlignmentResult {
        offset_secs: selected.lag_samples as f64 / selected.sample_rate as f64,
        peak_correlation: selected.peak,
        peak_to_mean_ratio: selected.peak_to_mean_ratio,
        sample_rate: selected.sample_rate,
        reference_duration_secs: reference.original_duration,
        target_duration_secs: target.original_duration,
    })
}

// ── 內部 helpers ──────────────────────────────────────────────────

struct PreparedAudio {
    mono: Vec<f32>,
    sample_rate: u32,
    original_duration: f64,
}

fn load_and_prepare(path: &str) -> Result<PreparedAudio, AppError> {
    let media = load_media(path)?;
    let mono = stereo_to_mono(&media.samples, media.channels as usize);
    let prepared = resampler::resample_offline(&mono, media.sample_rate, TARGET_SAMPLE_RATE);

    Ok(PreparedAudio {
        mono: prepared,
        sample_rate: TARGET_SAMPLE_RATE,
        original_duration: media.duration,
    })
}

#[derive(Debug, Clone, Copy)]
struct CorrelationCandidate {
    lag_samples: i64,
    peak: f32,
    peak_to_mean_ratio: f32,
    sample_rate: u32,
}

fn correlation_candidate(
    reference: &[f32],
    target: &[f32],
    sample_rate: u32,
) -> CorrelationCandidate {
    let (lag_samples, peak, mean_abs) = cross_correlate_peak(reference, target);
    let peak_to_mean_ratio = if mean_abs > f32::EPSILON {
        peak.abs() / mean_abs
    } else {
        0.0
    };
    CorrelationCandidate {
        lag_samples,
        peak,
        peak_to_mean_ratio,
        sample_rate,
    }
}

fn select_alignment_candidate(
    waveform: CorrelationCandidate,
    envelope: CorrelationCandidate,
) -> CorrelationCandidate {
    if waveform.peak_to_mean_ratio < 2.0
        && envelope.peak_to_mean_ratio > waveform.peak_to_mean_ratio
    {
        return envelope;
    }
    if envelope.peak_to_mean_ratio >= 2.0
        && envelope.peak_to_mean_ratio > waveform.peak_to_mean_ratio * 1.25
    {
        return envelope;
    }
    waveform
}

/// 把交錯樣本 down-mix 成 mono。支援任意聲道數 ≥ 1。
pub(crate) fn stereo_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Boxcar average down-sample：等同於簡易 low-pass + decimation，
/// 足以避免嚴重 aliasing 影響 correlation peak 清晰度。
#[cfg(test)]
fn downsample_boxcar(samples: &[f32], factor: usize) -> Vec<f32> {
    if factor <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks(factor)
        .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
        .collect()
}

fn normalize_for_correlation(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let centered: Vec<f32> = samples.iter().map(|sample| sample - mean).collect();
    let rms =
        (centered.iter().map(|sample| sample * sample).sum::<f32>() / centered.len() as f32).sqrt();
    if rms <= 1e-6 {
        return centered;
    }
    centered
        .into_iter()
        .map(|sample| (sample / rms).clamp(-6.0, 6.0))
        .collect()
}

pub(crate) fn build_alignment_envelope(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let window =
        ((sample_rate.max(1) as f32 * ENVELOPE_WINDOW_MS / 1000.0).round() as usize).max(1);
    let hop = ((sample_rate.max(1) as f32 * ENVELOPE_HOP_MS / 1000.0).round() as usize).max(1);
    let mut energy = Vec::with_capacity(samples.len() / hop + 1);
    let mut start = 0;
    while start < samples.len() {
        let end = (start + window).min(samples.len());
        let sum_sq = samples[start..end]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>();
        energy.push((sum_sq / (end - start).max(1) as f32).sqrt());
        start += hop;
    }

    normalize_unit(&mut energy);
    let mut smoothed = 0.0_f32;
    let mut onset = Vec::with_capacity(energy.len());
    for &value in energy.iter() {
        smoothed = smoothed * 0.86 + value * 0.14;
        onset.push((value - smoothed).max(0.0));
    }
    normalize_unit(&mut onset);

    energy
        .iter()
        .zip(onset.iter())
        .map(|(&e, &o)| e * 0.38 + o * 0.72)
        .collect()
}

fn normalize_unit(values: &mut [f32]) {
    if values.is_empty() {
        return;
    }
    let floor = percentile(values, 0.20);
    let high = percentile(values, 0.95);
    let scale = (high - floor).max(1e-6);
    for value in values {
        *value = ((*value - floor) / scale).clamp(0.0, 1.0).sqrt();
    }
}

fn percentile(values: &[f32], percentile: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let index =
        ((sorted.len().saturating_sub(1)) as f32 * percentile.clamp(0.0, 1.0)).round() as usize;
    sorted[index.min(sorted.len().saturating_sub(1))]
}

/// 計算 `reference` 與 `target` 的 FFT-based linear cross-correlation，
/// 回傳 `(lag_samples, peak_value, mean_abs_correlation)`。
///
/// `lag_samples` 的符號遵循模組 doc：正值代表 reference 需要「延遲」
/// lag 個 sample 才對齊 target（即 reference 比 target 早開始）。
fn cross_correlate_peak(reference: &[f32], target: &[f32]) -> (i64, f32, f32) {
    let n = reference.len();
    let m = target.len();
    if n == 0 || m == 0 {
        return (0, 0.0, 0.0);
    }

    // Linear correlation 長度 = n + m - 1，補到 2 的冪次加速 FFT
    let fft_len = (n + m).next_power_of_two();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_len);
    let ifft = planner.plan_fft_inverse(fft_len);

    // Zero-pad reference
    let mut ref_buf: Vec<Complex<f32>> = Vec::with_capacity(fft_len);
    ref_buf.extend(reference.iter().map(|&x| Complex::new(x, 0.0)));
    ref_buf.resize(fft_len, Complex::new(0.0, 0.0));

    // Zero-pad target
    let mut tgt_buf: Vec<Complex<f32>> = Vec::with_capacity(fft_len);
    tgt_buf.extend(target.iter().map(|&x| Complex::new(x, 0.0)));
    tgt_buf.resize(fft_len, Complex::new(0.0, 0.0));

    fft.process(&mut ref_buf);
    fft.process(&mut tgt_buf);

    // Element-wise: REF * conj(TGT)
    // 這樣得到的 inverse FFT 是 cross_corr[k] = sum_i ref[i] * target[i - k]
    // 其中 k 的正方向表示 target 領先 reference（reference 比 target 晚）
    // 但我們慣例是「reference 比 target 早」為正，所以符號要翻。
    for (r, t) in ref_buf.iter_mut().zip(tgt_buf.iter()) {
        *r *= t.conj();
    }

    ifft.process(&mut ref_buf);

    // rustfft IFFT 不做 1/N 正規化，對找 peak 無影響

    // Cyclic 結果的 unwrap：
    //   index k ∈ [0, n)      → lag = k    （reference 領先 target）
    //   index k ∈ (fft_len-m, fft_len)
    //                          → lag = k - fft_len （reference 落後 target）
    // 我們搜尋整個 cyclic 範圍，找最大絕對值 peak。
    let mut peak_idx: usize = 0;
    let mut peak_val: f32 = 0.0;
    let mut peak_abs: f32 = f32::NEG_INFINITY;
    let mut sum_abs: f64 = 0.0;
    for (i, c) in ref_buf.iter().enumerate() {
        let val = c.re;
        sum_abs += val.abs() as f64;
        if val.abs() > peak_abs {
            peak_val = val;
            peak_abs = val.abs();
            peak_idx = i;
        }
    }
    let mean_abs = (sum_abs / fft_len as f64) as f32;

    // Unwrap cyclic index → signed lag
    let lag_samples: i64 = if peak_idx < n {
        peak_idx as i64
    } else {
        peak_idx as i64 - fft_len as i64
    };

    // 上面的 lag 方向是「reference 相對於 target 領先多少 sample」。
    // 依照模組 doc 的定義，offset_secs 為正 = reference 比 target 晚開始，
    // 所以要把方向翻過來。
    // 推導：c[l] = sum_n ref[n] * target[n - l]
    //
    // 當 target[n] = ref[n + D]（target 跳過了 reference 前 D 個樣本）：
    //   c[l] = sum_n ref[n] * ref[(n - l) + D]
    //   最大當 l = D → 回傳 +D
    //
    // 當 target[n] = ref[n - D]（target 比 reference 多了 D 個 silence padding）：
    //   最大當 l = -D → 回傳 -D（cyclic wrap 後 unwrap）
    //
    // 這正好對應模組 doc 的 convention：
    //   正值 = target 的 t=0 在 reference 的較晚位置（target 跳過 intro）
    //   負值 = target 的 t=0 在 reference 的較早位置（target 多了 padding）
    (lag_samples, peak_val, mean_abs)
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_noise(len: usize, seed: u32) -> Vec<f32> {
        // 線性同餘產生 pseudo-random，比直接呼叫 rand crate 輕
        let mut state = seed.wrapping_mul(2654435761).wrapping_add(1);
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                (state >> 16) as f32 / 32768.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn stereo_to_mono_averages_channels() {
        let interleaved = [1.0, 3.0, 2.0, 4.0, 5.0, 7.0];
        let mono = stereo_to_mono(&interleaved, 2);
        assert_eq!(mono, vec![2.0, 3.0, 6.0]);
    }

    #[test]
    fn stereo_to_mono_passthrough_for_mono_input() {
        let samples = vec![0.1, 0.2, 0.3];
        let mono = stereo_to_mono(&samples, 1);
        assert_eq!(mono, samples);
    }

    #[test]
    fn downsample_boxcar_preserves_mean_of_constant_signal() {
        let signal = vec![0.5_f32; 100];
        let ds = downsample_boxcar(&signal, 4);
        assert_eq!(ds.len(), 25);
        for v in ds {
            assert!((v - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn downsample_boxcar_factor_one_is_identity() {
        let signal = vec![1.0, 2.0, 3.0];
        let ds = downsample_boxcar(&signal, 1);
        assert_eq!(ds, signal);
    }

    #[test]
    fn cross_correlate_identical_signals_at_zero_lag() {
        let noise = make_noise(4096, 42);
        let (lag, peak, _mean) = cross_correlate_peak(&noise, &noise);
        assert_eq!(lag, 0, "self-correlation should peak at lag 0");
        assert!(peak > 0.0);
    }

    #[test]
    fn cross_correlate_handles_inverted_signal() {
        let noise = make_noise(4096, 43);
        let inverted: Vec<f32> = noise.iter().map(|sample| -*sample).collect();
        let (lag, peak, mean) = cross_correlate_peak(&noise, &inverted);
        assert_eq!(lag, 0, "inverted stems should still align at zero lag");
        assert!(peak < 0.0);
        assert!(
            peak.abs() / mean > 10.0,
            "peak/mean = {}",
            peak.abs() / mean
        );
    }

    #[test]
    fn cross_correlate_detects_target_with_padding() {
        // Target 多了 100 samples 的 silence padding（像是 count-in）
        // → target 的 t=0 對應 reference 的 t=-100 → offset 應為 -100
        let noise = make_noise(4096, 7);
        let shift: usize = 100;
        let mut target = vec![0.0_f32; shift];
        target.extend_from_slice(&noise);

        let (lag, _peak, _mean) = cross_correlate_peak(&noise, &target);
        assert_eq!(lag, -(shift as i64));
    }

    #[test]
    fn cross_correlate_detects_target_skipping_reference_intro() {
        // Reference 前 100 samples 是 silence，之後才是音樂
        // Target 從音樂開頭直接播（跳過了 reference 的 silence）
        // → target 的 t=0 對應 reference 的 t=+100 → offset 應為 +100
        let noise = make_noise(4096, 13);
        let shift: usize = 100;
        let mut reference = vec![0.0_f32; shift];
        reference.extend_from_slice(&noise);

        let (lag, _peak, _mean) = cross_correlate_peak(&reference, &noise);
        assert_eq!(lag, shift as i64);
    }

    #[test]
    fn cross_correlate_handles_target_starts_at_middle_of_reference() {
        // Target 是 reference 從 index 2500 開始的一段（典型情境：伴奏版剪掉了
        // 原曲的長 intro）
        // → target 的 t=0 對應 reference 的 t=+2500 → offset 應為 +2500
        let noise = make_noise(8192, 99);
        let start: usize = 2500;
        let target: Vec<f32> = noise[start..start + 2000].to_vec();

        let (lag, _peak, _mean) = cross_correlate_peak(&noise, &target);
        assert_eq!(lag, start as i64);
    }

    #[test]
    fn cross_correlate_peak_is_high_for_matched_signals() {
        let noise = make_noise(2048, 5);
        let (_lag, peak, mean) = cross_correlate_peak(&noise, &noise);
        // 自相關 peak 應該遠大於平均能量
        assert!(peak / mean > 10.0, "peak/mean = {}", peak / mean);
    }

    #[test]
    fn envelope_alignment_detects_stem_like_padding() {
        let sr = TARGET_SAMPLE_RATE;
        let mut reference = vec![0.0_f32; sr as usize * 3];
        for &start in &[2_000_usize, 8_000, 18_000, 26_000] {
            for i in 0..700 {
                let t = i as f32 / sr as f32;
                reference[start + i] =
                    (2.0 * std::f32::consts::PI * 330.0 * t).sin() * (1.0 - i as f32 / 700.0) * 0.7;
            }
        }

        let pad_samples = (sr as f32 * 0.37).round() as usize;
        let mut target = vec![0.0_f32; pad_samples];
        target.extend(reference.iter().enumerate().map(|(i, sample)| {
            let t = i as f32 / sr as f32;
            let carrier = (2.0 * std::f32::consts::PI * 95.0 * t).sin().signum();
            sample.abs() * carrier * 0.6
        }));

        let reference_envelope = build_alignment_envelope(&reference, sr);
        let target_envelope = build_alignment_envelope(&target, sr);
        let candidate =
            correlation_candidate(&reference_envelope, &target_envelope, ENVELOPE_SAMPLE_RATE);
        let detected_secs = candidate.lag_samples as f32 / ENVELOPE_SAMPLE_RATE as f32;

        assert!(
            (detected_secs + 0.37).abs() < 0.03,
            "expected -0.37s padding alignment, got {detected_secs}"
        );
        assert!(
            candidate.peak_to_mean_ratio > 2.0,
            "expected useful envelope confidence, got {}",
            candidate.peak_to_mean_ratio
        );
    }

    #[test]
    fn select_alignment_candidate_uses_envelope_when_waveform_is_weak() {
        let waveform = CorrelationCandidate {
            lag_samples: 123,
            peak: 1.0,
            peak_to_mean_ratio: 1.2,
            sample_rate: TARGET_SAMPLE_RATE,
        };
        let envelope = CorrelationCandidate {
            lag_samples: -37,
            peak: 1.0,
            peak_to_mean_ratio: 2.4,
            sample_rate: ENVELOPE_SAMPLE_RATE,
        };
        let selected = select_alignment_candidate(waveform, envelope);

        assert_eq!(selected.lag_samples, -37);
        assert_eq!(selected.sample_rate, ENVELOPE_SAMPLE_RATE);
    }

    #[test]
    fn cross_correlate_handles_empty_input() {
        let empty: Vec<f32> = Vec::new();
        let other = make_noise(100, 1);
        let (lag, peak, mean) = cross_correlate_peak(&empty, &other);
        assert_eq!(lag, 0);
        assert_eq!(peak, 0.0);
        assert_eq!(mean, 0.0);
    }

    #[test]
    fn cross_correlate_subsample_accuracy_on_sine() {
        // 對正弦波做 lag 偵測（週期性訊號，所有 lag 都會有 peak，
        // 但真實 shift 應該是最強的那個）
        let sr = 11025.0_f32;
        let freq = 220.0_f32;
        let len = 2048;
        let reference: Vec<f32> = (0..len)
            .map(|i| ((i as f32) * 2.0 * std::f32::consts::PI * freq / sr).sin())
            .collect();
        // Shift by 50 samples forward
        let shift = 50;
        let target: Vec<f32> = (0..len)
            .map(|i| (((i + shift) as f32) * 2.0 * std::f32::consts::PI * freq / sr).sin())
            .collect();

        let (lag, _peak, _mean) = cross_correlate_peak(&reference, &target);
        // 由於正弦波週期性，peak 可能落在 ±週期倍數，這邊只驗證「有偵測到某個位移」
        let period_samples = (sr / freq).round() as i64;
        let lag_mod = ((lag % period_samples) + period_samples) % period_samples;
        let expected_mod = ((-(shift as i64) % period_samples) + period_samples) % period_samples;
        assert_eq!(lag_mod, expected_mod);
    }
}
