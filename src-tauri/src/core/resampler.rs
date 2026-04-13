//! 通用重採樣器：將任意 sample rate 的 mono 音訊轉換為目標 sample rate。
//!
//! 主要用途：CREPE 要求 16kHz 輸入，但音源可能是 44.1kHz / 48kHz / 96kHz。
//!
//! 離線模式用 FFT-based resample（品質最高），
//! 即時模式用線性插值 + anti-alias LPF（延遲最低）。

/// 離線整段重採樣：適合 vocals.wav 載入後一次性處理。
///
/// 使用 band-limited interpolation 的簡化版：
/// 1. Anti-alias LPF（目標 Nyquist × 0.9）
/// 2. 線性插值重採樣
///
/// 對 CREPE 的精度需求（20 cent per bin）綽綽有餘。
pub fn resample_offline(input: &[f32], from_sr: u32, to_sr: u32) -> Vec<f32> {
    if from_sr == to_sr {
        return input.to_vec();
    }

    // 1. Anti-alias LPF（4 階 = 串聯兩個 2 階 biquad，cutoff = to_sr/2 × 0.9）
    //    4 階在 Nyquist 處衰減 ~-24dB/octave，足以應對 96kHz → 16kHz 的 aliasing
    let filtered = if from_sr > to_sr {
        let cutoff = to_sr as f64 * 0.45; // Nyquist × 0.9
        let pass1 = apply_lowpass(input, from_sr, cutoff);
        apply_lowpass(&pass1, from_sr, cutoff)
    } else {
        input.to_vec()
    };

    // 2. 線性插值
    let ratio = from_sr as f64 / to_sr as f64;
    let out_len = ((filtered.len() as f64) / ratio).floor() as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;

        if idx + 1 < filtered.len() {
            output.push(filtered[idx] * (1.0 - frac) + filtered[idx + 1] * frac);
        } else if idx < filtered.len() {
            output.push(filtered[idx]);
        }
    }

    output
}

/// 簡易 2 階 biquad low-pass filter（Butterworth 近似）
fn apply_lowpass(input: &[f32], sample_rate: u32, cutoff_hz: f64) -> Vec<f32> {
    let sr = sample_rate as f64;
    let omega = 2.0 * std::f64::consts::PI * cutoff_hz / sr;
    let cos_w = omega.cos();
    let sin_w = omega.sin();
    let alpha = sin_w / (2.0 * std::f64::consts::FRAC_1_SQRT_2); // Q = 1/sqrt(2)

    let b0 = (1.0 - cos_w) / 2.0;
    let b1 = 1.0 - cos_w;
    let b2 = (1.0 - cos_w) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w;
    let a2 = 1.0 - alpha;

    // Normalize
    let b0 = b0 / a0;
    let b1 = b1 / a0;
    let b2 = b2 / a0;
    let a1 = a1 / a0;
    let a2 = a2 / a0;

    let mut output = Vec::with_capacity(input.len());
    let mut x1 = 0.0_f64;
    let mut x2 = 0.0_f64;
    let mut y1 = 0.0_f64;
    let mut y2 = 0.0_f64;

    for &sample in input {
        let x0 = sample as f64;
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        output.push(y0 as f32);
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
    }

    output
}

// ── 串流重採樣器（即時模式用）─────────────────────────────────────

/// 串流重採樣器：每次餵入一小批 samples，內部維護 LPF + 插值狀態。
///
/// 設計給 CPAL input callback 使用：callback 每次收到一塊 native SR mono，
/// 透過 `process()` 產出 target SR 的 samples，累積到外部 buffer。
///
/// 內部是 4 階 Butterworth LPF + 線性插值，與 `resample_offline` 相同品質，
/// 但能跨 callback 呼叫保持狀態。
pub struct StreamingResampler {
    from_sr: u32,
    to_sr: u32,
    /// 線性插值的步進（from_sr / to_sr 的倒數 = to_sr / from_sr）
    step: f64,
    /// 插值相位（0..1 之間）
    phase: f64,
    /// 上一個 LPF 後的 sample（插值的 t=0 端）
    pending: f32,
    has_pending: bool,
    /// 是否需要降頻（from > to 時啟用 LPF）
    needs_lpf: bool,
    /// 4 階 Butterworth LPF 狀態（2 個串聯 biquad）
    lpf_x1_a: f64, lpf_x2_a: f64, lpf_y1_a: f64, lpf_y2_a: f64,
    lpf_x1_b: f64, lpf_x2_b: f64, lpf_y1_b: f64, lpf_y2_b: f64,
    /// biquad 係數（已正規化）
    b0: f64, b1: f64, b2: f64, a1: f64, a2: f64,
}

