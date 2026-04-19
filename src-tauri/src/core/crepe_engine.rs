//! CREPE AI 音高偵測引擎
//!
//! 使用 CREPE tiny ONNX 模型（~1.9MB, MIT license）做音高偵測，
//! 準確度遠超 PYIN，對分離人聲的 reverb artifact 更 robust。
//!
//! # 模型規格
//!
//! - Input:  `[n_frames, 1024]` f32 — 每 frame 1024 samples @16kHz = 64ms
//! - Output: `[n_frames, 360]`  f32 — 360 bin 的機率分佈（C1~B7, 每 bin 20 cent）
//! - Opset:  v10
//!
//! # 頻率解碼
//!
//! CREPE 輸出 360 個 bin，對應 MIDI 音高 [0, 7200) cent（從 C1 = MIDI 24 開始）。
//! bin_i 對應 cent = i * 20，頻率 = 10.0 * 2^(cent / 1200)。
//! 使用加權平均（weighted average around peak）取得亞 bin 精度。

use crate::core::pitch_data::{freq_to_note, PitchSample};
use crate::core::resampler::resample_offline;
use crate::error::AppError;
use ort::session::Session;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// CREPE 要求的輸入 sample rate
const CREPE_SR: u32 = 16000;

/// 每個 frame 的 sample 數
const FRAME_SIZE: usize = 1024;

/// CREPE 的 bin 數量（C1 ~ B7, 每 bin 20 cent）
const N_BINS: usize = 360;

/// CREPE 的起始 MIDI（C1 = MIDI 24）
#[allow(dead_code)]
const MIDI_OFFSET: f64 = 24.0;

/// 每個 bin 的 cent 寬度
const CENTS_PER_BIN: f64 = 20.0;

/// CREPE cent 偏移量：bin 0 對應 C1 (32.703 Hz)，
/// 而 freq = 10.0 * 2^(cent / 1200) 中 cent 0 = 10 Hz，
/// 所以 bin 0 的絕對 cent = 1200 * log2(32.703 / 10.0) ≈ 1997.38。
/// 來源：marl/crepe 官方 Python 實作的 `to_local_average_cents()`。
const CENT_OFFSET: f64 = 1997.3794084376191;

/// 全域 ONNX session（OnceLock + Mutex：OnceLock 保證只初始化一次，
/// Mutex 提供 run() 需要的 &mut 存取）
static CREPE_SESSION: OnceLock<Result<Mutex<Session>, String>> = OnceLock::new();

/// 離線分析結果
pub struct CrepeResult {
    /// 音高 samples（含 timestamp, freq, confidence）
    pub samples: Vec<PitchSample>,
    /// 品質統計
    pub quality: CrepeQuality,
}

/// 品質指標
#[derive(Debug, Clone)]
pub struct CrepeQuality {
    pub total_frames: usize,
    pub voiced_frames: usize,
    pub voiced_ratio: f64,
    pub mean_confidence: f64,
}

/// 嘗試取得全域 CREPE session（lazy 初始化）。
///
/// `model_dir` 是模型檔所在的目錄（通常是 app resource dir）。
fn get_or_init_session(model_dir: &Path) -> Result<&'static Mutex<Session>, String> {
    let result = CREPE_SESSION.get_or_init(|| {
        let model_path = model_dir.join("crepe-tiny.onnx");
        if !model_path.exists() {
            return Err(format!("CREPE 模型檔不存在: {}", model_path.display()));
        }

        println!(
            "[crepe_engine] 載入模型: {} ({:.1} KB)",
            model_path.display(),
            model_path.metadata().map(|m| m.len()).unwrap_or(0) as f64 / 1024.0
        );

        let build_result = (|| -> Result<Session, ort::Error> {
            let mut builder = Session::builder()?;
            builder = builder.with_intra_threads(2)?;
            builder.commit_from_file(&model_path)
        })();

        match build_result {
            Ok(session) => {
                println!("[crepe_engine] 模型載入成功");
                Ok(Mutex::new(session))
            }
            Err(e) => Err(format!("ONNX session 建立失敗: {}", e)),
        }
    });

    match result {
        Ok(session) => Ok(session),
        Err(e) => Err(e.clone()),
    }
}

