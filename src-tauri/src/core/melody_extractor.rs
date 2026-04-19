//! 從乾淨人聲軌提取 MelodyTrack（Phase 3-new-c 的核心演算法）
//!
//! # Pipeline
//!
//! ```text
//! vocals.wav
//!     ↓  load_media (symphonia)
//! f32 interleaved stereo
//!     ↓  down-mix to mono
//! Vec<f32> mono
//!     ↓  YIN frame-by-frame (hop=2048, buf=4096)
//! Vec<PitchSample>
//!     ↓  cluster_pitch_samples
//! Vec<MelodyNote>
//!     ↓  wrap
//! MelodyTrack
//! ```
//!
//! # 為什麼對乾淨人聲軌的 YIN 參數與 audio_engine 不同
//!
//! `audio_engine::analyze_backing_pitch_async` 處理的是**混音伴奏**，要面對
//! bass 干擾，所以用寬鬆閾值 0.20、加上 80 Hz 高通濾波、f_min=60。
//!
//! 這裡處理的是**乾淨人聲軌**（使用者已用 UVR5/Moises 分離好），完全沒有
//! bass 干擾。可以用更嚴格的參數：
//!
//! - **無高通濾波**：人聲基頻本來就在 80 Hz 以上，濾波反而可能削弱基頻
//! - **harmonic_threshold = 0.15**：比混音嚴格，voiced 判定更精確
//! - **f_min = 70 Hz**：排除極低頻噪音（若有）
//! - **f_max = 1200 Hz**：涵蓋完整女聲音域
//!
//! # 群聚演算法（PitchSample → MelodyNote）
//!
//! YIN 輸出的是 frame-by-frame 的連續 pitch samples（hop=2048 約 46 ms 一個），
//! 但 MelodyNote 是離散音符。需要把「連續穩定」的樣本合併成一個音符。
//!
//! 合併規則：
//! - **時間 gap**：連續兩個 sample 的 timestamp 差 > 100 ms 視為斷點 → 開新音符
//! - **音高 gap**：新樣本與當前音符 median 頻率差 > 50 cent → 開新音符
//! - **最短音符**：低於 100 ms 的群聚結果視為 noise 丟棄（保守策略）
//!
//! 音符的代表音高用 **median** 而非 mean，對 vibrato 與 outlier 更 robust。

use crate::core::crepe_engine;
use crate::core::media_loader::load_media;
use crate::core::melody_track::{MelodyNote, MelodySource, MelodyTrack};
use crate::core::pitch_data::{freq_to_midi, PitchSample};
use crate::core::pyin_engine::{PyinAnalyzer, PyinParams, PyinQuality};
use crate::error::AppError;
use std::path::PathBuf;

// ── YIN 參數（對純人聲優化）──────────────────────────────────────

const BUF_SIZE: usize = 4096;
const HOP_SIZE: usize = 2048;
const F_MIN_HZ: f64 = 70.0;
const F_MAX_HZ: f64 = 1200.0;
const HARMONIC_THRESHOLD: f64 = 0.45;
const RMS_THRESHOLD: f64 = 0.0003;

// ── 群聚參數 ──────────────────────────────────────────────────────

/// 超過此時間 gap 視為音符斷點（秒）。
/// 0.15 秒允許輕微的 voiced frame 中斷仍視為同一音符，避免曲線過碎。
const MAX_GAP_SECS: f64 = 0.15;

/// 超過此音高差視為新音符（百分之一半音）。
const MAX_CENT_DIFF: f64 = 50.0;

/// 短於此長度的群聚結果視為 noise，丟棄（秒）。
/// 0.08 秒比原本 0.10 寬容，保留短裝飾音，但仍過濾單點 noise。
const MIN_NOTE_SECS: f64 = 0.05;

/// CREPE confidence 門檻：低於此值的 frame 視為 unvoiced
const CREPE_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// CREPE hop（@16kHz）：160 = 10ms，離線分析用高密度
const CREPE_HOP: usize = 160;

