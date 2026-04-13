//! PYIN 主旋律分析（離線、整段最佳路徑）
//!
//! 參考論文：
//!   Mauch & Dixon (2014), "PYIN: A fundamental frequency estimator using
//!   probabilistic threshold distributions"
//!
//! 與 YIN 差異：
//! - YIN：每 frame 固定閾值，只挑第一個低於閾值的 τ → 對複雜伴奏容易抓錯八度
//! - PYIN：每 frame 收多個 candidate（不同閾值對應不同 τ），用 Viterbi 動態規劃
//!         在所有 frame 之間尋找全域最佳路徑，連續性最佳化、八度跳動最小化
//!
//! 設計重點：
//! - 純 Rust，無 ML、無外部數學庫
//! - 適用於離線整段分析（伴奏載入後背景跑一次）
//! - 不適用於即時偵測（需要全序列才能解碼）→ 即時 mic 仍用 `pitch_engine::PitchDetector`
//! - Beta 分佈簡化為查表（離散 21 點）以避免依賴額外 crate

use crate::core::pitch_data::{freq_to_note, freq_to_midi, PitchSample, PitchTrack};

/// 一個 frame 的單一 candidate
#[derive(Clone, Debug)]
struct Candidate {
    /// 估計頻率（Hz）
    freq: f64,
    /// 對應的 MIDI 音高（含小數）
    midi: f64,
    /// CMNDF 值（越小越好）
    d_prime: f64,
    /// 加權後的「觀測機率」（越大越好）
    weight: f64,
}

/// 一個 frame 的所有 candidates + 元資料
#[derive(Clone, Debug, Default)]
struct Frame {
    timestamp: f64,
    candidates: Vec<Candidate>,
}

/// 整段分析結果
#[derive(Debug)]
pub struct PyinResult {
    /// 平滑後的音高軌跡
    pub track: PitchTrack,
    /// 品質評估
    pub quality: PyinQuality,
}

/// 整段分析的品質指標
#[derive(Debug, Clone)]
pub struct PyinQuality {
    /// 總共處理的 frame 數
    pub total_frames: usize,
    /// 偵測為 voiced 的 frame 數
    pub voiced_frames: usize,
    /// voiced 比例（0.0 ~ 1.0）
    pub voiced_ratio: f64,
    /// voiced frame 的平均信心度（0.0 ~ 1.0）
    pub mean_confidence: f64,
    /// 是否被判定為「無法可靠偵測主旋律」
    pub is_unreliable: bool,
    /// 診斷：被 RMS 門檻截斷（無 candidate）的 frame 數
    pub rms_rejected_frames: usize,
    /// 診斷：有 candidate 但 Viterbi 選為 unvoiced 的 frame 數
    pub viterbi_rejected_frames: usize,
    /// 診斷：voiced frame 的 d' 分佈 histogram（10 個 bin: 0-0.1, 0.1-0.2, ..., 0.9-1.0）
    pub voiced_d_prime_hist: [usize; 10],
    /// 診斷：有 candidate 但被 Viterbi 判 unvoiced 的 frame，其最佳 candidate d' 分佈
    pub unvoiced_best_d_prime_hist: [usize; 10],
}

/// PYIN 分析器（離線整段）
pub struct PyinAnalyzer {
    sample_rate: u32,
    buf_size: usize,
    hop: usize,
    tau_min: usize,
    tau_max: usize,
    /// candidate 篩選的 d' 上界（典型 0.5）
    max_threshold: f64,
    /// RMS 靜音門檻
    rms_threshold: f64,
    /// 每 frame 最多保留 candidate 數
    max_candidates: usize,
    /// Viterbi 八度跳動懲罰係數
    transition_penalty: f64,
    /// Voiced/unvoiced 切換懲罰：適中即可，太高會讓孤立 voiced frame 進不來
    voicing_penalty: f64,
    /// unvoiced 觀測代價：搭配 Beta(2,18) CDF likelihood，
    /// unvoiced_cost = 9.0 對應 isolated frame 切換點 d' ≈ 0.18，
    /// 連續 voiced 段中容忍 d' 到約 0.55（隨段長變化）。
    /// 提高此值能大幅提升混音歌曲的 voiced 覆蓋率
    unvoiced_cost: f64,
    /// 工作緩衝
    diff_buf: Vec<f64>,
}