/// 取得 CREPE 模型目錄（dev 模式在 src-tauri/models/，production 在 exe 同目錄）。
///
/// 與 `melody_commands::get_model_dir` 邏輯相同，但提升為公開函式供
/// `audio_engine` 等模組使用。
pub fn find_crepe_model_dir() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();

    // Dev 模式：exe 在 target/debug/ 下
    let dev_path = exe_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("models"));
    if let Some(ref p) = dev_path {
        if p.join("crepe-tiny.onnx").exists() {
            return dev_path;
        }
    }

    // Production：models/ 在 exe 同目錄
    let prod_path = exe_dir.join("models");
    if prod_path.join("crepe-tiny.onnx").exists() {
        return Some(prod_path);
    }

    // 最後嘗試 exe 同目錄
    if exe_dir.join("crepe-tiny.onnx").exists() {
        return Some(exe_dir);
    }

    None
}

/// CREPE 要求的輸入 sample rate（公開供 audio_engine 使用）
pub const CREPE_SAMPLE_RATE: u32 = CREPE_SR;

/// CREPE 每 frame 的 sample 數（公開供 audio_engine 使用）
pub const CREPE_FRAME_SIZE: usize = FRAME_SIZE;

/// 即時單 frame 推論：餵入 1024 個 @16kHz 的 mono samples，回傳 Option<PitchSample>。
///
/// 與離線版的差異：
/// - 不做後處理（八度修正、中位數濾波、音域過濾需要上下文）
/// - 只做 RMS 門檻 + confidence 門檻
/// - 設計為每 ~10ms 呼叫一次，延遲 < 5ms
///
/// # Arguments
/// - `frame`: 長度必須 == 1024 的 @16kHz mono samples
/// - `timestamp`: 時間戳（秒），會寫入回傳的 PitchSample
/// - `confidence_threshold`: 低於此 confidence 視為 unvoiced
/// - `model_dir`: 模型檔所在目錄
pub fn detect_realtime(
    frame: &[f32],
    timestamp: f64,
    confidence_threshold: f64,
    model_dir: &Path,
) -> Result<Option<PitchSample>, AppError> {
    if frame.len() != FRAME_SIZE {
        return Err(AppError::Audio(format!(
            "CREPE 即時推論需要 {} samples，收到 {}",
            FRAME_SIZE,
            frame.len()
        )));
    }

    // RMS 門檻：避免對靜音/殘響 frame 推論
    const MIN_FRAME_RMS: f64 = 0.003;
    let rms =
        (frame.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / FRAME_SIZE as f64).sqrt();
    if rms < MIN_FRAME_RMS {
        return Ok(None);
    }

    // 取得 session
    let session_mutex = get_or_init_session(model_dir)
        .map_err(|e| AppError::Audio(format!("CREPE 初始化失敗: {}", e)))?;
    let mut session = session_mutex
        .lock()
        .map_err(|e| AppError::Audio(format!("CREPE session lock 失敗: {}", e)))?;

    // 正規化
    let max_abs = frame.iter().fold(0.0_f32, |acc, &x| acc.max(x.abs()));
    let normalized: Vec<f32> = if max_abs > 1e-8 {
        frame.iter().map(|&x| x / max_abs).collect()
    } else {
        return Ok(None);
    };

    // 推論（單 frame, batch_size=1）
    let shape = vec![1_usize, FRAME_SIZE];
    let input_tensor = ort::value::Value::from_array((shape, normalized))
        .map_err(|e| AppError::Audio(format!("CREPE tensor 建立失敗: {}", e)))?;

    let outputs = session
        .run(ort::inputs![input_tensor])
        .map_err(|e| AppError::Audio(format!("CREPE 推論失敗: {}", e)))?;

    let output = &outputs[0];
    let tensor = output
        .try_extract_tensor::<f32>()
        .map_err(|e| AppError::Audio(format!("CREPE output 解析失敗: {}", e)))?;

    let probabilities = tensor.1;
    if probabilities.len() != N_BINS {
        return Ok(None);
    }

    let (freq, confidence) = decode_pitch(probabilities);

    if confidence < confidence_threshold || freq <= 0.0 {
        return Ok(None);
    }

    // 硬音域邊界過濾
    if freq < HARD_VOCAL_LOW_HZ || freq > HARD_VOCAL_HIGH_HZ {
        return Ok(None);
    }

    let (note, octave, cent) = freq_to_note(freq);
    Ok(Some(PitchSample {
        timestamp,
        freq,
        confidence,
        note,
        octave,
        cent,
    }))
}