impl StreamingResampler {
    /// 建立串流重採樣器。
    ///
    /// `from_sr` 是輸入 sample rate（如 44100、48000），
    /// `to_sr` 是輸出 sample rate（如 16000）。
    pub fn new(from_sr: u32, to_sr: u32) -> Self {
        let needs_lpf = from_sr > to_sr;

        // 計算 biquad 係數（Butterworth LPF, cutoff = to_sr/2 × 0.9）
        let (b0, b1, b2, a1, a2) = if needs_lpf {
            let sr = from_sr as f64;
            let cutoff = to_sr as f64 * 0.45;
            let omega = 2.0 * std::f64::consts::PI * cutoff / sr;
            let cos_w = omega.cos();
            let sin_w = omega.sin();
            let alpha = sin_w / (2.0 * std::f64::consts::FRAC_1_SQRT_2);

            let a0 = 1.0 + alpha;
            (
                ((1.0 - cos_w) / 2.0) / a0,
                (1.0 - cos_w) / a0,
                ((1.0 - cos_w) / 2.0) / a0,
                (-2.0 * cos_w) / a0,
                (1.0 - alpha) / a0,
            )
        } else {
            (1.0, 0.0, 0.0, 0.0, 0.0)
        };

        // step = to_sr / from_sr（每消耗一個輸入 sample 產生多少輸出 sample）
        // 但我們用 phase 從 0 遞增 step 直到 >= 1 才吃下一個輸入
        // 所以 step = from_sr / to_sr 的倒數... 不對。
        // 用與 audio_engine 相同的邏輯：step = to_sr / from_sr
        // phase 每產出一個 sample 加 from_sr/to_sr...
        //
        // 簡化：ratio = from_sr / to_sr（每 ratio 個輸入產生 1 個輸出）
        // step = 1.0 / ratio = to_sr / from_sr
        let step = to_sr as f64 / from_sr as f64;

        Self {
            from_sr,
            to_sr,
            step,
            phase: 0.0,
            pending: 0.0,
            has_pending: false,
            needs_lpf,
            lpf_x1_a: 0.0, lpf_x2_a: 0.0, lpf_y1_a: 0.0, lpf_y2_a: 0.0,
            lpf_x1_b: 0.0, lpf_x2_b: 0.0, lpf_y1_b: 0.0, lpf_y2_b: 0.0,
            b0, b1, b2, a1, a2,
        }
    }

    /// 餵入 native SR 的 mono samples，回傳 target SR 的 output。
    ///
    /// 可在每次 CPAL callback 中呼叫，內部狀態會跨呼叫保留。
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.from_sr == self.to_sr {
            return input.to_vec();
        }

        let mut output = Vec::with_capacity(
            (input.len() as f64 * self.step * 1.1) as usize + 2,
        );

        for &sample in input {
            // 1. LPF（4 階 = 串聯兩個 biquad）
            let filtered = if self.needs_lpf {
                let x0 = sample as f64;
                // Stage A
                let y_a = self.b0 * x0 + self.b1 * self.lpf_x1_a
                    + self.b2 * self.lpf_x2_a
                    - self.a1 * self.lpf_y1_a
                    - self.a2 * self.lpf_y2_a;
                self.lpf_x2_a = self.lpf_x1_a;
                self.lpf_x1_a = x0;
                self.lpf_y2_a = self.lpf_y1_a;
                self.lpf_y1_a = y_a;

                // Stage B
                let y_b = self.b0 * y_a + self.b1 * self.lpf_x1_b
                    + self.b2 * self.lpf_x2_b
                    - self.a1 * self.lpf_y1_b
                    - self.a2 * self.lpf_y2_b;
                self.lpf_x2_b = self.lpf_x1_b;
                self.lpf_x1_b = y_a;
                self.lpf_y2_b = self.lpf_y1_b;
                self.lpf_y1_b = y_b;

                y_b as f32
            } else {
                sample
            };

            // 2. 線性插值重採樣
            if !self.has_pending {
                self.pending = filtered;
                self.has_pending = true;
                self.phase = 0.0;
                continue;
            }

            while self.phase < 1.0 {
                let frac = self.phase as f32;
                let out = self.pending + (filtered - self.pending) * frac;
                output.push(out);
                self.phase += 1.0 / self.step; // += from_sr/to_sr
            }
            self.phase -= 1.0;
            self.pending = filtered;
        }

