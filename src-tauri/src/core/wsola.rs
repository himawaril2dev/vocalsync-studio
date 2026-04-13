//! WSOLA (Waveform Similarity Overlap-Add) 變速不變調處理器
//!
//! 支援即時 time-stretching（變速不變調）和 pitch-shifting（移調）。
//!
//! # 演算法
//!
//! WSOLA 在 OLA 基礎上增加容差搜尋（tolerance search），
//! 透過 cross-correlation 找到最佳重疊位置，減少合成時的波形不連續感。
//!
//! ## Overlap-Add 正確性
//!
//! 以 75% overlap（OVERLAP_FACTOR=4）為例，在穩態下每個輸出位置
//! 同時接收 4 個 Hanning 視窗的貢獻。
//! 本實作透過逐位置 COLA 歸一化表補償增益，確保輸出振幅正確。
//!
//! ## 串流安全
//!
//! `partial_offset` 追蹤每個 HOP_SYN 已輸出的幀數，
//! 確保 CPAL callback 大小不對齊 HOP_SYN 時不會丟失資料。
//! 只有完整輸出一個 HOP_SYN 後才執行 shift 和位置推進。
//!
//! # 使用方式
//!
//! - 僅變速：`stretch_ratio = 1.0 / speed`
//! - 僅移調：`stretch_ratio = pitch_ratio`，呼叫方另做 resample
//! - 同時變速 + 移調：`stretch_ratio = pitch_ratio / speed`
//! - `speed=1.0` 且 `pitch=0` 時由呼叫方繞過（bypass），不經本模組

/// 分析/合成視窗大小（sample 數），~93ms at 44.1kHz
/// 較大的視窗能更好地保留低頻成分，減少 artifact
const WINDOW_SIZE: usize = 4096;

/// 重疊因子：4 = 75% overlap
const OVERLAP_FACTOR: usize = 4;

/// 合成 hop（每步合成的新輸出幀��）
const HOP_SYN: usize = WINDOW_SIZE / OVERLAP_FACTOR; // 1024

/// 交叉相關搜尋範圍（±sample 數）
/// 882 samples ≈ 20ms at 44.1kHz，足以覆蓋 50Hz 低音一個完整週期
const SEARCH_RANGE: usize = 882;

/// 串流式 WSOLA 處理器。
pub struct WsolaProcessor {
    channels: usize,

    /// 當前在 source 中的讀取位置（frame 單位，允許小數）
    input_pos: f64,

    /// 輸出累積緩衝區（per channel，大小 = WINDOW_SIZE）
    /// 存放加窗後累加的結果；每次完整輸出 HOP_SYN 幀後左移
    accum_buf: Vec<Vec<f32>>,

    /// 預先計算的 Hanning 視窗
    hann: Vec<f32>,

    /// 逐位置 COLA 歸一化倒數表（大小 = HOP_SYN）
    cola_inv_table: Vec<f32>,

    /// 當前 HOP_SYN 已輸出的幀數（0..HOP_SYN）
    /// 確保 callback 大小不對齊 HOP_SYN 時不丟失資料
    partial_offset: usize,

    /// 當前 hop 是否已完成視窗累加（需要等 partial 輸出完再 shift）
    hop_pending_shift: bool,

    /// 待推進的分析步幅（在 partial 完整輸出後才推進 input_pos）
    pending_hop_analysis: f64,

    /// 是否已完成第一個視窗
    initialized: bool,
}