/// 離線整段分析：對 mono 音訊跑 CREPE，回傳所有偵測到的 PitchSample。
///
/// # Arguments
/// - `mono`: 原始 mono samples（任意 sample rate）
/// - `sample_rate`: 原始 sample rate
/// - `hop`: 每次滑動的 sample 數（@16kHz），控制時間解析度。
///   - 160 = 10ms（高密度，離線推薦）
///   - 512 = 32ms（即時推薦）
/// - `confidence_threshold`: 低於此 confidence 的 frame 視為 unvoiced
/// - `model_dir`: 模型檔所在目錄
pub fn analyze_offline(
    mono: &[f32],
    sample_rate: u32,
    hop: usize,
    confidence_threshold: f64,
    model_dir: &Path,
) -> Result<CrepeResult, AppError> {
    // 1. 取得 session（Mutex guard）
    let session_mutex = get_or_init_session(model_dir)
        .map_err(|e| AppError::Audio(format!("CREPE 初始化失敗: {}", e)))?;
    let mut session = session_mutex
        .lock()
        .map_err(|e| AppError::Audio(format!("CREPE session lock 失敗: {}", e)))?;

    // 2. Resample 到 16kHz
    let resampled = resample_offline(mono, sample_rate, CREPE_SR);
    println!(
        "[crepe_engine] 重採樣: {}Hz → {}Hz ({} → {} samples)",
        sample_rate,
        CREPE_SR,
        mono.len(),
        resampled.len()
    );

    // 3. 切 frames
    let frames = slice_frames(&resampled, hop);
    let total_frames = frames.len();
    println!("[crepe_engine] 總 frame 數: {} (hop={})", total_frames, hop);

    if total_frames == 0 {
        return Ok(CrepeResult {
            samples: Vec::new(),
            quality: CrepeQuality {
                total_frames: 0,
                voiced_frames: 0,
                voiced_ratio: 0.0,
                mean_confidence: 0.0,
            },
        });
    }

    // 4. Batch 推論（一次送所有 frames）
    let probabilities = run_inference(&mut session, &frames, total_frames)?;

    // 5. 解碼頻率 + confidence（兩遍：先收集再用自適應門檻過濾）
    let mut raw_decoded: Vec<(f64, f64, f64)> = Vec::new(); // (timestamp, freq, confidence)

    for (i, prob_row) in probabilities.chunks_exact(N_BINS).enumerate() {
        let timestamp = i as f64 * hop as f64 / CREPE_SR as f64;
        let (freq, confidence) = decode_pitch(prob_row);
        if freq > 0.0 {
            raw_decoded.push((timestamp, freq, confidence));
        }
    }

    // 自適應信心門檻：取 p25 percentile，用動態上限避免高品質音訊丟失 voiced sample
    // 下限固定 0.25（防止噪音太多），上限根據平均信心度動態計算
    let adaptive_threshold = if raw_decoded.len() >= 10 {
        let mut confs: Vec<f64> = raw_decoded.iter().map(|&(_, _, c)| c).collect();
        confs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p25_idx = (confs.len() as f64 * 0.25) as usize;
        let p25 = confs[p25_idx];
        let mean_conf: f64 = confs.iter().sum::<f64>() / confs.len() as f64;
        // 上限 = 平均信心度 × 0.7，最高不超過 0.65
        let upper_clamp = (mean_conf * 0.7).min(0.65);
        let adaptive = p25.clamp(0.25, upper_clamp.max(0.3));
        println!(
            "[crepe_engine] 自適應信心門檻: {:.3} (p25={:.3}, mean={:.3}, upper_clamp={:.3})",
            adaptive, p25, mean_conf, upper_clamp
        );
        adaptive
    } else {
        confidence_threshold
    };

    let mut samples = Vec::new();
    let mut voiced_count = 0_usize;

    for &(timestamp, freq, confidence) in &raw_decoded {
        if confidence >= adaptive_threshold {
            let (note, octave, cent) = freq_to_note(freq);
            samples.push(PitchSample {
                timestamp,
                freq,
                confidence,
                note,
                octave,
                cent,
            });
            voiced_count += 1;
        }
    }

    let voiced_ratio = if total_frames > 0 {
        voiced_count as f64 / total_frames as f64
    } else {
        0.0
    };

    // 6. 八度錯誤修正：CREPE 偶爾會鎖定泛音導致頻率跳高/低一個八度，
    //    偵測後修正回正確八度（而非直接刪除，保留更多 voiced 資料）
    let samples = fix_octave_errors(samples);

    // 7. 中位數濾波：移除孤立的音高 spike（泛音/噪音誤判）
    let samples = median_filter_pitch(samples, 5);

    // 8. 音域範圍過濾：根據整首歌的實際演唱音域，過濾超出範圍的異常值
    let (samples, vocal_range) = filter_by_vocal_range(samples);

    // 重新計算統計
    let voiced_count = samples.len();
    let conf_sum: f64 = samples.iter().map(|s| s.confidence).sum();
    let mean_confidence = if voiced_count > 0 {
        conf_sum / voiced_count as f64
    } else {
        0.0
    };

    if let Some(range) = &vocal_range {
        println!(
            "[crepe_engine] 偵測音域: {:.1} Hz ~ {:.1} Hz (MIDI {:.1} ~ {:.1}), 中位 {:.1} Hz",
            range.low_freq, range.high_freq, range.low_midi, range.high_midi, range.median_freq
        );
    }

    println!(
        "[crepe_engine] 最終 Voiced: {} / {} ({:.1}%), 平均 confidence: {:.3}",
        voiced_count,
        total_frames,
        voiced_ratio * 100.0,
        mean_confidence
    );

    Ok(CrepeResult {
        samples,
        quality: CrepeQuality {
            total_frames,
            voiced_frames: voiced_count,
            voiced_ratio,
            mean_confidence,
        },
    })
}