/// 從一個乾淨人聲音檔提取 MelodyTrack。
///
/// 優先嘗試 CREPE AI 引擎，若失敗則 fallback 到 PYIN。
pub fn extract_melody_from_vocals(
    vocals_path: &str,
    model_dir: Option<&PathBuf>,
) -> Result<MelodyTrack, AppError> {
    let media = load_media(vocals_path)?;

    println!(
        "[melody_extractor] 載入完成: sr={} Hz, channels={}, duration={:.1}s, samples={}",
        media.sample_rate,
        media.channels,
        media.duration,
        media.samples.len()
    );

    // Down-mix 到 mono
    let mono = downmix_to_mono(&media.samples, media.channels as usize);

    // 嘗試 CREPE → fallback PYIN
    let (pitch_samples, voiced_ratio, is_crepe) = if let Some(dir) = model_dir {
        match try_crepe(&mono, media.sample_rate, dir) {
            Ok((samples, ratio)) => {
                println!("[melody_extractor] 使用 CREPE AI 引擎");
                (samples, ratio, true)
            }
            Err(e) => {
                println!("[melody_extractor] CREPE 失敗 ({}), fallback 到 PYIN", e);
                let (s, r) = fallback_pyin(&mono, media.sample_rate);
                (s, r, false)
            }
        }
    } else {
        println!("[melody_extractor] 無模型目錄，使用 PYIN");
        let (s, r) = fallback_pyin(&mono, media.sample_rate);
        (s, r, false)
    };

    if pitch_samples.is_empty() {
        return Err(AppError::Audio(
            "音高分析未偵測到任何音高，請確認輸入檔是否為乾淨的人聲軌".to_string(),
        ));
    }

    let total_duration_secs = media.duration;

    if is_crepe {
        // CREPE 模式：直接使用原始 PitchSample，保留自然音高曲線
        println!(
            "[melody_extractor] CREPE 原始樣本: {} 個 pitch sample（跳過群聚）",
            pitch_samples.len()
        );

        Ok(MelodyTrack {
            source: MelodySource::ImportedVocals {
                vocals_path: vocals_path.to_string(),
                note_count: pitch_samples.len(),
                voiced_ratio,
            },
            notes: Vec::new(),
            total_duration_secs,
            raw_pitch_track: Some(pitch_samples),
        })
    } else {
        // PYIN 模式：群聚成離散音符
        let notes = cluster_pitch_samples(&pitch_samples, media.sample_rate);

        if notes.is_empty() {
            return Err(AppError::Audio(format!(
                "群聚後無有效音符（總共 {} 個 pitch sample 但都過短）",
                pitch_samples.len()
            )));
        }

        println!(
            "[melody_extractor] 群聚結果: {} 個音符（從 {} 個 pitch sample）",
            notes.len(),
            pitch_samples.len()
        );

        Ok(MelodyTrack {
            source: MelodySource::ImportedVocals {
                vocals_path: vocals_path.to_string(),
                note_count: notes.len(),
                voiced_ratio,
            },
            notes,
            total_duration_secs,
            raw_pitch_track: None,
        })
    }
}

/// 嘗試用 CREPE 分析，回傳 (pitch_samples, voiced_ratio)。
fn try_crepe(
    mono: &[f32],
    sample_rate: u32,
    model_dir: &PathBuf,
) -> Result<(Vec<PitchSample>, f64), AppError> {
    let result = crepe_engine::analyze_offline(
        mono,
        sample_rate,
        CREPE_HOP,
        CREPE_CONFIDENCE_THRESHOLD,
        model_dir,
    )?;

    let voiced_ratio = result.quality.voiced_ratio;
    Ok((result.samples, voiced_ratio))
}