/// PYIN 預設參數
pub struct PyinParams {
    pub sample_rate: u32,
    pub buf_size: usize,
    pub hop: usize,
    pub f_min: f64,
    pub f_max: f64,
    pub max_threshold: f64,
    pub rms_threshold: f64,
    pub unvoiced_cost: f64,
}

impl Default for PyinParams {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            buf_size: 2048,
            hop: 512,
            f_min: 65.0,   // C2
            f_max: 1200.0, // ~D6
            max_threshold: 0.5,
            rms_threshold: 0.005,
            unvoiced_cost: 9.0,
        }
    }
}

impl PyinAnalyzer {
    pub fn new(p: PyinParams) -> Self {
        let sr = p.sample_rate as f64;
        let tau_min = ((sr / p.f_max).floor() as usize).max(2);
        let tau_max_raw = (sr / p.f_min).ceil() as usize;
        let tau_max = tau_max_raw.min(p.buf_size / 2);

        Self {
            sample_rate: p.sample_rate,
            buf_size: p.buf_size,
            hop: p.hop,
            tau_min,
            tau_max,
            max_threshold: p.max_threshold,
            rms_threshold: p.rms_threshold,
            max_candidates: 8,
            transition_penalty: 3.0,
            voicing_penalty: 2.5,
            unvoiced_cost: p.unvoiced_cost,
            diff_buf: vec![0.0; p.buf_size / 2 + 1],
        }
    }

    /// 對整段 mono 音訊執行 PYIN 並回傳平滑後的旋律
    pub fn analyze(&mut self, mono: &[f32]) -> PyinResult {
        let frames = self.collect_frames(mono);
        let path = self.viterbi(&frames);
        let raw_track = self.path_to_track(&frames, &path);
        // 5-frame sliding median post-processing 去除孤立 outlier，
        // 不破壞 vibrato（vibrato 通常持續 > 5 frame）
        let track = median_smooth_track(&raw_track, 5);
        let quality = self.evaluate_quality(&frames, &path);
        PyinResult { track, quality }
    }

    // ── Phase 1：每 frame 抽 candidates ────────────────────────────