// ── 內部 helpers ──────────────────────────────────────────────────

/// 把連續音訊切成重疊的 frames，每 frame 1024 samples。
fn slice_frames(audio: &[f32], hop: usize) -> Vec<Vec<f32>> {
    let mut frames = Vec::new();
    let mut start = 0;
    while start + FRAME_SIZE <= audio.len() {
        frames.push(audio[start..start + FRAME_SIZE].to_vec());
        start += hop;
    }
    frames
}

/// 每批最多處理的 frame 數（避免超長音樂佔用過多記憶體）
const MAX_BATCH_FRAMES: usize = 500;

/// 執行 ONNX 推論，回傳 [n_frames * 360] 的機率。
/// 自動分批處理，避免超長音樂的記憶體問題。
fn run_inference(
    session: &mut Session,
    frames: &[Vec<f32>],
    n_frames: usize,
) -> Result<Vec<f32>, AppError> {
    let mut all_probs: Vec<f32> = Vec::with_capacity(n_frames * N_BINS);

    for chunk in frames.chunks(MAX_BATCH_FRAMES) {
        let batch_size = chunk.len();

        // 展平 + 正規化（加 RMS 門檻避免弱音段噪音放大）
        // -50 dBFS ≈ 0.003，低於此的 frame 幾乎確定不是演唱段落
        const MIN_FRAME_RMS: f64 = 0.003;

        let mut flat: Vec<f32> = Vec::with_capacity(batch_size * FRAME_SIZE);
        for frame in chunk {
            let rms = (frame.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>()
                / FRAME_SIZE as f64)
                .sqrt();

            if rms < MIN_FRAME_RMS {
                // 靜音/殘響/呼吸 frame，直接填零避免噪音被放大
                flat.extend(std::iter::repeat(0.0_f32).take(FRAME_SIZE));
            } else {
                let max_abs = frame.iter().fold(0.0_f32, |acc, &x| acc.max(x.abs()));
                if max_abs > 1e-8 {
                    flat.extend(frame.iter().map(|&x| x / max_abs));
                } else {
                    flat.extend(std::iter::repeat(0.0_f32).take(FRAME_SIZE));
                }
            }
        }

        let shape = vec![batch_size, FRAME_SIZE];
        let input_tensor = ort::value::Value::from_array((shape, flat))
            .map_err(|e| AppError::Audio(format!("CREPE tensor 建立失敗: {}", e)))?;

        let outputs = session
            .run(ort::inputs![input_tensor])
            .map_err(|e| AppError::Audio(format!("CREPE 推論失敗: {}", e)))?;

        let output = &outputs[0];
        let tensor = output
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::Audio(format!("CREPE output 解析失敗: {}", e)))?;

        all_probs.extend_from_slice(tensor.1);
    }

    Ok(all_probs)
}