/// PYIN fallback：包含完整診斷輸出。
fn fallback_pyin(mono: &[f32], sample_rate: u32) -> (Vec<PitchSample>, f64) {
    let (pitch_samples, quality) = analyze_mono(mono, sample_rate);

    // ── 診斷輸出 ──
    println!("[melody_extractor] ═══ PYIN 診斷報告 ═══");
    println!("[melody_extractor] 總 frame 數: {}", quality.total_frames);
    println!(
        "[melody_extractor] Voiced frames: {} ({:.1}%)",
        quality.voiced_frames,
        quality.voiced_ratio * 100.0
    );
    println!(
        "[melody_extractor] RMS 門檻截斷 (靜音): {} frames ({:.1}%)",
        quality.rms_rejected_frames,
        if quality.total_frames > 0 {
            quality.rms_rejected_frames as f64 / quality.total_frames as f64 * 100.0
        } else {
            0.0
        }
    );
    println!(
        "[melody_extractor] Viterbi 判 unvoiced (有 candidate 但被拒): {} frames ({:.1}%)",
        quality.viterbi_rejected_frames,
        if quality.total_frames > 0 {
            quality.viterbi_rejected_frames as f64 / quality.total_frames as f64 * 100.0
        } else {
            0.0
        }
    );
    println!(
        "[melody_extractor] 平均信心度: {:.3}",
        quality.mean_confidence
    );

    println!("[melody_extractor] ── Voiced d' 分佈 ──");
    let d_labels = [
        "0.0-0.1", "0.1-0.2", "0.2-0.3", "0.3-0.4", "0.4-0.5", "0.5-0.6", "0.6-0.7", "0.7-0.8",
        "0.8-0.9", "0.9-1.0",
    ];
    for (i, label) in d_labels.iter().enumerate() {
        let count = quality.voiced_d_prime_hist[i];
        if count > 0 {
            let bar = "█".repeat(
                (count as f64 / quality.voiced_frames.max(1) as f64 * 40.0).ceil() as usize,
            );
            println!("[melody_extractor]   {}: {:>5} {}", label, count, bar);
        }
    }

    if quality.viterbi_rejected_frames > 0 {
        println!("[melody_extractor] ── Unvoiced (有 candidate 但被拒) 最佳 d' 分佈 ──");
        for (i, label) in d_labels.iter().enumerate() {
            let count = quality.unvoiced_best_d_prime_hist[i];
            if count > 0 {
                let bar = "░".repeat(
                    (count as f64 / quality.viterbi_rejected_frames.max(1) as f64 * 40.0).ceil()
                        as usize,
                );
                println!("[melody_extractor]   {}: {:>5} {}", label, count, bar);
            }
        }
    }

    println!("[melody_extractor] ── 參數快照 ──");
    println!(
        "[melody_extractor]   BUF={}, HOP={}, f=[{}-{}Hz], threshold={}, rms={}, unvoiced_cost=25.0",
        BUF_SIZE, HOP_SIZE, F_MIN_HZ, F_MAX_HZ, HARMONIC_THRESHOLD, RMS_THRESHOLD
    );
    println!("[melody_extractor] ═══════════════════════");

    (pitch_samples, quality.voiced_ratio)
}

// ── 公開 helpers（供 center_channel_cancel 等外部模組使用）─────────

/// 從已有的 PitchSample 向量建立 MelodyTrack。
///
/// CREPE 模式直接保留原始曲線，不做群聚。
pub fn pitch_samples_to_melody_track(
    samples: &[PitchSample],
    source_label: &str,
) -> Result<MelodyTrack, AppError> {
    if samples.is_empty() {
        return Err(AppError::Audio("音高分析未偵測到任何音高".to_string()));
    }

    let total_duration = samples.last().map(|s| s.timestamp + 0.01).unwrap_or(0.0);

    let voiced_ratio = 1.0; // 已經過 CREPE 過濾，全都是 voiced

    Ok(MelodyTrack {
        source: MelodySource::ImportedVocals {
            vocals_path: source_label.to_string(),
            note_count: samples.len(),
            voiced_ratio,
        },
        notes: Vec::new(),
        total_duration_secs: total_duration,
        raw_pitch_track: Some(samples.to_vec()),
    })
}

/// 從已有的 mono f32 樣本用 PYIN 提取 MelodyTrack。
///
/// 供 center_channel_cancel fallback 使用（無 CREPE 模型時）。
pub fn extract_melody_from_mono_samples(
    mono: &[f32],
    sample_rate: u32,
) -> Result<MelodyTrack, AppError> {
    let (pitch_samples, _quality) = analyze_mono(mono, sample_rate);

    if pitch_samples.is_empty() {
        return Err(AppError::Audio("PYIN 音高分析未偵測到任何音高".to_string()));
    }

    let notes = cluster_pitch_samples(&pitch_samples, sample_rate);
    if notes.is_empty() {
        return Err(AppError::Audio("群聚後無有效音符".to_string()));
    }

    let total_duration = mono.len() as f64 / sample_rate as f64;

    Ok(MelodyTrack {
        source: MelodySource::ImportedVocals {
            vocals_path: "center_cancel_pyin".to_string(),
            note_count: notes.len(),
            voiced_ratio: _quality.voiced_ratio,
        },
        notes,
        total_duration_secs: total_duration,
        raw_pitch_track: None,
    })
}

// ── 內部 helpers ──────────────────────────────────────────────────

fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// 對 mono 訊號跑離線 PYIN，回傳 (pitch samples, quality)。
fn analyze_mono(mono: &[f32], sample_rate: u32) -> (Vec<PitchSample>, PyinQuality) {
    let mut params = PyinParams::default();
    params.sample_rate = sample_rate;
    params.buf_size = BUF_SIZE;
    params.hop = HOP_SIZE;
    params.f_min = F_MIN_HZ;
    params.f_max = F_MAX_HZ;
    params.max_threshold = HARMONIC_THRESHOLD;
    params.rms_threshold = RMS_THRESHOLD;
    params.unvoiced_cost = 25.0; // 強制 Viterbi 在伴奏干擾時仍偏好保留音高，而不是切斷為 unvoiced

    let mut analyzer = PyinAnalyzer::new(params);
    let result = analyzer.analyze(mono);

    (result.track.samples, result.quality)
}

/// 把連續穩定的 PitchSample 群聚成離散 MelodyNote。
///
/// 演算法見模組 doc 的「群聚演算法」段落。
fn cluster_pitch_samples(samples: &[PitchSample], sample_rate: u32) -> Vec<MelodyNote> {
    let hop_secs = HOP_SIZE as f64 / sample_rate as f64;
    let mut notes: Vec<MelodyNote> = Vec::new();
    if samples.is_empty() {
        return notes;
    }

    // 當前正在累積的音符狀態
    let mut current_start: f64 = samples[0].timestamp;
    let mut current_last_timestamp: f64 = samples[0].timestamp;
    let mut current_freqs: Vec<f64> = vec![samples[0].freq];

    for s in samples.iter().skip(1) {
        let gap = s.timestamp - current_last_timestamp;
        let current_median = median_freq(&current_freqs);
        let cent_diff = 1200.0 * (s.freq / current_median).log2().abs();

        let should_break = gap > MAX_GAP_SECS || cent_diff > MAX_CENT_DIFF;

        if should_break {
            // Flush 當前音符（若夠長）
            push_note_if_long_enough(
                &mut notes,
                current_start,
                current_last_timestamp,
                &current_freqs,
                hop_secs,
            );

            // 重設狀態開新音符
            current_start = s.timestamp;
            current_last_timestamp = s.timestamp;
            current_freqs.clear();
            current_freqs.push(s.freq);
        } else {
            current_last_timestamp = s.timestamp;
            current_freqs.push(s.freq);
        }
    }

    // Flush 最後一個音符
    push_note_if_long_enough(
        &mut notes,
        current_start,
        current_last_timestamp,
        &current_freqs,
        hop_secs,
    );

    notes
}

/// 若音符長度 ≥ MIN_NOTE_SECS，把它加進 notes 清單。
fn push_note_if_long_enough(
    notes: &mut Vec<MelodyNote>,
    start: f64,
    last_timestamp: f64,
    freqs: &[f64],
    hop_secs: f64,
) {
    // 音符 duration = 最後一個 sample 的 timestamp + 一個 hop 的長度
    //（因為最後一個 sample 還佔 hop 的時間）
    let duration = (last_timestamp - start).max(0.0) + hop_secs;
    if duration < MIN_NOTE_SECS || freqs.is_empty() {
        return;
    }

    let median = median_freq(freqs);
    let midi_float = freq_to_midi(median);
    if !midi_float.is_finite() {
        return;
    }
    let midi_pitch = midi_float.round().clamp(0.0, 127.0) as u8;

    notes.push(MelodyNote::from_midi(
        start, duration, midi_pitch, None,  // lyric: 乾淨人聲軌沒有標注
        false, // is_golden
        false, // is_freestyle
    ));
}