impl WsolaProcessor {
    pub fn new(channels: usize) -> Self {
        let ch = channels.max(1);
        let hann: Vec<f32> = (0..WINDOW_SIZE)
            .map(|i| {
                let t = i as f32 / (WINDOW_SIZE - 1) as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * t).cos())
            })
            .collect();

        // 逐位置計算 COLA 歸一化倒數表
        let cola_inv_table: Vec<f32> = (0..HOP_SYN)
            .map(|i| {
                let sum: f32 = (0..OVERLAP_FACTOR)
                    .map(|k| {
                        let idx = i + k * HOP_SYN;
                        if idx < WINDOW_SIZE { hann[idx] } else { 0.0 }
                    })
                    .sum();
                if sum > 1e-6 { 1.0 / sum } else { 1.0 }
            })
            .collect();

        Self {
            channels: ch,
            input_pos: 0.0,
            accum_buf: vec![vec![0.0; WINDOW_SIZE]; ch],
            hann,
            cola_inv_table,
            partial_offset: 0,
            hop_pending_shift: false,
            pending_hop_analysis: 0.0,
            initialized: false,
        }
    }

    pub fn input_pos(&self) -> f64 {
        self.input_pos
    }

    pub fn set_input_pos(&mut self, pos: f64) {
        if (self.input_pos - pos).abs() > HOP_SYN as f64 {
            self.reset_state();
        }
        self.input_pos = pos.max(0.0);
    }

    pub fn reset(&mut self) {
        self.reset_state();
        self.input_pos = 0.0;
    }

    fn reset_state(&mut self) {
        for buf in &mut self.accum_buf {
            buf.fill(0.0);
        }
        self.partial_offset = 0;
        self.hop_pending_shift = false;
        self.pending_hop_analysis = 0.0;
        self.initialized = false;
    }

    /// 產生 time-stretched 的交錯音訊資料。
    ///
    /// 核心不變式：accum_buf 只在完整輸出一個 HOP_SYN 後才做 shift。
    /// 這確保任何 callback 大小都不會丟失資料。
    pub fn process(
        &mut self,
        source: &[f32],
        output: &mut [f32],
        stretch_ratio: f64,
    ) -> f64 {
        let ch = self.channels;
        let total_source_frames = source.len() / ch;
        let output_frames = output.len() / ch;

        if output_frames == 0 || total_source_frames == 0 {
            return 0.0;
        }

        let stretch = stretch_ratio.clamp(0.1, 10.0);
        let hop_analysis = HOP_SYN as f64 / stretch;

        let start_pos = self.input_pos;
        let mut written = 0;

        while written < output_frames {
            // ── 先把上一次未完的 partial hop 輸出完 ──
            if self.hop_pending_shift {
                let remaining = HOP_SYN - self.partial_offset;
                let can_write = remaining.min(output_frames - written);

                for f in 0..can_write {
                    let buf_pos = self.partial_offset + f;
                    let norm = self.cola_inv_table[buf_pos];
                    for c in 0..ch {
                        output[(written + f) * ch + c] = self.accum_buf[c][buf_pos] * norm;
                    }
                }
                written += can_write;
                self.partial_offset += can_write;

                if self.partial_offset >= HOP_SYN {
                    // 完整輸出了一個 HOP_SYN → 執行 shift 和位置推進
                    for c in 0..ch {
                        self.accum_buf[c].copy_within(HOP_SYN..WINDOW_SIZE, 0);
                        let tail_start = WINDOW_SIZE - HOP_SYN;
                        for i in tail_start..WINDOW_SIZE {
                            self.accum_buf[c][i] = 0.0;
                        }
                    }
                    self.input_pos += self.pending_hop_analysis;
                    self.partial_offset = 0;
                    self.hop_pending_shift = false;
                }
                continue;
            }

            // ── 處理新的一個 hop ──
            let nom_pos = self.input_pos.round() as isize;

            // 邊界檢查：source 不夠讀一整個 window → 填零結束
            if nom_pos < 0 || (nom_pos as usize + WINDOW_SIZE) > total_source_frames {
                for f in written..output_frames {
                    for c in 0..ch {
                        output[f * ch + c] = 0.0;
                    }
                }
                break;
            }

            // ① 搜尋最佳重疊位置
            let read_pos = if self.initialized {
                let offset =
                    self.find_best_offset(source, nom_pos as usize, total_source_frames);
                let adjusted = (nom_pos + offset) as usize;
                adjusted.min(total_source_frames.saturating_sub(WINDOW_SIZE))
            } else {
                self.initialized = true;
                nom_pos as usize
            };

            // ② 將 Hanning 加權的視窗累加到 accum_buf
            for c in 0..ch {
                let base = read_pos * ch + c;
                for i in 0..WINDOW_SIZE {
                    let src_idx = base + i * ch;
                    if src_idx < source.len() {
                        self.accum_buf[c][i] += source[src_idx] * self.hann[i];
                    }
                }
            }

            // ③ 從 accum_buf 前端輸出（可能只輸出一部分）
            let can_write = HOP_SYN.min(output_frames - written);
            for f in 0..can_write {
                let norm = self.cola_inv_table[f];
                for c in 0..ch {
                    output[(written + f) * ch + c] = self.accum_buf[c][f] * norm;
                }
            }
            written += can_write;

            if can_write >= HOP_SYN {
                // 完整輸出 → 立即 shift 和推進
                for c in 0..ch {
                    self.accum_buf[c].copy_within(HOP_SYN..WINDOW_SIZE, 0);
                    let tail_start = WINDOW_SIZE - HOP_SYN;
                    for i in tail_start..WINDOW_SIZE {
                        self.accum_buf[c][i] = 0.0;
                    }
                }
                self.input_pos += hop_analysis;
            } else {
                // 部分輸出 → 記錄 partial 狀態，下次繼續
                self.partial_offset = can_write;
                self.hop_pending_shift = true;
                self.pending_hop_analysis = hop_analysis;
                // 不 shift、不推進 input_pos
            }
        }

        self.input_pos - start_pos
    }

    /// 用 normalized cross-correlation 在搜尋範圍內找最佳重疊位置。
    ///
    /// ��較對象是 accum_buf 的前端（前一次遺留的重疊尾巴），
    /// 候選也施加 Hanning 加權以匹配加窗疊加的實際輸出。
    ///
    /// 改進：
    /// - 使用所有聲道的平均相關性（而非僅 channel 0）
    /// - 二階搜尋（粗搜 step=4 + 精搜 ±4）減少計算量
    fn find_best_offset(
        &self,
        source: &[f32],
        nominal_pos: usize,
        total_frames: usize,
    ) -> isize {
        let ch = self.channels;
        let compare_len = HOP_SYN;

        let search_start = -(SEARCH_RANGE as isize);
        let search_end = SEARCH_RANGE as isize;

        // ── 第一階段：粗��（step=4） ──
        let coarse_step = 4_isize;
        let mut best_offset: isize = 0;
        let mut best_corr = f64::NEG_INFINITY;

        let mut offset = search_start;
        while offset <= search_end {
            let corr = self.compute_correlation(
                source, nominal_pos, total_frames, offset, ch, compare_len,
            );
            if corr > best_corr {
                best_corr = corr;
                best_offset = offset;
            }
            offset += coarse_step;
        }

        // ── 第二階段：精搜（best ± coarse_step） ──
        let fine_start = (best_offset - coarse_step).max(search_start);
        let fine_end = (best_offset + coarse_step).min(search_end);

        for offset in fine_start..=fine_end {
            let corr = self.compute_correlation(
                source, nominal_pos, total_frames, offset, ch, compare_len,
            );
            if corr > best_corr {
                best_corr = corr;
                best_offset = offset;
            }
        }

        best_offset
    }

    /// 計算單一 offset 的 normalized cross-correlation。
    /// 用所有聲道的平均相關性，候選施加 Hanning 加權。
    fn compute_correlation(
        &self,
        source: &[f32],
        nominal_pos: usize,
        total_frames: usize,
        offset: isize,
        ch: usize,
        compare_len: usize,
    ) -> f64 {
        let pos = nominal_pos as isize + offset;
        if pos < 0 || (pos as usize + WINDOW_SIZE) > total_frames {
            return f64::NEG_INFINITY;
        }

        let base_pos = pos as usize;
        let mut total_corr = 0.0_f64;

        for c in 0..ch {
            let ref_buf = &self.accum_buf[c];
            let mut dot = 0.0_f64;
            let mut norm_ref = 0.0_f64;
            let mut norm_cand = 0.0_f64;

            for i in 0..compare_len {
                let a = ref_buf[i] as f64;
                let src_idx = base_pos * ch + i * ch + c;
                let b = if src_idx < source.len() {
                    (source[src_idx] * self.hann[i]) as f64
                } else {
                    0.0
                };
                dot += a * b;
                norm_ref += a * a;
                norm_cand += b * b;
            }

            let corr = if norm_ref > 1e-10 && norm_cand > 1e-10 {
                dot / (norm_ref.sqrt() * norm_cand.sqrt())
            } else {
                0.0
            };
            total_corr += corr;
        }

        total_corr / ch as f64
    }
}