/// 偵測到的演唱音域資訊
#[derive(Debug, Clone)]
pub struct VocalRange {
    /// 音域下限頻率（5th percentile）
    pub low_freq: f64,
    /// 音域上限頻率（95th percentile）
    pub high_freq: f64,
    /// 中位數頻率
    pub median_freq: f64,
    /// 對應的 MIDI 值
    pub low_midi: f64,
    pub high_midi: f64,
}

/// 人聲基頻的絕對硬邊界（Hz）。
/// 低於 55 Hz (A1) 或高於 1400 Hz (F6) 幾乎確定不是人聲基頻。
const HARD_VOCAL_LOW_HZ: f64 = 55.0;
const HARD_VOCAL_HIGH_HZ: f64 = 1400.0;

/// 根據整首歌的音高分佈，過濾超出合理音域的 sample。
///
/// 步驟：
/// 1. 先用硬邊界 55~1400 Hz 過濾掉明顯不是人聲的頻率
/// 2. 收集所有頻率，排序後取 5th ~ 95th percentile 作為核心音域
/// 3. 向外再擴展 300 cent（2.5 個半音）作為容許邊界
/// 4. 超出邊界的 sample 被移除
fn filter_by_vocal_range(samples: Vec<PitchSample>) -> (Vec<PitchSample>, Option<VocalRange>) {
    // 第一階段：硬邊界過濾（無論 sample 數量多少都執行）
    let samples: Vec<PitchSample> = samples
        .into_iter()
        .filter(|s| s.freq >= HARD_VOCAL_LOW_HZ && s.freq <= HARD_VOCAL_HIGH_HZ)
        .collect();

    // 需要足夠的 sample 做統計（50 個 = 500ms @10ms hop）
    if samples.len() < 50 {
        return (samples, None);
    }

    let mut freqs: Vec<f64> = samples.iter().map(|s| s.freq).collect();
    freqs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let p5_idx = (freqs.len() as f64 * 0.05) as usize;
    let p50_idx = freqs.len() / 2;
    let p95_idx = (freqs.len() as f64 * 0.95) as usize;

    let p5_freq = freqs[p5_idx];
    let p50_freq = freqs[p50_idx];
    let p95_freq = freqs[p95_idx.min(freqs.len() - 1)];

    // 向外擴展 300 cent（2.5 半音）作為容許邊界
    let expand_ratio = 2.0_f64.powf(300.0 / 1200.0); // ≈ 1.189
    let low_bound = p5_freq / expand_ratio;
    let high_bound = p95_freq * expand_ratio;

    let range = VocalRange {
        low_freq: p5_freq,
        high_freq: p95_freq,
        median_freq: p50_freq,
        low_midi: freq_to_midi_f64(p5_freq),
        high_midi: freq_to_midi_f64(p95_freq),
    };

    let filtered: Vec<PitchSample> = samples
        .into_iter()
        .filter(|s| s.freq >= low_bound && s.freq <= high_bound)
        .collect();

    (filtered, Some(range))
}