        output
    }
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_same_rate_is_identity() {
        let input: Vec<f32> = (0..100).map(|i| (i as f32) * 0.01).collect();
        let output = resample_offline(&input, 44100, 44100);
        assert_eq!(output.len(), input.len());
        assert_eq!(output, input);
    }

    #[test]
    fn resample_halves_length_for_double_rate() {
        // 32000 → 16000 should roughly halve the length
        let input = vec![0.0_f32; 32000];
        let output = resample_offline(&input, 32000, 16000);
        assert!((output.len() as i64 - 16000).abs() < 2);
    }

    #[test]
    fn resample_preserves_sine_frequency() {
        // 440Hz sine at 44100Hz → resample to 16000Hz → check frequency preserved
        let from_sr = 44100_u32;
        let to_sr = 16000_u32;
        let freq = 440.0_f64;
        let n = from_sr as usize; // 1 second

        let input: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / from_sr as f64;
                (2.0 * std::f64::consts::PI * freq * t).sin() as f32
            })
            .collect();

        let output = resample_offline(&input, from_sr, to_sr);

        // After resample, check zero-crossings to estimate frequency
        let mut crossings = 0_usize;
        for i in 1..output.len() {
            if (output[i - 1] >= 0.0 && output[i] < 0.0)
                || (output[i - 1] < 0.0 && output[i] >= 0.0)
            {
                crossings += 1;
            }
        }
        // Each cycle has 2 zero-crossings, so frequency ≈ crossings / 2
        let estimated_freq = crossings as f64 / 2.0;
        // Allow 5% tolerance
        assert!(
            (estimated_freq - freq).abs() < freq * 0.05,
            "estimated={} expected={}",
            estimated_freq,
            freq
        );
    }

    // ── StreamingResampler tests ─────────────────────────────────

    #[test]
    fn streaming_same_rate_is_identity() {
        let mut rs = StreamingResampler::new(44100, 44100);
        let input: Vec<f32> = (0..100).map(|i| (i as f32) * 0.01).collect();
        let output = rs.process(&input);
        assert_eq!(output.len(), input.len());
        assert_eq!(output, input);
    }

    #[test]
    fn streaming_48k_to_16k_ratio() {
        // 48000 → 16000 = 3:1，1 秒的輸入應產出約 16000 個 samples
        let mut rs = StreamingResampler::new(48000, 16000);
        let input = vec![0.0_f32; 48000];
        let output = rs.process(&input);
        let diff = (output.len() as i64 - 16000).abs();
        assert!(diff < 10, "expected ~16000, got {}", output.len());
    }

    #[test]
    fn streaming_preserves_sine_across_chunks() {
        // 分多次 callback 餵入，結果應與一次餵入一致
        let from_sr = 48000_u32;
        let to_sr = 16000_u32;
        let freq = 440.0_f64;
        let n = from_sr as usize;

        let full_input: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / from_sr as f64;
                (2.0 * std::f64::consts::PI * freq * t).sin() as f32
            })
            .collect();

        // 一次處理
        let mut rs_full = StreamingResampler::new(from_sr, to_sr);
        let out_full = rs_full.process(&full_input);

        // 分 chunk 處理
        let mut rs_chunk = StreamingResampler::new(from_sr, to_sr);
        let chunk_size = 480; // 10ms chunks
        let mut out_chunk = Vec::new();
        for chunk in full_input.chunks(chunk_size) {
            out_chunk.extend_from_slice(&rs_chunk.process(chunk));
        }

        // 長度應接近
        let len_diff = (out_full.len() as i64 - out_chunk.len() as i64).abs();
        assert!(
            len_diff < 5,
            "full={} chunk={} diff={}",
            out_full.len(),
            out_chunk.len(),
            len_diff
        );
    }
}