/// 計算頻率陣列的中位數（對 vibrato 與 outlier robust）。
fn median_freq(freqs: &[f64]) -> f64 {
    if freqs.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = freqs.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) * 0.5
    } else {
        sorted[mid]
    }
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pitch_data::{freq_to_note, PitchSample};

    fn make_sample(timestamp: f64, freq: f64) -> PitchSample {
        let (note, octave, cent) = freq_to_note(freq);
        PitchSample {
            timestamp,
            freq,
            confidence: 0.9,
            note,
            octave,
            cent,
        }
    }

    #[test]
    fn median_freq_handles_odd_count() {
        assert!((median_freq(&[440.0, 220.0, 880.0]) - 440.0).abs() < 1e-9);
    }

    #[test]
    fn median_freq_handles_even_count() {
        assert!((median_freq(&[400.0, 500.0]) - 450.0).abs() < 1e-9);
    }

    #[test]
    fn median_freq_empty_returns_zero() {
        assert_eq!(median_freq(&[]), 0.0);
    }

    #[test]
    fn downmix_averages_stereo_to_mono() {
        let stereo = [1.0_f32, 3.0, 2.0, 4.0];
        let mono = downmix_to_mono(&stereo, 2);
        assert_eq!(mono, vec![2.0, 3.0]);
    }

    #[test]
    fn downmix_passthrough_for_mono_input() {
        let samples = vec![0.1_f32, 0.2, 0.3];
        let mono = downmix_to_mono(&samples, 1);
        assert_eq!(mono, samples);
    }

    #[test]
    fn cluster_empty_samples_returns_empty() {
        assert!(cluster_pitch_samples(&[], 44100).is_empty());
    }

    #[test]
    fn cluster_single_note_of_stable_pitch() {
        // 440 Hz 維持 0.5 秒，每 46 ms 一個 sample
        let samples: Vec<PitchSample> = (0..11)
            .map(|i| make_sample(i as f64 * 0.046, 440.0))
            .collect();

        let notes = cluster_pitch_samples(&samples, 44100);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].midi_pitch, 69); // A4
        assert!(notes[0].start_secs.abs() < 1e-9);
        // 長度應該覆蓋從 0 到 ~0.46 秒 + 一個 hop
        assert!(notes[0].duration_secs > 0.4);
    }

    #[test]
    fn cluster_splits_on_large_time_gap() {
        // 兩段 440 Hz，中間 200 ms gap（超過 MAX_GAP_SECS 0.1）
        let first: Vec<PitchSample> = (0..5)
            .map(|i| make_sample(i as f64 * 0.046, 440.0))
            .collect();
        let second: Vec<PitchSample> = (0..5)
            .map(|i| make_sample(0.5 + i as f64 * 0.046, 440.0))
            .collect();
        let mut all = first;
        all.extend(second);

        let notes = cluster_pitch_samples(&all, 44100);
        assert_eq!(notes.len(), 2, "時間 gap 應該造成斷點");
        assert_eq!(notes[0].midi_pitch, 69);
        assert_eq!(notes[1].midi_pitch, 69);
    }

    #[test]
    fn cluster_splits_on_large_pitch_jump() {
        // 440 Hz 唱一段，然後立刻跳到 880 Hz（高八度 = 1200 cent，遠超 50 cent）
        let first: Vec<PitchSample> = (0..5)
            .map(|i| make_sample(i as f64 * 0.046, 440.0))
            .collect();
        let second: Vec<PitchSample> = (0..5)
            .map(|i| make_sample(0.23 + i as f64 * 0.046, 880.0))
            .collect();
        let mut all = first;
        all.extend(second);

        let notes = cluster_pitch_samples(&all, 44100);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].midi_pitch, 69); // A4
        assert_eq!(notes[1].midi_pitch, 81); // A5
    }

    #[test]
    fn cluster_discards_too_short_notes() {
        // 只有 1 個 sample、duration 約 46.4 ms < MIN_NOTE_SECS (50 ms)
        let samples: Vec<PitchSample> = (0..1)
            .map(|i| make_sample(i as f64 * 0.023, 440.0))
            .collect();
        let notes = cluster_pitch_samples(&samples, 44100);
        assert!(notes.is_empty(), "短於 MIN_NOTE_SECS 的音符應該被丟棄");
    }

    #[test]
    fn cluster_uses_median_for_robust_pitch() {
        // 大部分 440 Hz + 一個離群的 880 Hz（但在 50 cent 內，不會斷開）
        // 由於 880 Hz 跟 440 Hz 差一個八度（1200 cent），實際上會斷開
        // 所以這個測試改成驗證「微小 drift 用 median 穩定」
        //
        // 441 Hz × 3 + 443 Hz + 440 Hz → median 應該是 441
        let samples = [
            make_sample(0.00, 441.0),
            make_sample(0.05, 441.0),
            make_sample(0.10, 441.0),
            make_sample(0.15, 443.0),
            make_sample(0.20, 440.0),
        ];
        let notes = cluster_pitch_samples(&samples, 44100);
        assert_eq!(notes.len(), 1);
        // Median 441 Hz → MIDI 69.04 → round 到 69 = A4
        assert_eq!(notes[0].midi_pitch, 69);
    }

    #[test]
    fn cluster_handles_tight_pitch_drift_without_splitting() {
        // 440 Hz → 445 Hz 是 19.6 cent，不應該斷
        let samples = [
            make_sample(0.00, 440.0),
            make_sample(0.05, 442.0),
            make_sample(0.10, 444.0),
            make_sample(0.15, 445.0),
        ];
        let notes = cluster_pitch_samples(&samples, 44100);
        assert_eq!(notes.len(), 1);
    }
}