/// 頻率轉 MIDI（浮點數版本，用於 log 輸出）
fn freq_to_midi_f64(freq: f64) -> f64 {
    if freq <= 0.0 {
        return 0.0;
    }
    69.0 + 12.0 * (freq / 440.0).log2()
}

/// 八度錯誤修正：CREPE 偶爾鎖定泛音，導致頻率跳高或跳低一個八度。
///
/// 策略：
/// 1. 先計算全域中位數頻率，作為段落起始的 fallback 參考
/// 2. 用 ±100ms 時間窗內的鄰居中位數作為「期望頻率」
/// 3. 若鄰居不足 3 個（段落邊界），用全域中位數代替
/// 4. 偏離 850~1350 cent（接近一個八度）的 sample，修正頻率 ÷2 或 ×2
fn fix_octave_errors(mut samples: Vec<PitchSample>) -> Vec<PitchSample> {
    if samples.len() < 5 {
        return samples;
    }

    // 計算全域中位數頻率（用於段落起始 fallback）
    let mut all_freqs: Vec<f64> = samples.iter().map(|s| s.freq).collect();
    all_freqs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let global_median = all_freqs[all_freqs.len() / 2];

    let freqs: Vec<f64> = samples.iter().map(|s| s.freq).collect();
    let timestamps: Vec<f64> = samples.iter().map(|s| s.timestamp).collect();

    /// 鄰居的最大時間距離（100ms）
    const MAX_NEIGHBOR_DISTANCE_SECS: f64 = 0.1;

    let mut corrections = 0_usize;

    for i in 0..samples.len() {
        let t_i = timestamps[i];

        // 收集 ±100ms 時間窗內的鄰居（排除自己）
        let mut neighbors: Vec<f64> = Vec::new();
        // 向前搜尋
        for j in (0..i).rev() {
            if t_i - timestamps[j] > MAX_NEIGHBOR_DISTANCE_SECS {
                break;
            }
            neighbors.push(freqs[j]);
        }
        // 向後搜尋
        for j in (i + 1)..samples.len() {
            if timestamps[j] - t_i > MAX_NEIGHBOR_DISTANCE_SECS {
                break;
            }
            neighbors.push(freqs[j]);
        }

        // 若鄰居不足 3 個（段落起始/結尾），用全域中位數作為參考
        let median = if neighbors.len() >= 3 {
            neighbors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            neighbors[neighbors.len() / 2]
        } else {
            global_median
        };

        let cent_diff = 1200.0 * (freqs[i] / median).log2();

        // 偏高約一個八度（850~1350 cent）→ 除以 2
        if cent_diff > 850.0 && cent_diff < 1350.0 {
            let corrected_freq = freqs[i] / 2.0;
            let (note, octave, cent) = freq_to_note(corrected_freq);
            samples[i].freq = corrected_freq;
            samples[i].note = note;
            samples[i].octave = octave;
            samples[i].cent = cent;
            corrections += 1;
        }
        // 偏低約一個八度（-1350~-850 cent）→ 乘以 2
        else if cent_diff < -850.0 && cent_diff > -1350.0 {
            let corrected_freq = freqs[i] * 2.0;
            let (note, octave, cent) = freq_to_note(corrected_freq);
            samples[i].freq = corrected_freq;
            samples[i].note = note;
            samples[i].octave = octave;
            samples[i].cent = cent;
            corrections += 1;
        }
    }

    if corrections > 0 {
        println!(
            "[crepe_engine] 八度錯誤修正: {} 個 sample 被修正（全域中位數 {:.1} Hz）",
            corrections, global_median
        );
    }

    samples
}