// ── 測試 ────────────────────────────────────────���─────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_wave(freq: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin() * 0.5)
            .collect()
    }

    fn sine_wave_stereo(freq: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        let mut buf = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin() * 0.5;
            buf.push(s);
            buf.push(s);
        }
        buf
    }

    #[test]
    fn bypass_stretch_ratio_one_preserves_length() {
        let source = sine_wave(440.0, 44100.0, 1.0);
        let mut output = vec![0.0; 8192];
        let mut wsola = WsolaProcessor::new(1);
        let consumed = wsola.process(&source, &mut output, 1.0);
        assert!(
            (consumed - 8192.0).abs() < HOP_SYN as f64 * 2.0,
            "consumed={consumed}, expected ~8192"
        );
    }

    #[test]
    fn stretch_two_consumes_half_input() {
        let source = sine_wave(440.0, 44100.0, 2.0);
        let mut output = vec![0.0; 8192];
        let mut wsola = WsolaProcessor::new(1);
        let consumed = wsola.process(&source, &mut output, 2.0);
        let ratio = consumed / 8192.0;
        assert!(ratio > 0.35 && ratio < 0.65, "ratio={ratio}, expected ~0.5");
    }

    #[test]
    fn stretch_half_consumes_double_input() {
        let source = sine_wave(440.0, 44100.0, 2.0);
        let mut output = vec![0.0; 4096];
        let mut wsola = WsolaProcessor::new(1);
        let consumed = wsola.process(&source, &mut output, 0.5);
        let ratio = consumed / 4096.0;
        assert!(ratio > 1.5 && ratio < 2.5, "ratio={ratio}, expected ~2.0");
    }

    #[test]
    fn stereo_processing_preserves_channel_count() {
        let source = sine_wave_stereo(440.0, 44100.0, 1.0);
        let mut output = vec![0.0; 4096 * 2];
        let mut wsola = WsolaProcessor::new(2);
        wsola.process(&source, &mut output, 1.0);
        let l_energy: f32 = output.iter().step_by(2).map(|s| s * s).sum();
        let r_energy: f32 = output.iter().skip(1).step_by(2).map(|s| s * s).sum();
        assert!(l_energy > 0.01, "Left channel should have energy");
        assert!(r_energy > 0.01, "Right channel should have energy");
    }

    #[test]
    fn output_is_not_silence_for_tonal_input() {
        let source = sine_wave(440.0, 44100.0, 1.0);
        let mut output = vec![0.0; 8192];
        let mut wsola = WsolaProcessor::new(1);
        wsola.process(&source, &mut output, 1.5);
        let energy: f32 = output.iter().map(|s| s * s).sum::<f32>() / output.len() as f32;
        assert!(energy > 0.001, "RMS²={energy}");
    }

    #[test]
    fn reset_clears_state() {
        let source = sine_wave(440.0, 44100.0, 1.0);
        let mut output = vec![0.0; 2048];
        let mut wsola = WsolaProcessor::new(1);
        wsola.process(&source, &mut output, 1.0);
        assert!(wsola.input_pos() > 0.0);
        wsola.reset();
        assert_eq!(wsola.input_pos(), 0.0);
        assert!(!wsola.initialized);
    }

    #[test]
    fn set_input_pos_resets_on_large_jump() {
        let mut wsola = WsolaProcessor::new(1);
        wsola.input_pos = 10000.0;
        wsola.initialized = true;
        wsola.set_input_pos(0.0);
        assert!(!wsola.initialized);
    }

    #[test]
    fn set_input_pos_preserves_state_on_small_change() {
        let mut wsola = WsolaProcessor::new(1);
        wsola.input_pos = 1000.0;
        wsola.initialized = true;
        wsola.accum_buf[0][0] = 0.5;
        wsola.set_input_pos(1100.0);
        assert!(wsola.initialized);
        assert_eq!(wsola.accum_buf[0][0], 0.5);
    }

    #[test]
    fn handles_source_boundary_gracefully() {
        let source = vec![0.5; 2048];
        let mut output = vec![0.0; 8192];
        let mut wsola = WsolaProcessor::new(1);
        wsola.process(&source, &mut output, 1.0);
    }

    #[test]
    fn streaming_consistency_across_multiple_calls() {
        let source = sine_wave(440.0, 44100.0, 2.0);
        let mut wsola = WsolaProcessor::new(1);
        let mut total_consumed = 0.0;
        for _ in 0..16 {
            let mut output = vec![0.0; 1024];
            total_consumed += wsola.process(&source, &mut output, 1.0);
        }
        assert!(
            (total_consumed - 16384.0).abs() < HOP_SYN as f64 * 4.0,
            "consumed={total_consumed}, expected ~16384"
        );
    }

    #[test]
    fn amplitude_stability_at_stretch_one() {
        let source = sine_wave(440.0, 44100.0, 1.0);
        let mut output = vec![0.0; 22050];
        let mut wsola = WsolaProcessor::new(1);
        wsola.process(&source, &mut output, 1.0);

        // 跳過 ramp-up（前 4 個視窗 = 4 * HOP_SYN = 4096 幀）
        let stable = &output[5000..20000];
        let rms: f32 = (stable.iter().map(|s| s * s).sum::<f32>() / stable.len() as f32).sqrt();

        // COLA 歸一化後，輸出振幅應與輸入一致
        // 輸入 = 0.5 振幅 sine wave，理論 RMS = 0.5/√2 ≈ 0.354
        assert!(
            rms > 0.25 && rms < 0.50,
            "RMS={rms}, expected in range [0.25, 0.50]"
        );
    }

    #[test]
    fn cola_normalization_prevents_gain() {
        let source = sine_wave(440.0, 44100.0, 1.0);
        let mut output = vec![0.0; 22050];
        let mut wsola = WsolaProcessor::new(1);
        wsola.process(&source, &mut output, 1.0);

        let stable = &output[5000..20000];
        let peak = stable.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        assert!(
            peak < 0.6,
            "peak={peak}, should be close to 0.5 (input amplitude)"
        );
    }

    /// 關鍵測試：用小 buffer 模擬 CPAL callback，驗證不丟失資料
    #[test]
    fn small_buffer_streaming_no_data_loss() {
        let source = sine_wave(440.0, 44100.0, 2.0);
        let mut wsola_big = WsolaProcessor::new(1);
        let mut wsola_small = WsolaProcessor::new(1);

        // 大 buffer 一次處理
        let mut big_out = vec![0.0; 8192];
        wsola_big.process(&source, &mut big_out, 1.0);

        // 小 buffer 多次處理（模擬 256-frame CPAL callback）
        let mut small_out = Vec::new();
        for _ in 0..32 {
            let mut chunk = vec![0.0; 256];
            wsola_small.process(&source, &mut chunk, 1.0);
            small_out.extend_from_slice(&chunk);
        }

        // 跳過 ramp-up 後比較能量
        let big_rms: f32 = (big_out[5000..8000]
            .iter()
            .map(|s| s * s)
            .sum::<f32>()
            / 3000.0)
            .sqrt();
        let small_rms: f32 = (small_out[5000..8000]
            .iter()
            .map(|s| s * s)
            .sum::<f32>()
            / 3000.0)
            .sqrt();

        // 兩者的 RMS 應該接近（10% 以內）
        let diff = (big_rms - small_rms).abs() / big_rms;
        assert!(
            diff < 0.15,
            "big_rms={big_rms}, small_rms={small_rms}, diff={diff:.1}%, \
             small buffer should not lose data"
        );
    }
}