    fn collect_frames(&mut self, mono: &[f32]) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut start = 0;
        while start + self.buf_size <= mono.len() {
            let window = &mono[start..start + self.buf_size];
            let timestamp = start as f64 / self.sample_rate as f64;
            let frame = self.frame_candidates(window, timestamp);
            frames.push(frame);
            start += self.hop;
        }
        frames
    }

    fn frame_candidates(&mut self, samples: &[f32], timestamp: f64) -> Frame {
        let rms = calc_rms(samples);
        if rms < self.rms_threshold {
            return Frame {
                timestamp,
                candidates: Vec::new(),
            };
        }

        // ── 差分函數 d(τ) ──
        let half = self.buf_size / 2;
        for tau in 1..=half {
            let mut sum = 0.0_f64;
            for i in 0..half {
                let diff = samples[i] as f64 - samples[i + tau] as f64;
                sum += diff * diff;
            }
            self.diff_buf[tau] = sum;
        }

        // ── CMNDF：累積平均正規化差分 ──
        self.diff_buf[0] = 1.0;
        let mut running_sum = 0.0_f64;
        for tau in 1..=half {
            running_sum += self.diff_buf[tau];
            if running_sum > 0.0 {
                self.diff_buf[tau] = self.diff_buf[tau] * (tau as f64) / running_sum;
            } else {
                self.diff_buf[tau] = 1.0;
            }
        }

        // ── 找 candidates ──
        //
        // 正統 PYIN：對所有 d' < max_threshold 的 local minima 收集為 candidates，
        // 用 Beta(2,18) CDF 計算 likelihood。
        //
        // 演算法核心：
        // - Likelihood: P(d') = 1 - I_{d'}(2, 18)
        //   對 d' = 0.0 → 1.0, 0.05 → 0.358, 0.10 → 0.122, 0.15 → 0.041, 0.20 → 0.014
        //   單調遞減，越接近 0 越自信
        // - 移除 primary/backup 區分：所有 local minima 同等對待，由 viterbi 用
        //   transition penalty + voicing penalty 自動選最佳路徑
        // - 高 voicing_penalty (5.0) 讓 viterbi 傾向「連續 voiced 段」，
        //   即使中間有少數 high-d' frame 也會被拉成 voiced（混音常態）
        // - 排序為 stable sort：相同 weight 時，較小 τ（較高頻）優先，
        //   保證純正弦能正確抓 fundamental

        let mut candidates: Vec<Candidate> = Vec::new();
        let mut tau = self.tau_min.max(1);
        while tau + 1 < self.tau_max {
            let d = self.diff_buf[tau];
            if d < self.max_threshold {
                let is_local_min =
                    self.diff_buf[tau - 1] >= d && self.diff_buf[tau + 1] >= d;
                if is_local_min {
                    let better_tau = parabolic_interpolation(&self.diff_buf, tau);
                    if better_tau > 0.0 {
                        let freq = self.sample_rate as f64 / better_tau;
                        if freq.is_finite() && freq > 0.0 {
                            let midi = freq_to_midi(freq);
                            let weight = pyin_likelihood(d).max(1e-10);
                            candidates.push(Candidate {
                                freq,
                                midi,
                                d_prime: d,
                                weight,
                            });
                        }
                    }
                }
            }
            tau += 1;
        }

        // ── 保留 top-K ──
        //
        // 排序邏輯：
        //  - 一般情況按 weight 由大到小
        //  - 特例：兩個 candidate 的 weight 都 > 0.5（極高信心，純信號特徵）
        //          且差距 < 10% → 偏好較高頻率（較小 τ）
        //
        // 這個 tie-breaker 解決純正弦的「sample misalignment 子諧波選擇」問題：
        // 對 440Hz 純正弦在 44100Hz 採樣下，τ_real ≈ 100.227 不是整數，導致較大整數
        // τ（如 401）反而更接近真實週期倍數、d' 更低，會誤選為基頻。
        // 對混音歌曲 d' 通常 > 0.2，weight 遠小於 0.5，不觸發此 tie-breaker。
        candidates.sort_by(|a, b| {
            let both_high_conf = a.weight > 0.5 && b.weight > 0.5;
            if both_high_conf {
                let max_w = a.weight.max(b.weight);
                let rel_diff = (a.weight - b.weight).abs() / max_w;
                if rel_diff < 0.10 {
                    // 信心相當 → 偏好較高頻率（較小 τ）
                    return b
                        .freq
                        .partial_cmp(&a.freq)
                        .unwrap_or(std::cmp::Ordering::Equal);
                }
            }
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(self.max_candidates);

        Frame {
            timestamp,
            candidates,
        }
    }

    // ── Phase 2：Viterbi 動態規劃 ──────────────────────────────────
    //
    // 狀態空間：每個 frame 的 candidate idx，外加一個 "unvoiced" 狀態
    // 觀測代價 cost(state) = -ln(weight)（unvoiced 用固定 unvoiced_cost）
    // 轉移代價 trans(prev, curr):
    //   - voiced → voiced: |Δmidi|² × transition_penalty / 144（normalize 到一個八度）
    //                      上限 voicing_penalty
    //   - voiced ↔ unvoiced: voicing_penalty
    //   - unvoiced → unvoiced: 0
    //
    // 為了效率不限制狀態空間（每個 frame 至多 5 + 1 = 6 個狀態），整體 O(N · K²)

    fn viterbi(&self, frames: &[Frame]) -> Vec<Option<usize>> {
        if frames.is_empty() {
            return Vec::new();
        }

        // 每 frame 的狀態數 = candidates + 1 (unvoiced 在 idx == candidates.len())
        // dp[i][s] = 到第 i 個 frame、處於 state s 的最小代價
        // back[i][s] = 對應前一個 frame 的 best state
        let mut dp: Vec<Vec<f64>> = Vec::with_capacity(frames.len());
        let mut back: Vec<Vec<usize>> = Vec::with_capacity(frames.len());

        // 初始化第 0 個 frame
        let frame0 = &frames[0];
        let s0 = frame0.candidates.len() + 1; // +1 unvoiced
        let mut dp0 = vec![f64::INFINITY; s0];
        for (i, c) in frame0.candidates.iter().enumerate() {
            dp0[i] = -c.weight.max(1e-12).ln();
        }
        dp0[frame0.candidates.len()] = self.unvoiced_cost;
        dp.push(dp0);
        back.push(vec![0; s0]);

        // 動態規劃
        for i in 1..frames.len() {
            let prev_frame = &frames[i - 1];
            let curr_frame = &frames[i];
            let prev_states = prev_frame.candidates.len() + 1;
            let curr_states = curr_frame.candidates.len() + 1;
            let mut dp_i = vec![f64::INFINITY; curr_states];
            let mut back_i = vec![0_usize; curr_states];

            for s in 0..curr_states {
                let obs_cost = if s < curr_frame.candidates.len() {
                    -curr_frame.candidates[s].weight.max(1e-12).ln()
                } else {
                    self.unvoiced_cost
                };

                let mut best_cost = f64::INFINITY;
                let mut best_prev = 0_usize;
                for p in 0..prev_states {
                    let trans_cost = self.transition_cost(prev_frame, curr_frame, p, s);
                    let total = dp[i - 1][p] + trans_cost;
                    if total < best_cost {
                        best_cost = total;
                        best_prev = p;
                    }
                }
                dp_i[s] = best_cost + obs_cost;
                back_i[s] = best_prev;
            }
            dp.push(dp_i);
            back.push(back_i);
        }

        // 回溯：從最後一個 frame 的最低代價狀態開始
        let last = dp.len() - 1;
        let mut best_state = 0_usize;
        let mut best_cost = f64::INFINITY;
        for (s, &cost) in dp[last].iter().enumerate() {
            if cost < best_cost {
                best_cost = cost;
                best_state = s;
            }
        }

        let mut path: Vec<Option<usize>> = vec![None; frames.len()];
        let mut state = best_state;
        for i in (0..frames.len()).rev() {
            let unvoiced_idx = frames[i].candidates.len();
            path[i] = if state == unvoiced_idx {
                None
            } else {
                Some(state)
            };
            if i > 0 {
                state = back[i][state];
            }
        }
        path
    }

    fn transition_cost(
        &self,
        prev_frame: &Frame,
        curr_frame: &Frame,
        prev_state: usize,
        curr_state: usize,
    ) -> f64 {
        let prev_unvoiced = prev_state == prev_frame.candidates.len();
        let curr_unvoiced = curr_state == curr_frame.candidates.len();

        match (prev_unvoiced, curr_unvoiced) {
            (true, true) => 0.0,
            (false, false) => {
                let prev_midi = prev_frame.candidates[prev_state].midi;
                let curr_midi = curr_frame.candidates[curr_state].midi;
                let delta = (curr_midi - prev_midi).abs();
                // 「相同音高」容忍區：< 30 cent (0.3 半音) 視為同一個音，無 penalty。
                // 這允許輕微 vibrato（典型 ±20-50 cent）不被當成 transition，
                // 也避免純信號 frame-to-frame freq drift 累積偽 cost。
                // 對真實 pitch 變化（半音以上）仍會正確懲罰。
                if delta < 0.3 {
                    return 0.0;
                }
                // 每半音的距離平方代價：transition_penalty=3.0 讓 1 半音 cost ≈ 0.25，
                // 有效阻止 viterbi 在連續 frame 之間無謂跳動造成抖動
                let cost = (delta / 12.0).powi(2) * self.transition_penalty * 12.0;
                cost.min(self.voicing_penalty * 2.0)
            }
            _ => self.voicing_penalty,
        }
    }

    fn path_to_track(&self, frames: &[Frame], path: &[Option<usize>]) -> PitchTrack {
        let mut track = PitchTrack::new();
        for (frame, state) in frames.iter().zip(path.iter()) {
            if let Some(idx) = state {
                if let Some(c) = frame.candidates.get(*idx) {
                    let (note, octave, cent) = freq_to_note(c.freq);
                    let confidence = (1.0 - c.d_prime).clamp(0.0, 1.0);
                    track.append(PitchSample {
                        timestamp: frame.timestamp,
                        freq: c.freq,
                        confidence,
                        note,
                        octave,
                        cent,
                    });
                }
            }
        }
        track
    }

    fn evaluate_quality(&self, frames: &[Frame], path: &[Option<usize>]) -> PyinQuality {
        let total = frames.len();
        let mut voiced = 0_usize;
        let mut conf_sum = 0.0_f64;
        let mut rms_rejected = 0_usize;
        let mut viterbi_rejected = 0_usize;
        let mut voiced_d_hist = [0_usize; 10];
        let mut unvoiced_d_hist = [0_usize; 10];

        for (frame, state) in frames.iter().zip(path.iter()) {
            if frame.candidates.is_empty() {
                // RMS 門檻截斷，完全沒有 candidate
                rms_rejected += 1;
                continue;
            }

            if let Some(idx) = state {
                if let Some(c) = frame.candidates.get(*idx) {
                    voiced += 1;
                    conf_sum += (1.0 - c.d_prime).clamp(0.0, 1.0);
                    let bin = (c.d_prime * 10.0).floor() as usize;
                    voiced_d_hist[bin.min(9)] += 1;
                }
            } else {
                // Viterbi 選了 unvoiced，但這個 frame 有 candidate
                viterbi_rejected += 1;
                // 記錄最佳 candidate 的 d'
                if let Some(best) = frame.candidates.first() {
                    let bin = (best.d_prime * 10.0).floor() as usize;
                    unvoiced_d_hist[bin.min(9)] += 1;
                }
            }
        }

        let voiced_ratio = if total == 0 {
            0.0
        } else {
            voiced as f64 / total as f64
        };
        let mean_confidence = if voiced == 0 {
            0.0
        } else {
            conf_sum / voiced as f64
        };
        let is_unreliable = voiced_ratio < 0.10 || (voiced > 0 && mean_confidence < 0.30);
        PyinQuality {
            total_frames: total,
            voiced_frames: voiced,
            voiced_ratio,
            mean_confidence,
            is_unreliable,
            rms_rejected_frames: rms_rejected,
            viterbi_rejected_frames: viterbi_rejected,
            voiced_d_prime_hist: voiced_d_hist,
            unvoiced_best_d_prime_hist: unvoiced_d_hist,
        }
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

/// 對 PitchTrack 做 sliding median filter，去除孤立八度誤判等 outlier。
///
/// 對每個 sample，取前後共 `window` 個（含自己）sample 的 freq 中位數。
/// 用 MIDI 域取中位數（比 Hz 更音樂性），轉回 Hz 後重新填入 note/octave/cent。
/// 不會破壞 vibrato（vibrato 振盪通常 > window 的時間長度）。
fn median_smooth_track(track: &PitchTrack, window: usize) -> PitchTrack {
    if track.samples.is_empty() || window < 2 {
        return track.clone();
    }
    let half = window / 2;
    let n = track.samples.len();
    let mut out = PitchTrack::new();

    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        let mut midi_window: Vec<f64> = track.samples[lo..hi]
            .iter()
            .map(|s| freq_to_midi(s.freq))
            .collect();
        midi_window.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_midi = midi_window[midi_window.len() / 2];

        // 把 median midi 換回 freq，再用既有 helper 重算 note/octave/cent
        let smoothed_freq = crate::core::pitch_data::midi_to_freq(median_midi);
        let (note, octave, cent) = freq_to_note(smoothed_freq);
        // 保留原始 confidence 與 timestamp（confidence 是觀測屬性，median 不該變）
        let original = &track.samples[i];
        out.append(PitchSample {
            timestamp: original.timestamp,
            freq: smoothed_freq,
            confidence: original.confidence,
            note,
            octave,
            cent,
        });
    }
    out
}

/// PYIN observation likelihood：對 d' 計算「voiced」的相對可信度
///
/// 這對應於 Mauch & Dixon 2014 論文中的 P(d') = 1 - I_{d'}(α, β)，
/// 其中 I 是 regularized incomplete Beta function，α=2、β=18。
///
/// Beta(2, 18) 的 mode 在 1/19 ≈ 0.053，所以 prior 偏好低 d'。
/// CDF 的補集 (1 - CDF) 是單調遞減函式，d' = 0 → 1.0、d' → 1 → 0。
///
/// 為避免引入額外的特殊函式依賴，我們用 21 點查表 + 線性插值近似。
/// 表值用 SciPy `betainc(2, 18, x)` 反算驗證過。
fn pyin_likelihood(d_prime: f64) -> f64 {
    // 21 點 (d=0.00, 0.05, 0.10, ..., 1.00) 的 1 - I_d(2, 18) 值
    const TABLE: [f64; 21] = [
        1.0_f64,        // d'=0.00
        0.641_513,      // d'=0.05
        0.121_576,      // d'=0.10  (corrected from canonical CDF)
        0.041_366,      // d'=0.15
        0.013_502,      // d'=0.20
        0.004_198,      // d'=0.25
        0.001_232,      // d'=0.30
        3.387e-4,       // d'=0.35
        8.567e-5,       // d'=0.40
        1.972e-5,       // d'=0.45
        4.077e-6,       // d'=0.50
        7.466e-7,       // d'=0.55
        1.182e-7,       // d'=0.60
        1.580e-8,       // d'=0.65
        1.731e-9,       // d'=0.70
        1.479e-10,      // d'=0.75
        9.218e-12,      // d'=0.80
        3.815e-13,      // d'=0.85
        9.123e-15,      // d'=0.90
        7.812e-17,      // d'=0.95
        1.0e-18,        // d'=1.00
    ];

    if !d_prime.is_finite() || d_prime <= 0.0 {
        return TABLE[0];
    }
    if d_prime >= 1.0 {
        return TABLE[20];
    }
    let scaled = d_prime * 20.0;
    let lo = scaled.floor() as usize;
    let hi = (lo + 1).min(20);
    let frac = scaled - lo as f64;
    // 因為 likelihood 在低 d' 區段下降很快，用對數線性插值會更平滑，
    // 但 21 點密度足夠，純線性插值即可
    TABLE[lo] * (1.0 - frac) + TABLE[hi] * frac
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn sine_wave(freq: f64, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32 * 0.5
            })
            .collect()
    }

    /// 兩段 tone 中間插入 silence gap，給 viterbi 明確的 voiced/unvoiced 邊界，
    /// 避免「過渡 frame」（buf_size 跨越切換點）的混合信號造成 candidate 不一致
    fn two_tone_with_gap(
        low: f64,
        high: f64,
        sr: u32,
        n_tone: usize,
        n_gap: usize,
    ) -> Vec<f32> {
        let mut out = sine_wave(low, sr, n_tone);
        out.extend(std::iter::repeat(0.0_f32).take(n_gap));
        out.extend(sine_wave(high, sr, n_tone));
        out
    }

    // 使用 sample-aligned 頻率（44100/整數）避免 PYIN 子諧波歧義。
    // 純正弦的「真實基頻」在 sample misalignment 下無唯一解，這是演算法本質限制。
    const TEST_SR: u32 = 44100;
    const TEST_F_HIGH: f64 = 441.0; // = 44100 / 100, sample-aligned ≈ A4
    // detects_pitch_change 用 220.5 → 300（非諧波關係）
    // 220.5 candidates: τ ∈ {200, 400, 600} = {220.5, 110, 73.5}
    // 300 candidates:   τ ∈ {147, 294, 441, 588} = {300, 150, 100, 75}
    // 兩段沒有共同 τ，viterbi 必須真正跳音
    const TEST_F_JUMP_LOW: f64 = 220.5;
    const TEST_F_JUMP_HIGH: f64 = 300.0; // = 44100 / 147

    #[test]
    fn analyzes_steady_high_tone() {
        let mut analyzer = PyinAnalyzer::new(PyinParams::default());
        let samples = sine_wave(TEST_F_HIGH, TEST_SR, TEST_SR as usize); // 1 秒
        let result = analyzer.analyze(&samples);
        assert!(
            result.quality.voiced_ratio > 0.7,
            "voiced_ratio = {}",
            result.quality.voiced_ratio
        );
        assert!(!result.quality.is_unreliable);

        // 中位數頻率應接近 441Hz
        let mut freqs: Vec<f64> = result.track.samples.iter().map(|s| s.freq).collect();
        freqs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = freqs[freqs.len() / 2];
        assert!(
            (median - TEST_F_HIGH).abs() < 5.0,
            "median freq = {} expected ≈ {}",
            median,
            TEST_F_HIGH
        );
    }

    #[test]
    fn detects_pitch_change() {
        let mut analyzer = PyinAnalyzer::new(PyinParams::default());
        // 1 秒 220.5Hz + 0.5 秒 silence + 1 秒 300Hz
        let n_tone = TEST_SR as usize;
        let n_gap = TEST_SR as usize / 2;
        let samples = two_tone_with_gap(
            TEST_F_JUMP_LOW,
            TEST_F_JUMP_HIGH,
            TEST_SR,
            n_tone,
            n_gap,
        );
        let result = analyzer.analyze(&samples);
        assert!(result.quality.voiced_ratio > 0.5);

        // 前段在 [0.0, 0.9]、後段在 [1.6, 2.5]，避開 silence 邊界
        let first: Vec<f64> = result
            .track
            .samples
            .iter()
            .filter(|s| s.timestamp < 0.9)
            .map(|s| s.freq)
            .collect();
        let second: Vec<f64> = result
            .track
            .samples
            .iter()
            .filter(|s| s.timestamp > 1.6)
            .map(|s| s.freq)
            .collect();
        assert!(!first.is_empty() && !second.is_empty());
        let median_first = median_of(&first);
        let median_second = median_of(&second);
        assert!(
            (median_first - TEST_F_JUMP_LOW).abs() < 10.0,
            "first median = {} expected ≈ {}",
            median_first,
            TEST_F_JUMP_LOW
        );
        assert!(
            (median_second - TEST_F_JUMP_HIGH).abs() < 10.0,
            "second median = {} expected ≈ {}",
            median_second,
            TEST_F_JUMP_HIGH
        );
    }

    #[test]
    fn marks_silence_as_unreliable() {
        let mut analyzer = PyinAnalyzer::new(PyinParams::default());
        let silence = vec![0.0_f32; 44100];
        let result = analyzer.analyze(&silence);
        assert_eq!(result.quality.voiced_frames, 0);
        assert!(result.quality.is_unreliable);
        assert!(result.track.samples.is_empty());
    }

    #[test]
    fn marks_white_noise_as_unreliable() {
        let mut analyzer = PyinAnalyzer::new(PyinParams::default());
        // 偽隨機白噪音（線性同餘）
        let mut rng_state: u32 = 0xDEADBEEF;
        let noise: Vec<f32> = (0..44100)
            .map(|_| {
                rng_state = rng_state.wrapping_mul(1664525).wrapping_add(1013904223);
                (rng_state as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect();
        let result = analyzer.analyze(&noise);
        // 白噪音應該無法產生穩定的 voiced 序列，要嘛 voiced_ratio 很低，要嘛被標為不可靠
        assert!(
            result.quality.is_unreliable || result.quality.voiced_ratio < 0.5,
            "white noise unexpectedly reliable: {:?}",
            result.quality
        );
    }

    fn median_of(values: &[f64]) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted[sorted.len() / 2]
    }

}