/// 中位數濾波：若某個 sample 的頻率與前後鄰居的中位數差距超過 120 cent，
/// 視為 spike 移除。`window` 是單側鄰居數（window=5 → 看前後各 2 個有效點）。
fn median_filter_pitch(samples: Vec<PitchSample>, window: usize) -> Vec<PitchSample> {
    if samples.len() < 3 {
        return samples;
    }

    let half = window / 2;
    let freqs: Vec<f64> = samples.iter().map(|s| s.freq).collect();
    let mut keep = vec![true; samples.len()];

    for i in 0..samples.len() {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(samples.len());
        if hi - lo < 3 {
            continue;
        }

        let mut neighborhood: Vec<f64> = freqs[lo..hi].to_vec();
        neighborhood.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = neighborhood[neighborhood.len() / 2];

        // 超過 120 cent (1 個半音) 的偏離視為 spike
        // 比 100 cent 稍寬鬆保護 vibrato，但比 150 cent 更精準地移除 spike
        let cent_diff = (1200.0 * (freqs[i] / median).log2()).abs();
        if cent_diff > 120.0 {
            keep[i] = false;
        }
    }

    samples
        .into_iter()
        .zip(keep.into_iter())
        .filter_map(|(s, k)| if k { Some(s) } else { None })
        .collect()
}

/// 從 360-bin 機率分佈解碼出頻率和 confidence。
///
/// 使用 weighted average around peak（±4 bins）取得亞 bin 精度。
fn decode_pitch(probabilities: &[f32]) -> (f64, f64) {
    if probabilities.len() != N_BINS {
        return (0.0, 0.0);
    }

    // 找到最大機率的 bin
    let mut max_idx = 0_usize;
    let mut max_val = 0.0_f32;
    for (i, &p) in probabilities.iter().enumerate() {
        if p > max_val {
            max_val = p;
            max_idx = i;
        }
    }

    let confidence = max_val as f64;
    if confidence < 1e-6 {
        return (0.0, 0.0);
    }

    // Weighted average around peak (±4 bins) for sub-bin precision
    // ±4 比 ±2 更穩定，尤其在 vibrato 時機率分佈較扁平的情況
    let lo = max_idx.saturating_sub(4);
    let hi = (max_idx + 5).min(N_BINS);
    let mut weight_sum = 0.0_f64;
    let mut cent_sum = 0.0_f64;

    for i in lo..hi {
        let w = probabilities[i] as f64;
        let cent = i as f64 * CENTS_PER_BIN;
        cent_sum += cent * w;
        weight_sum += w;
    }

    if weight_sum < 1e-10 {
        return (0.0, 0.0);
    }

    let cent = cent_sum / weight_sum;
    // 加上 CENT_OFFSET：bin 0 = C1 ≈ 32.7 Hz，不是 10 Hz
    let absolute_cent = cent + CENT_OFFSET;
    let freq = 10.0 * 2.0_f64.powf(absolute_cent / 1200.0);

    (freq, confidence)
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_pitch_from_peak_at_a4() {
        // A4 = 440Hz
        // absolute_cent = 1200 * log2(440/10) = 6517.4
        // bin_cent = absolute_cent - CENT_OFFSET = 6517.4 - 1997.38 = 4520.0
        // bin index = 4520.0 / 20 = 226
        let mut probs = vec![0.0_f32; N_BINS];
        probs[226] = 0.9;
        probs[225] = 0.3;
        probs[227] = 0.3;

        let (freq, conf) = decode_pitch(&probs);
        assert!(conf > 0.8, "confidence={}", conf);
        // 允許 ±10 Hz 誤差（受 bin 離散化影響）
        assert!((freq - 440.0).abs() < 10.0, "freq={} expected ~440", freq);
    }

    #[test]
    fn decode_pitch_returns_zero_for_silent() {
        let probs = vec![0.0_f32; N_BINS];
        let (freq, conf) = decode_pitch(&probs);
        assert_eq!(freq, 0.0);
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn slice_frames_correct_count() {
        let audio = vec![0.0_f32; 16000]; // 1 second @16kHz
        let hop = 160; // 10ms
        let frames = slice_frames(&audio, hop);
        // (16000 - 1024) / 160 + 1 = 93.6 → 93
        let expected = (16000 - FRAME_SIZE) / hop + 1;
        assert_eq!(frames.len(), expected);
    }

    #[test]
    fn slice_frames_each_has_correct_size() {
        let audio = vec![0.5_f32; 5000];
        let frames = slice_frames(&audio, 512);
        for f in &frames {
            assert_eq!(f.len(), FRAME_SIZE);
        }
    }

    // ── freq_to_midi_f64 ──

    #[test]
    fn freq_to_midi_a4_is_69() {
        let midi = freq_to_midi_f64(440.0);
        assert!(
            (midi - 69.0).abs() < 1e-9,
            "A4=440Hz should be MIDI 69, got {}",
            midi
        );
    }

    #[test]
    fn freq_to_midi_c4_is_60() {
        // C4 ≈ 261.626 Hz
        let midi = freq_to_midi_f64(261.626);
        assert!(
            (midi - 60.0).abs() < 0.01,
            "C4 should be MIDI 60, got {}",
            midi
        );
    }

    #[test]
    fn freq_to_midi_octave_relation() {
        // 一個八度 = 12 半音
        let midi_220 = freq_to_midi_f64(220.0);
        let midi_440 = freq_to_midi_f64(440.0);
        assert!((midi_440 - midi_220 - 12.0).abs() < 1e-9);
    }

    #[test]
    fn freq_to_midi_zero_returns_zero() {
        assert_eq!(freq_to_midi_f64(0.0), 0.0);
        assert_eq!(freq_to_midi_f64(-100.0), 0.0);
    }

    // ── fix_octave_errors ──

    #[test]
    fn fix_octave_errors_corrects_double_frequency() {
        // 模擬一段穩定 440Hz，中間一個 sample 跳到 880Hz（高八度錯誤）
        let samples: Vec<PitchSample> = (0..20)
            .map(|i| {
                let freq = if i == 10 { 880.0 } else { 440.0 };
                let (note, octave, cent) = freq_to_note(freq);
                PitchSample {
                    timestamp: i as f64 * 0.01,
                    freq,
                    confidence: 0.9,
                    note,
                    octave,
                    cent,
                }
            })
            .collect();
        let fixed = fix_octave_errors(samples);
        // 跳到 880Hz 的那個應被修正回 ~440Hz
        let problem_sample = &fixed[10];
        assert!(
            (problem_sample.freq - 440.0).abs() < 10.0,
            "八度跳躍應被修正，got freq={}",
            problem_sample.freq
        );
    }

    // ── median_filter_pitch ──

    #[test]
    fn median_filter_smooths_spike() {
        // 穩定 440Hz，中間插一個 500Hz 尖刺
        let samples: Vec<PitchSample> = (0..11)
            .map(|i| {
                let freq = if i == 5 { 500.0 } else { 440.0 };
                let (note, octave, cent) = freq_to_note(freq);
                PitchSample {
                    timestamp: i as f64 * 0.01,
                    freq,
                    confidence: 0.9,
                    note,
                    octave,
                    cent,
                }
            })
            .collect();
        let filtered = median_filter_pitch(samples, 120);
        // 中間的尖刺應被平滑掉
        let center = &filtered[5];
        assert!(
            (center.freq - 440.0).abs() < 30.0,
            "尖刺應被中位數濾波平滑，got freq={}",
            center.freq
        );
    }

    #[test]
    fn median_filter_preserves_empty() {
        let result = median_filter_pitch(vec![], 120);
        assert!(result.is_empty());
    }
}
