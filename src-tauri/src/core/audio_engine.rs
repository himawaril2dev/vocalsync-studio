//! 音訊引擎核心（Phase 1 完整版）
//!
//! 職責：
//! - 載入 WAV 伴奏
//! - CPAL 輸出串流：試聽 / 回放
//! - CPAL 輸入串流：錄音（搭配同步播放伴奏）
//! - 導出 WAV
//!
//! 跨執行緒設計：
//! - 共享狀態用 `Arc<Atomic*>`，避免 callback 卡 mutex
//! - CPAL Stream 是 !Send，必須在 worker thread 內建立並持有
//! - Worker thread 負責 emit Tauri events（進度、RMS、狀態）
//!
//! Phase 2 將加入：音高偵測、變速不變調、移調

use crate::core::crepe_engine;
use crate::core::pitch_data::{PitchSample, PitchTrack};
use crate::core::pitch_engine::PitchDetector;
use crate::core::resampler::StreamingResampler;
use crate::error::AppError;
use crate::events;
use crate::events::{
    BackingPitchAnalyzingPayload, BackingPitchNotDetectedPayload, BackingPitchQualityPayload,
    CalibrationBeatPayload, CalibrationCompletePayload, CalibrationFailedPayload,
    CalibrationStartedPayload,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::AppHandle;
use timestretch::{EdmPreset, EnvelopePreset, StreamProcessor, StretchParams};

const PITCH_BUF_SIZE: usize = 2048;

// ── 型別 ──────────────────────────────────────────────────────────

/// 引擎狀態
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Idle,
    Previewing,
    Recording,
    PlayingBack,
}

/// 導出結果路徑
#[derive(Debug, Serialize)]
pub struct ExportPaths {
    pub vocal_path: String,
    pub mix_path: String,
}

/// 載入伴奏的結果（給前端判斷是否要顯示影片）
#[derive(Debug, Serialize)]
pub struct LoadResult {
    pub duration: f64,
    pub sample_rate: u32,
    pub is_video: bool,
    pub video_path: Option<String>,
    /// 自動偵測到的目標旋律來源標籤，例如 "midi" / "uvr_cache"，
    /// 沒偵測到就是 None。這是給前端顯示狀態的提示字串，實際載入由
    /// `auto_load_melody_for_backing` command 處理。
    pub melody_source: Option<String>,
}

/// 裝置資訊
#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub index: usize,
    pub is_default: bool,
}

/// 裝置列表
#[derive(Debug, Serialize)]
pub struct DeviceList {
    pub input_devices: Vec<DeviceInfo>,
    pub output_devices: Vec<DeviceInfo>,
}

/// 🔴 R2 修正：A-B 兩個幀位置打包成一個 u64 原子更新，避免半更新可見性問題。
/// 高 32 bit = A 幀（u32），低 32 bit = B 幀（u32）。
/// 全 0xFFFF_FFFF_FFFF_FFFF = 停用。
const LOOP_PACKED_DISABLED: u64 = u64::MAX;

/// 將 A/B 幀（u32）打包成一個 u64
fn pack_loop(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | (b as u64)
}

/// 從 u64 解包 A/B 幀，回傳 None 代表停用
fn unpack_loop(packed: u64) -> Option<(u64, u64)> {
    if packed == LOOP_PACKED_DISABLED {
        return None;
    }
    let a = (packed >> 32) as u64;
    let b = (packed & 0xFFFF_FFFF) as u64;
    Some((a, b))
}

/// 跨執行緒共享狀態（callback 與主程式都能讀寫）
#[derive(Clone)]
struct SharedState {
    /// 目前播放幀位置（in source sample rate）
    playback_pos: Arc<AtomicU64>,
    /// 是否正在執行
    running: Arc<AtomicBool>,
    /// 伴奏播放音量（f32 bits）
    backing_volume: Arc<AtomicU32>,
    /// 麥克風增益（f32 bits）
    mic_gain: Arc<AtomicU32>,
    /// 即時 RMS 量測
    backing_rms: Arc<AtomicU32>,
    mic_rms: Arc<AtomicU32>,
    /// 最新的音高偵測結果（None 代表靜音/無法偵測）
    current_pitch: Arc<Mutex<Option<PitchSample>>>,
    /// A-B 循環（打包的 u64：高32=A幀, 低32=B幀, 全FF=停用）
    loop_range: Arc<AtomicU64>,
    /// 播放速度（f32 bits，1.0 = 正常）
    speed: Arc<AtomicU32>,
    /// 移調半音數（i32 以 AtomicU32 存放：0 = 不移調）
    pitch_semitones: Arc<AtomicU32>,
}

impl SharedState {
    fn new() -> Self {
        Self {
            playback_pos: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            backing_volume: Arc::new(AtomicU32::new(0.05_f32.to_bits())),
            mic_gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            backing_rms: Arc::new(AtomicU32::new(0)),
            mic_rms: Arc::new(AtomicU32::new(0)),
            current_pitch: Arc::new(Mutex::new(None)),
            loop_range: Arc::new(AtomicU64::new(LOOP_PACKED_DISABLED)),
            speed: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            pitch_semitones: Arc::new(AtomicU32::new(0_i32 as u32)),
        }
    }
}

// ── AudioEngine ───────────────────────────────────────────────────

pub struct AudioEngine {
    pub state: EngineState,
    pub sample_rate: u32,

    // 伴奏資料（立體聲交錯：[L, R, L, R, ...]）
    backing_data: Option<Arc<Vec<f32>>>,
    backing_channels: u16,
    duration: f64,

    // 錄音累積（單聲道）— 共享給錄音 worker 寫入
    vocal_buffer: Arc<Mutex<Vec<f32>>>,

    // 完整人聲音高軌跡（錄音結束後供音高頁分析）
    pitch_track: Arc<Mutex<PitchTrack>>,

    // 伴奏旋律音高軌跡（載入後背景分析，None 代表尚未準備好）
    backing_pitch_track: Arc<Mutex<Option<PitchTrack>>>,

    // 跨執行緒共享狀態
    shared: SharedState,

    // 背景 worker thread handle
    worker: Option<thread::JoinHandle<()>>,

    // 設定（讀寫由 Tauri command 控制，callback 透過 SharedState 讀）
    pub export_volume: f32,
}

/// 80Hz Butterworth 高通濾波器（4 階，由兩級 DirectForm1 串接而成）。
///
/// 目的：移除混音歌曲中低頻 bass 能量（通常落在 40-80Hz），避免 YIN
/// 演算法把最強週期訊號鎖定在 bass line 而抓不到人聲基頻（C3-C5 範圍）。
///
/// 回傳新的 `Vec<f32>`，不改動輸入切片。
fn apply_highpass_80hz(mono: &[f32], sample_rate: u32) -> Vec<f32> {
    use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};

    let fs = (sample_rate as f32).hz();
    let f0 = 80.0_f32.hz();

    // 常數參數，理論上不會失敗；真失敗代表 crate bug，直接 panic 也無妨
    let coeffs = Coefficients::<f32>::from_params(Type::HighPass, fs, f0, Q_BUTTERWORTH_F32)
        .expect("biquad 80Hz HPF coefficients");

    // 兩級 cascade → 4 階 Butterworth，讓 60-80Hz 過渡帶更陡
    let mut stage1 = DirectForm1::<f32>::new(coeffs);
    let mut stage2 = DirectForm1::<f32>::new(coeffs);

    mono.iter().map(|&s| stage2.run(stage1.run(s))).collect()
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            state: EngineState::Idle,
            sample_rate: 44100,
            backing_data: None,
            backing_channels: 2,
            duration: 0.0,
            vocal_buffer: Arc::new(Mutex::new(Vec::new())),
            pitch_track: Arc::new(Mutex::new(PitchTrack::new())),
            backing_pitch_track: Arc::new(Mutex::new(None)),
            shared: SharedState::new(),
            worker: None,
            export_volume: 0.5,
        }
    }

    /// 取得目前的人聲音高軌跡（供音高分析頁查詢）
    pub fn get_pitch_track(&self) -> PitchTrack {
        self.pitch_track
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default()
    }

    /// 取得伴奏旋律音高軌跡（None 代表尚未分析完成）
    pub fn get_backing_pitch_track(&self) -> Option<PitchTrack> {
        self.backing_pitch_track.lock().ok().and_then(|t| t.clone())
    }

    // ── 載入 ───────────────────────────────────────────────────────

    /// 載入伴奏檔（支援 WAV / MP3 / MP4 / FLAC / OGG）
    pub fn load_backing(&mut self, path: &str) -> Result<LoadResult, AppError> {
        // 載入前先停止任何進行中的播放/錄音
        self.stop();

        // 🔴 Codex 安全審查 P2 #7：換曲時清掉舊的人聲錄音。
        // 避免載入新歌後還殘留上一首的 vocal_buffer / pitch_track，
        // 使用者按「回放」或「匯出」時會拿到跨曲錯亂資料，
        // 也算一層隱私防線（不讓前一位使用者的錄音殘留到下一人）。
        self.clear_recording();

        // 統一走 symphonia 解碼，輸出固定為交錯立體聲 f32
        let media = crate::core::media_loader::load_media(path)?;

        self.sample_rate = media.sample_rate;
        self.backing_channels = media.channels;
        self.duration = media.duration;
        self.backing_data = Some(Arc::new(media.samples));
        self.shared.playback_pos.store(0, Ordering::Relaxed);

        // 重設伴奏旋律分析（會在下面背景重新計算）
        if let Ok(mut t) = self.backing_pitch_track.lock() {
            *t = None;
        }

        // 判斷是否為影片格式
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_video = matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "mov" | "avi");

        log::info!(
            "Loaded backing: {}ch, {}Hz, {:.1}s, is_video={}",
            media.channels,
            media.sample_rate,
            media.duration,
            is_video
        );

        Ok(LoadResult {
            duration: self.duration,
            sample_rate: self.sample_rate,
            is_video,
            video_path: if is_video {
                Some(path.to_string())
            } else {
                None
            },
            // melody_source 由 command 層填入（audio_engine 不負責檔案系統掃描）
            melody_source: None,
        })
    }

    /// 啟動背景伴奏旋律分析。
    ///
    /// 使用純 YIN frame-by-frame 偵測，參數對齊 Python 版（buf_size=4096, hop=2048,
    /// f_min=60, f_max=1200, harmonic_threshold=0.20）。
    ///
    /// 設計理念：簡單可靠優先，過去用 PYIN viterbi 反而引入八度誤判跟過度平滑等
    /// bug。Python 版本只用純 YIN 工作得很好，前端用 spline smooth 處理視覺平滑。
    ///
    /// 啟動時 emit `backing_pitch:analyzing`，完成後依品質擇一 emit：
    /// - `backing_pitch:ready`：偵測到可靠主旋律，前端拉取軌跡並繪製灰藍線
    /// - `backing_pitch:not_detected`：純伴奏無主旋律，前端切換自由模式
    ///
    /// 應該在 Tauri command 中於 load_backing 之後立即呼叫。
    /// NOTE: 目前無呼叫端，保留供未來「自動分析伴奏主旋律」功能使用。
    #[allow(dead_code)]
    pub fn analyze_backing_pitch_async(&self, app: AppHandle) {
        let backing = match self.backing_data.clone() {
            Some(b) => b,
            None => return,
        };
        let channels = self.backing_channels as usize;
        let sample_rate = self.sample_rate;
        let track_slot = self.backing_pitch_track.clone();
        let duration = self.duration;

        // 立即通知前端「分析啟動」，避免使用者誤以為功能壞掉
        events::emit_backing_pitch_analyzing(&app, BackingPitchAnalyzingPayload { duration });

        thread::spawn(move || {
            let started = std::time::Instant::now();
            let total_frames = backing.len() / channels;

            // Down-mix 為單聲道
            let mut mono: Vec<f32> = Vec::with_capacity(total_frames);
            for i in 0..total_frames {
                let mut s = 0.0_f32;
                for c in 0..channels {
                    s += backing[i * channels + c];
                }
                mono.push(s / channels as f32);
            }

            // ── 80Hz Butterworth 高通濾波：移除 bass 干擾 ──
            // YIN 對混音歌曲常鎖定 bass line（C2 範圍），過濾 80Hz 以下後
            // 能量重心才會回到人聲基頻範圍（C3-C5）
            let mono = apply_highpass_80hz(&mono, sample_rate);

            // ── 純 YIN frame-by-frame 分析（對齊 Python 版）──
            // buf_size 4096 = 93ms 視窗：能涵蓋更多諧波週期，d' 比 2048 穩定許多
            // hop 2048 = 50% overlap：兼顧時間解析度與計算量
            // f0 60-1200Hz：涵蓋人聲完整音域
            // harmonic_threshold 0.20：寬鬆閾值對混音歌曲友善
            const BACKING_BUF_SIZE: usize = 4096;
            const BACKING_HOP: usize = 2048;
            let mut detector =
                PitchDetector::new(sample_rate, BACKING_BUF_SIZE, 60.0, 1200.0, 0.20, 0.01);

            let mut track = PitchTrack::new();
            let mut total_frame_count = 0_usize;
            let mut voiced_frame_count = 0_usize;
            let mut conf_sum = 0.0_f64;

            let mut start = 0;
            while start + BACKING_BUF_SIZE <= mono.len() {
                let timestamp = start as f64 / sample_rate as f64;
                total_frame_count += 1;
                if let Some(sample) =
                    detector.detect(&mono[start..start + BACKING_BUF_SIZE], timestamp)
                {
                    voiced_frame_count += 1;
                    conf_sum += sample.confidence;
                    track.append(sample);
                }
                start += BACKING_HOP;
            }

            let elapsed_secs = started.elapsed().as_secs_f64();
            let voiced_ratio = if total_frame_count == 0 {
                0.0
            } else {
                voiced_frame_count as f64 / total_frame_count as f64
            };
            let mean_confidence = if voiced_frame_count == 0 {
                0.0
            } else {
                conf_sum / voiced_frame_count as f64
            };
            // 寬鬆的不可靠判定：voiced 比例 < 5% 才視為「真的無主旋律」
            // Python 版根本沒有 unreliable 判定，這裡只用來觸發自由模式 fallback
            let is_unreliable = voiced_ratio < 0.05;

            println!(
                "[YIN] {:.1}s audio → analyzed in {:.2}s, voiced={}/{} ({:.1}%), \
                 mean_conf={:.2}, unreliable={}",
                duration,
                elapsed_secs,
                voiced_frame_count,
                total_frame_count,
                voiced_ratio * 100.0,
                mean_confidence,
                is_unreliable
            );

            // 即使品質不佳也保留軌跡（前端選擇是否使用）
            if let Ok(mut slot) = track_slot.lock() {
                *slot = Some(track);
            }

            // 依品質擇一通知前端
            if is_unreliable {
                let reason = "純伴奏可能無明顯主旋律，已切換為自由模式".to_string();
                events::emit_backing_pitch_not_detected(
                    &app,
                    BackingPitchNotDetectedPayload {
                        voiced_ratio,
                        mean_confidence,
                        elapsed_secs,
                        reason,
                    },
                );
            } else {
                events::emit_backing_pitch_ready(
                    &app,
                    BackingPitchQualityPayload {
                        total_frames: total_frame_count,
                        voiced_frames: voiced_frame_count,
                        voiced_ratio,
                        mean_confidence,
                        elapsed_secs,
                    },
                );
            }
        });
    }

    // ── 屬性查詢 ───────────────────────────────────────────────────

    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn elapsed(&self) -> f64 {
        let frames = self.shared.playback_pos.load(Ordering::Relaxed);
        frames as f64 / self.sample_rate as f64
    }

    pub fn has_backing(&self) -> bool {
        self.backing_data.is_some()
    }

    pub fn has_recording(&self) -> bool {
        self.vocal_buffer
            .lock()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    // ── 設定 ───────────────────────────────────────────────────────

    pub fn set_volume(&mut self, backing: f32, mic: f32) {
        let backing = backing.clamp(0.0, 2.0);
        let mic = mic.clamp(0.0, 5.0);
        self.shared
            .backing_volume
            .store(backing.to_bits(), Ordering::Relaxed);
        self.shared.mic_gain.store(mic.to_bits(), Ordering::Relaxed);
    }

    pub fn seek(&mut self, seconds: f64) {
        let frame = (seconds.max(0.0) * self.sample_rate as f64) as u64;
        self.shared.playback_pos.store(frame, Ordering::Relaxed);
    }

    /// 清除目前錄音資料並回到起點，供前端「清除錄音」按鈕使用。
    ///
    /// 清空 `vocal_buffer`（已錄人聲波形）、`pitch_track`（人聲音高軌跡）、
    /// `current_pitch`（即時顯示值），並把 `playback_pos` 歸零。
    /// 僅在 `Idle` 狀態呼叫（前端按鈕會 disable），不處理執行中的 worker。
    pub fn clear_recording(&self) {
        if let Ok(mut v) = self.vocal_buffer.lock() {
            v.clear();
        }
        if let Ok(mut t) = self.pitch_track.lock() {
            t.clear();
        }
        if let Ok(mut p) = self.shared.current_pitch.lock() {
            *p = None;
        }
        self.shared.playback_pos.store(0, Ordering::Relaxed);
    }

    /// 設定 A-B 循環區間（秒）。A 必須小於 B，B 不可超過歌曲長度。
    pub fn set_loop_points(&self, a_secs: f64, b_secs: f64) {
        if a_secs >= b_secs || a_secs < 0.0 {
            return;
        }
        // 🟡 Y7 修正：B 點 clamp 到歌曲長度
        let b_clamped = b_secs.min(self.duration);
        if a_secs >= b_clamped {
            return;
        }
        let a_frame = (a_secs * self.sample_rate as f64) as u32;
        let b_frame = (b_clamped * self.sample_rate as f64) as u32;
        self.shared
            .loop_range
            .store(pack_loop(a_frame, b_frame), Ordering::Relaxed);
    }

    /// 清除 A-B 循環
    pub fn clear_loop(&self) {
        self.shared
            .loop_range
            .store(LOOP_PACKED_DISABLED, Ordering::Relaxed);
    }

    /// 設定播放速度（0.25 ~ 4.0，1.0 = 正常）
    pub fn set_speed(&self, speed: f64) {
        let clamped = speed.clamp(0.25, 4.0) as f32;
        self.shared
            .speed
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// 取得目前播放速度
    pub fn get_speed(&self) -> f64 {
        f32::from_bits(self.shared.speed.load(Ordering::Relaxed)) as f64
    }

    /// 設定移調半音數（-24 ~ +24，0 = 不移調）
    pub fn set_pitch_semitones(&self, semitones: i32) {
        let clamped = semitones.clamp(-24, 24);
        self.shared
            .pitch_semitones
            .store(clamped as u32, Ordering::Relaxed);
    }

    /// 取得目前移調半音數
    pub fn get_pitch_semitones(&self) -> i32 {
        self.shared.pitch_semitones.load(Ordering::Relaxed) as i32
    }

    /// 取得目前 A-B 循環區間（秒），None 代表未設定
    pub fn get_loop_points(&self) -> Option<(f64, f64)> {
        let packed = self.shared.loop_range.load(Ordering::Relaxed);
        let (a, b) = unpack_loop(packed)?;
        Some((
            a as f64 / self.sample_rate as f64,
            b as f64 / self.sample_rate as f64,
        ))
    }

    // ── 試聽（純伴奏）─────────────────────────────────────────────

    pub fn start_preview(
        &mut self,
        app: AppHandle,
        start_frame: Option<u64>,
        output_device: Option<usize>,
        input_device: Option<usize>,
        pitch_engine_pref: &str,
    ) -> Result<(), AppError> {
        if self.state != EngineState::Idle {
            return Err(AppError::Audio("引擎正忙，請先停止".to_string()));
        }
        let backing = self
            .backing_data
            .clone()
            .ok_or_else(|| AppError::Audio("尚未載入伴奏".to_string()))?;

        // 預覽模式啟用音高偵測時，清空舊的即時音高
        if input_device.is_some() {
            if let Ok(mut p) = self.shared.current_pitch.lock() {
                *p = None;
            }
            self.shared.mic_rms.store(0, Ordering::Relaxed);
        }

        self.spawn_playback_worker(
            app,
            backing,
            start_frame,
            false,
            output_device,
            0.0,
            input_device,
            pitch_engine_pref,
        );
        self.state = EngineState::Previewing;
        Ok(())
    }

    // ── 回放（伴奏 + 人聲混音）────────────────────────────────────

    pub fn start_playback(
        &mut self,
        app: AppHandle,
        start_frame: Option<u64>,
        output_device: Option<usize>,
        latency_ms: f64,
    ) -> Result<(), AppError> {
        if self.state != EngineState::Idle {
            return Err(AppError::Audio("引擎正忙，請先停止".to_string()));
        }
        let backing = self
            .backing_data
            .clone()
            .ok_or_else(|| AppError::Audio("尚未載入伴奏".to_string()))?;

        self.spawn_playback_worker(
            app,
            backing,
            start_frame,
            true,
            output_device,
            latency_ms,
            None,
            "auto",
        );
        self.state = EngineState::PlayingBack;
        Ok(())
    }

    fn spawn_playback_worker(
        &mut self,
        app: AppHandle,
        backing: Arc<Vec<f32>>,
        start_frame: Option<u64>,
        mix_vocal: bool,
        output_device: Option<usize>,
        latency_ms: f64,
        input_device: Option<usize>,
        pitch_engine_pref: &str,
    ) {
        let backing_channels = self.backing_channels as usize;
        let source_sr = self.sample_rate;
        let total_frames = (backing.len() / backing_channels) as u64;
        let duration_s = total_frames as f64 / source_sr as f64;

        let shared = self.shared.clone();
        let vocal_buffer = self.vocal_buffer.clone();

        if let Some(sf) = start_frame {
            shared.playback_pos.store(sf, Ordering::Relaxed);
        }
        shared.running.store(true, Ordering::Relaxed);
        shared.backing_rms.store(0, Ordering::Relaxed);

        let pitch_engine_pref = pitch_engine_pref.to_string();
        let handle = thread::spawn(move || {
            run_playback(
                app,
                backing,
                vocal_buffer,
                backing_channels,
                source_sr,
                total_frames,
                duration_s,
                mix_vocal,
                output_device,
                latency_ms,
                shared,
                input_device,
                &pitch_engine_pref,
            );
        });

        self.worker = Some(handle);
    }

    // ── 錄音 ───────────────────────────────────────────────────────

    pub fn start_recording(
        &mut self,
        app: AppHandle,
        start_frame: Option<u64>,
        input_device: Option<usize>,
        output_device: Option<usize>,
        pitch_engine_pref: &str,
    ) -> Result<(), AppError> {
        if self.state != EngineState::Idle {
            return Err(AppError::Audio("引擎正忙，請先停止".to_string()));
        }
        let backing = self
            .backing_data
            .clone()
            .ok_or_else(|| AppError::Audio("尚未載入伴奏".to_string()))?;

        let backing_channels = self.backing_channels as usize;
        let source_sr = self.sample_rate;
        let total_frames = (backing.len() / backing_channels) as u64;
        let duration_s = total_frames as f64 / source_sr as f64;

        let shared = self.shared.clone();
        let vocal_buffer = self.vocal_buffer.clone();
        let pitch_track = self.pitch_track.clone();

        // 🔴 R1 修正：錄音模式下強制停用 A-B 循環。
        // 原因：output callback 跳回 A 點後 input callback 仍持續 append，
        // 會導致 vocal_buffer 的時間軸與 backing 嚴重錯位。
        shared
            .loop_range
            .store(LOOP_PACKED_DISABLED, Ordering::Relaxed);

        // 續錄判斷：start_frame = Some(>0) 時保留已錄的 vocal_buffer / pitch_track，
        // 讓「錄到 N 秒 → 暫停/停止 → 再按錄音」能從目標位置繼續錄。
        //
        // 時間軸對齊（三種情境）：
        //   vocal_buffer 是 mono，1 frame = 1 sample，index 對應 backing 時間軸。
        //   1. target == v.len()：正常接續，不動 buffer
        //   2. target > v.len()（向後 seek）：用 0 填到 target，新樣本從正確位置 append
        //   3. target < v.len()（向前 seek）：truncate 到 target，捨棄後段
        //      ⚠️ 捨棄後段是破壞性操作，前端 RecordingTab.startRecording()
        //         會先用 dialog 確認使用者意圖，避免誤觸毀掉錄音。
        //      同時 pitch_track 也要同步截斷，避免後段 pitch 殘留。
        let resume_frame = start_frame.filter(|&f| f > 0);
        let fresh_start = resume_frame.is_none();

        if fresh_start {
            if let Ok(mut v) = vocal_buffer.lock() {
                v.clear();
            }
            if let Ok(mut t) = pitch_track.lock() {
                t.clear();
            }
        } else if let Some(target) = resume_frame {
            let target_len = target as usize;
            let target_secs = target as f64 / source_sr as f64;
            if let Ok(mut v) = vocal_buffer.lock() {
                match v.len().cmp(&target_len) {
                    std::cmp::Ordering::Less => v.resize(target_len, 0.0),
                    std::cmp::Ordering::Greater => {
                        log::info!(
                            "[record] 向前 seek 續錄：truncate vocal_buffer {} → {} samples",
                            v.len(),
                            target_len
                        );
                        v.truncate(target_len);
                    }
                    std::cmp::Ordering::Equal => {}
                }
            }
            // 同步截斷 pitch_track（只保留時戳 < target_secs 的樣本）
            if let Ok(mut t) = pitch_track.lock() {
                t.truncate_after(target_secs);
            }
        }
        // current_pitch 是即時顯示值，無論如何都清掉（避免 idle 殘留值閃一下）
        if let Ok(mut p) = shared.current_pitch.lock() {
            *p = None;
        }

        shared
            .playback_pos
            .store(resume_frame.unwrap_or(0), Ordering::Relaxed);
        shared.running.store(true, Ordering::Relaxed);
        shared.backing_rms.store(0, Ordering::Relaxed);
        shared.mic_rms.store(0, Ordering::Relaxed);

        let pitch_engine_pref = pitch_engine_pref.to_string();
        let handle = thread::spawn(move || {
            run_recording(
                app,
                backing,
                vocal_buffer,
                pitch_track,
                backing_channels,
                source_sr,
                total_frames,
                duration_s,
                input_device,
                output_device,
                shared,
                &pitch_engine_pref,
            );
        });

        self.worker = Some(handle);
        self.state = EngineState::Recording;
        Ok(())
    }

    // ── 停止 ───────────────────────────────────────────────────────

    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.state = EngineState::Idle;
    }

    pub fn pause(&mut self) -> u64 {
        let pos = self.shared.playback_pos.load(Ordering::Relaxed);
        self.stop();
        pos
    }

    // ── 裝置列舉 ───────────────────────────────────────────────────

    pub fn list_devices() -> Result<DeviceList, AppError> {
        let host = cpal::default_host();
        let default_input = host
            .default_input_device()
            .map(|d| d.name().unwrap_or_default());
        let default_output = host
            .default_output_device()
            .map(|d| d.name().unwrap_or_default());

        let input_devices: Vec<DeviceInfo> = host
            .input_devices()
            .map_err(|e| AppError::Audio(e.to_string()))?
            .enumerate()
            .filter_map(|(i, d)| {
                let name = d.name().ok()?;
                Some(DeviceInfo {
                    is_default: Some(&name) == default_input.as_ref(),
                    name,
                    index: i,
                })
            })
            .collect();

        let output_devices: Vec<DeviceInfo> = host
            .output_devices()
            .map_err(|e| AppError::Audio(e.to_string()))?
            .enumerate()
            .filter_map(|(i, d)| {
                let name = d.name().ok()?;
                Some(DeviceInfo {
                    is_default: Some(&name) == default_output.as_ref(),
                    name,
                    index: i,
                })
            })
            .collect();

        Ok(DeviceList {
            input_devices,
            output_devices,
        })
    }

    // ── 導出 ───────────────────────────────────────────────────────

    pub fn export(
        &self,
        dir: &str,
        prefix: &str,
        auto_balance: bool,
        latency_ms: f64,
    ) -> Result<ExportPaths, AppError> {
        let vocal = self
            .vocal_buffer
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if vocal.is_empty() {
            return Err(AppError::Audio("沒有錄音資料可導出".to_string()));
        }

        let backing = self
            .backing_data
            .as_ref()
            .ok_or_else(|| AppError::Audio("沒有伴奏資料".to_string()))?;

        let sr = self.sample_rate;
        std::fs::create_dir_all(dir).ok();

        let vocal_path = format!("{}/{}_vocal.wav", dir, prefix);
        let mix_path = format!("{}/{}_mix.wav", dir, prefix);

        let mono_spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let stereo_spec = hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };

        // 計算延遲偏移 (Sample 數)
        let latency_frames = (latency_ms / 1000.0 * sr as f64).round() as isize;

        // 純人聲（單聲道）套用平移
        let mut shifted_vocal = Vec::with_capacity(vocal.len());
        for i in 0..vocal.len() {
            let src_idx = i as isize + latency_frames;
            if src_idx >= 0 && src_idx < vocal.len() as isize {
                shifted_vocal.push(vocal[src_idx as usize]);
            } else {
                shifted_vocal.push(0.0);
            }
        }

        let mut writer = hound::WavWriter::create(&vocal_path, mono_spec)?;
        for &s in shifted_vocal.iter() {
            writer.write_sample(s.clamp(-1.0, 1.0))?;
        }
        writer.finalize()?;

        let backing_channels = self.backing_channels as usize;
        let total_backing_frames = backing.len() / backing_channels;
        let n = total_backing_frames.min(shifted_vocal.len());

        // 參數：標準化/自動音量調整
        let mut vocal_gain = 1.0;
        if auto_balance && n > 0 {
            // 找伴奏 RMS 與 人聲 RMS
            let mut sum_b_sq = 0.0;
            let mut sum_v_sq = 0.0;
            for i in 0..n {
                let bidx = i * backing_channels;
                let bg = (backing[bidx] + backing[bidx + (1.min(backing_channels - 1))]) * 0.5;
                sum_b_sq += bg * bg;
                let vg = shifted_vocal[i];
                sum_v_sq += vg * vg;
            }
            let b_rms = (sum_b_sq / n as f32).sqrt();
            let v_rms = (sum_v_sq / n as f32).sqrt();

            // 目標：人聲 RMS ≈ 原始 backing RMS × 1.5。
            //
            // 重要：下方實際混音時伴奏會被 `self.export_volume`（= 0.5）衰減，
            // 所以人聲對「實際混音伴奏」的比例 = 1.5 / 0.5 = 3.0 倍 ≈ +9.5 dB，
            // 卡拉 OK 場景下人聲清楚主導。
            //
            // 只設下限、不設上限：
            //   舊設計有 clamp 上限 4.0 會把小聲錄音的 gain 卡死，
            //   人聲放不夠大反而讓伴奏聽起來蓋過人聲（使用者實測回報）。
            //   下限 0.5 防止超大音量錄音被過度衰減失去動態。
            //   normalize 階段的 peak limiter 會處理整體 clip 風險。
            if v_rms > 0.001 {
                vocal_gain = ((b_rms * 1.5) / v_rms).max(0.5);
            }
        }

        // 預掃描找峰值，確保加總後不會 Clip
        let mut max_peak = 0.0_f32;
        let mut mix_buffer = Vec::with_capacity(n * 2);
        for i in 0..n {
            let bidx = i * backing_channels;
            let bl = backing[bidx] * self.export_volume;
            let br = backing[bidx + (1.min(backing_channels - 1))] * self.export_volume;
            let v = shifted_vocal[i] * vocal_gain;

            let l = bl + v;
            let r = br + v;
            if l.abs() > max_peak {
                max_peak = l.abs();
            }
            if r.abs() > max_peak {
                max_peak = r.abs();
            }

            mix_buffer.push(l);
            mix_buffer.push(r);
        }

        let mix_gain = if max_peak > 0.9 {
            // -1 dBTP = 10^(-1/20) = 0.891
            0.891 / max_peak
        } else {
            1.0
        };

        let mut writer = hound::WavWriter::create(&mix_path, stereo_spec)?;
        for s in mix_buffer {
            writer.write_sample((s * mix_gain).clamp(-1.0, 1.0))?;
        }
        writer.finalize()?;

        log::info!("Exported: vocal={}, mix={}", vocal_path, mix_path);

        Ok(ExportPaths {
            vocal_path,
            mix_path,
        })
    }

    // ── 互動式延遲校準 ────────────────────────────────────────────────

    /// 互動式延遲校準（方向二改良版）。
    ///
    /// 流程：
    /// 1. 產生 8 下木魚音 click（前 2 拍暖身，後 6 拍量測），70 BPM
    /// 2. 透過 `run_recording` 同時播放 + 錄音
    /// 3. 錄音結束後，對每一拍跑 onset detection（搜尋窗口 [-200ms, +400ms]，容許提前拍）
    /// 4. 暖身拍丟棄；量測拍取 median，再剔除偏離 median > 80ms 的離群值
    /// 5. 有效拍 < 3 拍視為失敗
    /// 6. 每一拍都 emit `calibration:beat_detected` 事件給前端動畫補上即時回饋
    ///
    /// 修掉舊版 P0 bug：
    /// - 不再寫入 `self.backing_channels`（避免污染後續 stereo 播放）
    /// - 移除魔術 15ms 補償
    /// - peak 搜尋窗對稱（容許負延遲，即提前拍）
    /// - 動態 noise floor（從 prep 階段量測）
    /// - 用 onset detection（能量上升）取代純 amplitude peak
    pub fn calibrate_latency(
        &mut self,
        app: AppHandle,
        input_device_idx: Option<usize>,
        output_device_idx: Option<usize>,
    ) -> Result<u64, AppError> {
        let source_sr = self.sample_rate;

        // ── 校準參數 ──
        const BPM: f32 = 70.0;
        const WARMUP_BEATS: usize = 2;
        const MEASUREMENT_BEATS: usize = 6;
        const TOTAL_BEATS: usize = WARMUP_BEATS + MEASUREMENT_BEATS;
        const PREP_MS: u32 = 1500; // 1.5s 給使用者抓節奏感
        const TAIL_MS: u32 = 500; // 最後一拍後留 500ms 收尾錄音

        let beat_interval_ms: u32 = (60_000.0 / BPM) as u32;
        let frames_per_beat = (source_sr as f32 * 60.0 / BPM) as usize;
        let prep_delay_frames = (source_sr as f32 * PREP_MS as f32 / 1000.0) as usize;
        let tail_frames = (source_sr as f32 * TAIL_MS as f32 / 1000.0) as usize;
        let total_frames = prep_delay_frames + TOTAL_BEATS * frames_per_beat + tail_frames;

        // ── 產生 click track（兩段木魚式短音，比 sine beep 更易精準對拍）──
        let click_buf = generate_woodblock_click(source_sr);
        let mut click_track = vec![0.0_f32; total_frames];
        for b in 0..TOTAL_BEATS {
            let start = prep_delay_frames + b * frames_per_beat;
            for (i, sample) in click_buf.iter().enumerate() {
                if start + i < total_frames {
                    click_track[start + i] = *sample;
                }
            }
        }

        // ── 共享狀態（不污染 self.backing_channels）──
        let calibration_channels: u16 = 1;
        let shared = self.shared.clone();
        let vocal_buffer = self.vocal_buffer.clone();
        let pitch_track = self.pitch_track.clone();

        if let Ok(mut v) = vocal_buffer.lock() {
            v.clear();
        }
        if let Ok(mut t) = pitch_track.lock() {
            t.clear();
        }

        shared.playback_pos.store(0, Ordering::Relaxed);
        shared.running.store(true, Ordering::Relaxed);

        let dur_s = total_frames as f64 / source_sr as f64;

        // 通知前端校準正式啟動（時間軸從這一刻開始）
        events::emit_calibration_started(
            &app,
            CalibrationStartedPayload {
                bpm: BPM,
                warmup_beats: WARMUP_BEATS as u8,
                measurement_beats: MEASUREMENT_BEATS as u8,
                prep_ms: PREP_MS,
                beat_interval_ms,
            },
        );

        let app_thread = app.clone();
        let handle = thread::spawn(move || {
            run_recording(
                app_thread,
                Arc::new(click_track),
                vocal_buffer,
                pitch_track,
                calibration_channels as usize,
                source_sr,
                total_frames as u64,
                dur_s,
                input_device_idx,
                output_device_idx,
                shared,
                "auto", // 校準不關心音高引擎偏好
            );
        });

        let _ = handle.join();

        // ── 取出錄音資料 ──
        let recorded: Vec<f32> = match self.vocal_buffer.lock() {
            Ok(v) => v.clone(),
            Err(e) => {
                let reason = format!("無法讀取錄音 buffer：{}", e);
                events::emit_calibration_failed(
                    &app,
                    CalibrationFailedPayload {
                        reason: reason.clone(),
                    },
                );
                return Err(AppError::Audio(reason));
            }
        };

        if recorded.is_empty() {
            let reason = "錄音 buffer 空白，請確認麥克風裝置".to_string();
            events::emit_calibration_failed(
                &app,
                CalibrationFailedPayload {
                    reason: reason.clone(),
                },
            );
            return Err(AppError::Audio(reason));
        }

        // ── 動態 noise floor：從 prep 階段（前 1 秒）量測 ──
        let noise_window = (source_sr as usize).min(recorded.len() / 2);
        let noise_floor = compute_rms(&recorded[..noise_window]).max(0.0005);

        // ── 對每一拍跑 onset detection ──
        // 搜尋窗口：[-200ms, +400ms]（容許使用者提前 200ms 拍）
        let search_before = (source_sr as f32 * 0.200) as usize;
        let search_after = (source_sr as f32 * 0.400) as usize;

        let mut beat_results: Vec<BeatResult> = Vec::with_capacity(TOTAL_BEATS);

        for b in 0..TOTAL_BEATS {
            let expected = prep_delay_frames + b * frames_per_beat;
            let win_start = expected.saturating_sub(search_before);
            let win_end = (expected + search_after).min(recorded.len());

            let onset = if win_end > win_start {
                detect_onset_in_window(&recorded[win_start..win_end], source_sr, noise_floor)
                    .map(|rel| win_start + rel)
            } else {
                None
            };

            let offset_samples: i64 = match onset {
                Some(idx) => idx as i64 - expected as i64,
                None => 0,
            };
            let offset_ms = offset_samples as f64 / source_sr as f64 * 1000.0;

            beat_results.push(BeatResult {
                beat_idx: b as u8,
                is_warmup: b < WARMUP_BEATS,
                detected: onset.is_some(),
                offset_samples,
                offset_ms,
                accepted: false, // 後面再決定
            });
        }

        // ── 量測拍取 median，剔除離群值（> 80ms）──
        const OUTLIER_THRESHOLD_MS: f64 = 80.0;

        let measurement_offsets: Vec<f64> = beat_results
            .iter()
            .filter(|r| !r.is_warmup && r.detected)
            .map(|r| r.offset_ms)
            .collect();

        if measurement_offsets.len() < 3 {
            let reason = format!(
                "只偵測到 {} 拍有效回應，請確認麥克風音量或環境噪音後重試",
                measurement_offsets.len()
            );
            events::emit_calibration_failed(
                &app,
                CalibrationFailedPayload {
                    reason: reason.clone(),
                },
            );
            return Err(AppError::Audio(reason));
        }

        let median_ms = compute_median(&measurement_offsets);

        // 標記哪些拍被接受
        for r in beat_results.iter_mut() {
            r.accepted = !r.is_warmup
                && r.detected
                && (r.offset_ms - median_ms).abs() <= OUTLIER_THRESHOLD_MS;
        }

        // 逐拍 emit 事件（前端動畫補上即時回饋）
        for r in &beat_results {
            events::emit_calibration_beat(
                &app,
                CalibrationBeatPayload {
                    beat_idx: r.beat_idx,
                    is_warmup: r.is_warmup,
                    detected: r.detected,
                    accepted: r.accepted,
                    offset_ms: r.offset_ms,
                },
            );
        }

        let accepted_offsets: Vec<f64> = beat_results
            .iter()
            .filter(|r| r.accepted)
            .map(|r| r.offset_ms)
            .collect();

        if accepted_offsets.len() < 3 {
            let reason = format!(
                "離群值過多，僅 {} 拍合格（中位數 {:.0}ms）。請保持節奏穩定後重試",
                accepted_offsets.len(),
                median_ms
            );
            events::emit_calibration_failed(
                &app,
                CalibrationFailedPayload {
                    reason: reason.clone(),
                },
            );
            return Err(AppError::Audio(reason));
        }

        let mean_ms: f64 = accepted_offsets.iter().sum::<f64>() / accepted_offsets.len() as f64;

        let variance: f64 = accepted_offsets
            .iter()
            .map(|&o| (o - mean_ms).powi(2))
            .sum::<f64>()
            / accepted_offsets.len() as f64;
        let std_dev_ms = variance.sqrt();

        // 最終延遲值：負數視為 0（人類提前拍不應補償成負）
        let latency_ms = mean_ms.max(0.0).round() as u64;

        events::emit_calibration_complete(
            &app,
            CalibrationCompletePayload {
                latency_ms,
                valid_beats: accepted_offsets.len() as u8,
                measurement_beats: MEASUREMENT_BEATS as u8,
                std_dev_ms,
            },
        );

        Ok(latency_ms)
    }
}

// ── 校準輔助函式 ──────────────────────────────────────────────────

/// 校準量測中單一拍的結果
struct BeatResult {
    beat_idx: u8,
    is_warmup: bool,
    detected: bool,
    #[allow(dead_code)]
    offset_samples: i64,
    offset_ms: f64,
    accepted: bool,
}

/// 產生木魚式短促 click（40ms，雙頻 + 指數衰減包絡）。
/// 比 1000Hz sine beep 更具瞬態感，使用者更容易精準對拍。
fn generate_woodblock_click(sample_rate: u32) -> Vec<f32> {
    let len = (sample_rate as f32 * 0.040) as usize; // 40ms
    let freq1 = 1800.0_f32;
    let freq2 = 2400.0_f32;
    let two_pi = 2.0 * std::f32::consts::PI;

    (0..len)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            // 指數衰減包絡，attack 極短（< 1ms）
            let env = (-t * 80.0).exp();
            let s1 = (two_pi * freq1 * t).sin();
            let s2 = (two_pi * freq2 * t).sin();
            env * (s1 * 0.6 + s2 * 0.4) * 0.85
        })
        .collect()
}

/// 計算切片的 RMS。
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// 計算 f64 切片的中位數（不修改原資料）。
fn compute_median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// 在指定樣本切片內偵測 onset（能量上升點）。
///
/// 演算法：
/// 1. 用 5ms 滑動視窗計算 short-time RMS（hop = 8 樣本）
/// 2. 從頭找第一個「相對前 4ms 上升幅度」最大且超過 noise_floor*3 的位置
/// 3. 回傳該位置的樣本 index（相對於切片起點）
///
/// 為什麼用「上升幅度」而非「絕對最大」：木魚 click 的瞬態能量上升比後續尾音更具
/// 時間定位精度，能避免「最大值漂移到尾音中段」的偏差。
fn detect_onset_in_window(slice: &[f32], sample_rate: u32, noise_floor: f32) -> Option<usize> {
    let win = ((sample_rate as f32 * 0.005) as usize).max(8);
    let hop = 8_usize;
    let lookback_frames = ((sample_rate as f32 * 0.004) as usize / hop).max(2);

    if slice.len() < win + hop * lookback_frames * 2 {
        return None;
    }

    // 預先計算 hop 步長的 RMS envelope
    let n_frames = (slice.len().saturating_sub(win)) / hop;
    if n_frames <= lookback_frames {
        return None;
    }

    let mut env = Vec::with_capacity(n_frames);
    for f in 0..n_frames {
        let start = f * hop;
        let end = start + win;
        let mut sum_sq = 0.0_f32;
        for s in &slice[start..end] {
            sum_sq += s * s;
        }
        env.push((sum_sq / win as f32).sqrt());
    }

    let threshold = noise_floor * 3.0;
    let mut best_rise = 0.0_f32;
    let mut best_frame: Option<usize> = None;

    for f in lookback_frames..env.len() {
        if env[f] < threshold {
            continue;
        }
        let rise = env[f] - env[f - lookback_frames];
        if rise > best_rise {
            best_rise = rise;
            best_frame = Some(f);
        }
    }

    // 至少要有可量測的上升量；rise 太小代表沒有真正的 onset
    let min_rise = noise_floor * 2.0;
    best_frame.filter(|_| best_rise > min_rise).map(|f| f * hop)
}

// ── 共用 output callback 邏輯 ─────────────────────────────────────

/// 🟡 Y4 修正：抽取 playback / recording 共用的 backing 播放 callback 邏輯。
///
/// 回傳值：(new_pos, rms)，讓呼叫方做後續更新。
/// - `vocal_snapshot`：回放模式時傳入已錄製的人聲；錄音模式傳 None
/// - `latency_ms` / `source_sr`：用於回放模式人聲混音的延遲補償
#[allow(clippy::too_many_arguments)]
fn fill_backing_output(
    data: &mut [f32],
    backing: &[f32],
    backing_channels: usize,
    out_channels: usize,
    rate_ratio: f64,
    total_frames: u64,
    cur_pos: u64,
    vol: f32,
    vocal_snapshot: Option<&[f32]>,
    latency_ms: f64,
    source_sr: u32,
) -> (u64, f32) {
    let frame_count = data.len() / out_channels;
    let mut sum_sq = 0.0_f32;
    let mut sample_count = 0_usize;

    for frame in 0..frame_count {
        let src_pos_f = (cur_pos as f64) + (frame as f64) * rate_ratio;
        let src_idx_lo = src_pos_f.floor() as u64;
        let frac = (src_pos_f - src_idx_lo as f64) as f32;

        if src_idx_lo + 1 >= total_frames {
            for ch in 0..out_channels {
                data[frame * out_channels + ch] = 0.0;
            }
            continue;
        }

        let lo = (src_idx_lo as usize) * backing_channels;
        let hi = lo + backing_channels;

        for ch in 0..out_channels {
            let src_ch = ch.min(backing_channels - 1);
            let s_lo = backing[lo + src_ch];
            let s_hi = backing[hi + src_ch];
            let mut s = (s_lo + (s_hi - s_lo) * frac) * vol;

            // 混入人聲（回放模式）
            if let Some(vocal) = vocal_snapshot {
                let latency_frames = (latency_ms / 1000.0 * source_sr as f64).round() as isize;
                let src_idx = src_idx_lo as isize + latency_frames;
                if src_idx >= 0 && src_idx < vocal.len() as isize {
                    s += vocal[src_idx as usize];
                }
            }

            let s = s.clamp(-1.0, 1.0);
            data[frame * out_channels + ch] = s;
            sum_sq += s * s;
            sample_count += 1;
        }
    }

    let consumed = (frame_count as f64 * rate_ratio) as u64;
    let new_pos = (cur_pos + consumed).min(total_frames);
    let rms = if sample_count > 0 {
        (sum_sq / sample_count as f32).sqrt()
    } else {
        0.0
    };

    (new_pos, rms)
}

#[inline]
fn sample_cubic_interleaved(frames: &[f32], channels: usize, frame_pos: f64, src_ch: usize) -> f32 {
    let frame_count = frames.len() / channels;
    if frame_count == 0 {
        return 0.0;
    }

    let ch = src_ch.min(channels.saturating_sub(1));
    let x1 = frame_pos
        .floor()
        .clamp(0.0, (frame_count.saturating_sub(1)) as f64) as isize;
    let t = (frame_pos - x1 as f64) as f32;
    let x0 = (x1 - 1).clamp(0, frame_count as isize - 1) as usize;
    let x1 = x1.clamp(0, frame_count as isize - 1) as usize;
    let x2 = (x1 as isize + 1).clamp(0, frame_count as isize - 1) as usize;
    let x3 = (x1 as isize + 2).clamp(0, frame_count as isize - 1) as usize;

    let p0 = frames[x0 * channels + ch];
    let p1 = frames[x1 * channels + ch];
    let p2 = frames[x2 * channels + ch];
    let p3 = frames[x3 * channels + ch];
    let t2 = t * t;
    let t3 = t2 * t;

    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

#[inline]
fn sample_linear_mono(samples: &[f32], pos: f64) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let lo = pos
        .floor()
        .clamp(0.0, (samples.len().saturating_sub(1)) as f64) as usize;
    let hi = (lo + 1).min(samples.len().saturating_sub(1));
    let frac = (pos - lo as f64) as f32;
    let s_lo = samples[lo];
    let s_hi = samples[hi];
    s_lo + (s_hi - s_lo) * frac
}

/// Lanczos-3 sinc kernel（a=3，頻譜洩漏極低，sidelobe < -26dB）。
///
/// `L(x) = sinc(x) * sinc(x/a)` for `|x| < a`；超出取 0。
///
/// NOTE：目前 production 不用——實測 WSOLA hop 邊界 / HouseLoop phase artifact 的
/// 微相位跳變會被 6-tap 支撐擴散成 ringing，反而比 cubic 差。保留函式供未來若換
/// 其他 stretch crate（phase-continuous streaming）或純 offline 路徑時重用。
#[allow(dead_code)]
#[inline]
fn lanczos3_kernel(x: f64) -> f64 {
    const A: f64 = 3.0;
    let abs_x = x.abs();
    if abs_x < 1e-9 {
        return 1.0;
    }
    if abs_x >= A {
        return 0.0;
    }
    let px = std::f64::consts::PI * x;
    let px_a = px / A;
    (px.sin() / px) * (px_a.sin() / px_a)
}

/// 6-tap Lanczos-3 sinc 插值（for interleaved frames）。
///
/// 相較於 Catmull-Rom cubic：
/// - sidelobe < -26dB（cubic ~-14dB）→ 理論上高頻 aliasing/imaging 顯著減少
/// - 計算量約為 cubic 的 2 倍（6 乘加 vs 4 乘加）
///
/// 邊界策略：超出 `frames` 的 tap 以端點 clamp 填補（零餘假設會產生低頻誤差，
/// 對短尾端幀影響較大；端點保持能量比較保守）。
///
/// NOTE：目前 production 不用；見 `lanczos3_kernel` 註解。
#[allow(dead_code)]
#[inline]
fn sample_lanczos3_interleaved(
    frames: &[f32],
    channels: usize,
    frame_pos: f64,
    src_ch: usize,
) -> f32 {
    let ch_count = channels.max(1);
    let frame_count = frames.len() / ch_count;
    if frame_count == 0 {
        return 0.0;
    }

    let ch = src_ch.min(ch_count.saturating_sub(1));
    let i0 = frame_pos.floor() as isize;
    let t = frame_pos - i0 as f64;

    // 取 6 tap：i0-2, i0-1, i0, i0+1, i0+2, i0+3
    let mut sum = 0.0_f64;
    let mut weight_sum = 0.0_f64;
    let last = frame_count as isize - 1;
    for k in -2_isize..=3 {
        let idx = (i0 + k).clamp(0, last) as usize;
        let sample = frames[idx * ch_count + ch] as f64;
        let x = k as f64 - t;
        let w = lanczos3_kernel(x);
        sum += sample * w;
        weight_sum += w;
    }

    // 權重歸一化：sinc 在無限長 kernel 上 Σw = 1，6-tap 有限截斷會略偏；
    // 歸一化避免增益微幅起伏（對 RMS 穩定性有幫助）。
    if weight_sum.abs() > 1e-9 {
        (sum / weight_sum) as f32
    } else {
        sum as f32
    }
}

#[inline]
#[allow(dead_code)]
fn compact_interleaved_front(queue: &mut Vec<f32>, head_frames: &mut usize, channels: usize) {
    if *head_frames == 0 {
        return;
    }

    let drop_samples = (*head_frames).saturating_mul(channels);
    if drop_samples == 0 || drop_samples >= queue.len() {
        queue.clear();
        *head_frames = 0;
        return;
    }

    queue.copy_within(drop_samples.., 0);
    queue.truncate(queue.len() - drop_samples);
    *head_frames = 0;
}

fn build_output_resample_filters(
    channels: usize,
    from_sr: u32,
    to_sr: u32,
) -> Vec<[Option<biquad::DirectForm1<f32>>; 2]> {
    use biquad::{Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};

    let mut filters = (0..channels).map(|_| [None, None]).collect::<Vec<_>>();
    if from_sr <= to_sr {
        return filters;
    }

    let fs = (from_sr as f32).hz();
    let cutoff = ((to_sr as f32) * 0.45).hz();
    if let Ok(coeffs) =
        Coefficients::<f32>::from_params(Type::LowPass, fs, cutoff, Q_BUTTERWORTH_F32)
    {
        for channel_filters in &mut filters {
            channel_filters[0] = Some(DirectForm1::<f32>::new(coeffs));
            channel_filters[1] = Some(DirectForm1::<f32>::new(coeffs));
        }
    }

    filters
}

/// 重置 pitch anti-aliasing cascade 的內部 state（保留 coefficients，不 rebuild）。
///
/// 使用時機：transport discontinuity（seek、loop、path switch、pitch change），
/// 避免舊片段的 IIR 能量滲透到新片段造成 smear / click。
///
/// 設計筆記：這裡用 `reset_state()` 而非重新 `new(coeffs)`，因為 coefficients 本身
/// 沒變（還是同一個 cur_pitch_st 對應的 f_cut），只需清零 memory。
#[inline]
fn reset_pitch_aa_state(filters: &mut [[Option<biquad::DirectForm1<f32>>; 5]]) {
    use biquad::Biquad;
    for ch_filters in filters.iter_mut() {
        for stage in ch_filters.iter_mut().flatten() {
            stage.reset_state();
        }
    }
}

// ── Worker thread 函式 ────────────────────────────────────────────

/// 純播放 worker（試聽 / 回放共用）
///
/// `input_device_idx`：若提供，開啟麥克風做即時音高偵測（不寫入 vocal_buffer）。
/// 用於預覽模式讓使用者邊聽伴奏邊看自己的音高。
#[allow(clippy::too_many_arguments)]
fn run_playback(
    app: AppHandle,
    backing: Arc<Vec<f32>>,
    vocal_buffer: Arc<Mutex<Vec<f32>>>,
    backing_channels: usize,
    source_sr: u32,
    total_frames: u64,
    duration_s: f64,
    mix_vocal: bool,
    output_device_idx: Option<usize>,
    latency_ms: f64,
    shared: SharedState,
    input_device_idx: Option<usize>,
    pitch_engine_pref: &str,
) {
    let host = cpal::default_host();
    let device = output_device_idx
        .and_then(|idx| {
            host.output_devices()
                .ok()
                .and_then(|mut devs| devs.nth(idx))
        })
        .or_else(|| host.default_output_device());

    let device = match device {
        Some(d) => d,
        None => {
            events::emit_error(&app, "找不到輸出裝置");
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    let config = match build_output_config(&device, source_sr) {
        Ok(c) => c,
        Err(e) => {
            events::emit_error(&app, &format!("輸出設定錯誤：{}", e));
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    let out_channels = config.channels as usize;
    let out_sr = config.sample_rate.0;
    let rate_ratio = source_sr as f64 / out_sr as f64;

    // Snapshot 人聲資料（如果要混音）
    let vocal_snapshot: Option<Arc<Vec<f32>>> = if mix_vocal {
        vocal_buffer.lock().ok().map(|v| Arc::new(v.clone()))
    } else {
        None
    };

    let pos = shared.playback_pos.clone();
    let running = shared.running.clone();
    let volume = shared.backing_volume.clone();
    let backing_rms = shared.backing_rms.clone();
    let loop_range = shared.loop_range.clone();
    let speed_atomic = shared.speed.clone();
    let pitch_atomic = shared.pitch_semitones.clone();

    let backing_cb = backing.clone();
    let vocal_cb = vocal_snapshot.clone();
    let running_cb = running.clone();

    // ── Stretch producer/consumer 架構（lock-free ring buffer） ──
    //
    // 根因：processor.process_into() 是計算密集的 DSP 操作，
    // 在 CPAL output callback（即時線程）中執行會造成偶發 glitch。
    //
    // 解法：背景 producer 線程持續預先生成 stretched 音訊並寫入 ring buffer，
    // CPAL callback 只從 ring buffer 讀取——零 DSP 運算、zero-lock（SPSC）。
    let can_use_stream_processor = backing_channels <= 2;

    // 🔴 Codex 審查 P2 #8：F POC 把 producer 路徑關掉（consumer 全走 WSOLA），
    // 但如果 still spawn producer，它在 pitch != 0 時會跑 `process_into()` 白工，
    // ring 永遠沒人消費、還浪費 DSP（弱機器上會造成 glitch）。
    // 這個常數同時控制 (a) 是否 spawn producer (b) consumer 是否讀 ring。
    // 將來要 revert F POC 只需改這裡成 true 或傳外部旗標即可。
    const USE_PRODUCER_PATH: bool = false;

    // Ring buffer 容量：~0.5 秒的 stretched 音訊（source_sr 幀 × channels）
    let ring_capacity_frames: usize = (source_sr as usize / 2).max(16_384);
    let ring_capacity_samples = ring_capacity_frames * backing_channels;

    // Producer → Consumer 的 lock-free SPSC ring buffer（interleaved f32 samples）
    // HeapProd 會 move 進 producer 線程；HeapCons 會 move 進 CPAL callback closure
    let ring = HeapRb::<f32>::new(ring_capacity_samples);
    let (ring_producer, ring_consumer) = ring.split();

    // Producer 控制信號
    let producer_running = Arc::new(AtomicBool::new(true));
    // Seek 通知：callback 發現跳躍時設為目標幀位置 + 1（0 = 無 seek）
    let producer_seek_to = Arc::new(AtomicU64::new(0));
    // 參數同步（producer 自行從 shared 的原子讀取 speed/pitch）
    let producer_speed = shared.speed.clone();
    let producer_pitch = shared.pitch_semitones.clone();

    // Consumer 端的狀態追蹤（closure 捕獲為 mut）
    let mut stretch_play_pos = shared.playback_pos.load(Ordering::Relaxed) as f64;
    // stretch_active_prev：上一次 callback 是否走 timestretch producer 路徑（純移調用）
    let mut stretch_active_prev = false;
    // wsola_active_prev：上一次 callback 是否走 WSOLA 路徑（變速/變速+移調用）
    let mut wsola_active_prev = false;
    // 追蹤 consumer 從 ring buffer 中已消費的小數幀位移
    let mut ring_consume_frac: f64 = 0.0;
    // 跨 callback 保留的尾端 frames（cubic interpolation 需要 preview 後續幀）
    // 這樣 pop_slice 可以精確控制消耗量 = consumed_whole，避免 ring buffer 被過度消耗
    let mut carry_buf: Vec<f32> = Vec::with_capacity(32 * backing_channels);
    // 本地重用暫存（避免每個 callback 重複配置）
    let mut local_buf: Vec<f32> = Vec::with_capacity(8192 * backing_channels);

    // 啟動 producer 線程（需同時滿足：channels ≤ 2 且 F POC 有開啟 producer 路徑）
    let producer_handle = if can_use_stream_processor && USE_PRODUCER_PATH {
        let backing_for_producer = backing.clone();
        let producer_running_clone = producer_running.clone();
        let producer_seek_clone = producer_seek_to.clone();
        let loop_range_prod = shared.loop_range.clone();
        let running_main = shared.running.clone();

        Some(thread::spawn(move || {
            stretch_producer_worker(
                backing_for_producer,
                backing_channels,
                source_sr,
                out_sr,
                total_frames,
                ring_producer,
                producer_running_clone,
                producer_seek_clone,
                producer_speed,
                producer_pitch,
                loop_range_prod,
                running_main,
            );
        }))
    } else {
        // 不用 stretch producer 時：ring_producer 在這裡 drop，省掉 DSP 白工
        drop(ring_producer);
        None
    };

    // Consumer 端：move 擁有權進 callback closure
    let mut ring_cons: HeapCons<f32> = ring_consumer;

    // WSOLA 處理器（由 closure 擁有，跨 callback 持續存活）
    let mut wsola = crate::core::wsola::WsolaProcessor::new(backing_channels);
    let mut wsola_buf: Vec<f32> = Vec::new();

    // Pitch-shift anti-aliasing LPF state（升調時需要）
    // (2026-04-15, 選項 1 — 5 階 cascade): 從 3 級（6 階 Butterworth）升到 5 級（10 階），
    //   抑制力約 -60 dB/oct；+5 semi 邊界衰減 ~-22 dB → ~-37 dB（+15 dB 改善），
    //   針對 hi-hat/cymbal/sibilance 觸發的低頻 aliasing 殘音。CPU 小幅增加、零額外延遲。
    //   若過悶可回退成 3 級（改 array 長度 + init/clear/apply loop）。
    let mut pitch_aa_filters: Vec<[Option<biquad::DirectForm1<f32>>; 5]> = (0..backing_channels)
        .map(|_| [None, None, None, None, None])
        .collect();
    let mut pitch_aa_last_st: i32 = 0; // 追蹤上次的 pitch_st 以避免重複建 filter

    // P0: 追蹤 callback 端看到的 pitch 值，變動時需視為 seek 級 discontinuity
    // （ring 裡 buffered 的資料是用舊 pitch_ratio 拉伸的，consumer 如果直接用新
    //  combined_resample 去讀會 pitch/timing 錯位，造成 128-314ms transient glitch）
    let mut last_pitch_st: i32 = 0;
    // P1-b: underflow 追蹤（ring 完全空 → 輸出靜音的幀數）
    // 用來在推進 stretch_play_pos 時扣除，避免 vocal mix 時間軸飄掉
    let mut underflow_total: u64 = 0;
    let mut underflow_log_gate: u64 = 0;
    // E (2026-04-15, Codex round-2 instrumentation): partial-hold fallback 計數器
    //   當 ring 短缺（但非完全空）時 consumer 會 hold 最後有效幀（audio_engine.rs:1864-1874），
    //   這種情況不會輸出靜音、既有 log 也抓不到，但每次發生都是一次低頻 artifact
    //   （callback ~86Hz → 對應「波波波」的感知頻率）+ stretch_play_pos 照常推進（1918-1935）
    //   → 「變快」的直接成因。這個計數器量化 baseline 的 partial-hold 頻率，
    //   F POC 驗證後可用此數據對照 before/after。
    let mut hold_total: u64 = 0;

    let err_fn = move |err| log::error!("Output stream error: {}", err);

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _info| {
            let cur_pos = pos.load(Ordering::Relaxed);
            let vol = f32::from_bits(volume.load(Ordering::Relaxed));
            let cur_speed = (f32::from_bits(speed_atomic.load(Ordering::Relaxed)) as f64).max(0.1);
            let cur_pitch_st = pitch_atomic.load(Ordering::Relaxed) as i32;
            let pitch_ratio = 2.0_f64.powf(cur_pitch_st as f64 / 12.0);

            // P0: pitch 變動 = transport discontinuity
            //   ring 裡 buffered 的是用舊 pitch_ratio 拉伸的音訊，consumer 改用新的
            //   combined_resample 去讀會輸出錯誤的 pitch/timing（~128-314ms glitch）。
            //   處理方式與 seek 同級：flush ring + carry + reset AA state，並通知
            //   producer seek 到 cur_pos 用新 pitch_ratio 重新 process。
            if cur_pitch_st != last_pitch_st {
                if stretch_active_prev {
                    producer_seek_to.store(cur_pos + 1, Ordering::Release);
                    let discard = ring_cons.occupied_len();
                    ring_cons.skip(discard);
                    carry_buf.clear();
                    ring_consume_frac = 0.0;
                    stretch_play_pos = cur_pos as f64;
                    reset_pitch_aa_state(&mut pitch_aa_filters);
                }
                if wsola_active_prev {
                    // WSOLA 不經過 ring，但 AA filter state 要清，避免舊 pitch 能量滲入
                    wsola.set_input_pos(cur_pos as f64);
                    reset_pitch_aa_state(&mut pitch_aa_filters);
                }
                last_pitch_st = cur_pitch_st;
            }

            let needs_stretch = (cur_speed - 1.0).abs() > 0.01 || cur_pitch_st != 0;
            // 路徑選擇：
            //   bypass              → speed=1.0 且 pitch=0
            //   timestretch producer → speed=1.0 且 |pitch| <= 12（純移調，走純 stretch + cubic resample）
            //   WSOLA               → 其他（變速 / 變速+移調）方向已有單元測試覆蓋
            //
            // Producer path 採「純時間拉伸 + consumer resample」策略：
            //   不使用 timestretch 的 pitch_scale（有 streaming 累積誤差 bug），
            //   而是讓 timestretch 做純 stretch（pitch_ratio 倍），
            //   consumer 以 combined_resample = pitch_ratio × rate_ratio 做 cubic resample。
            //   這樣 ±2、±3…±12 都能用 HouseLoop phase vocoder 的高品質，
            //   同時繞過 pitch_scale 的時間漂移問題。
            // F POC (2026-04-15): 強制 pure pitch shift 改走 WSOLA 分支
            //   根因假說（Codex round-2）：baseline「波波波 + 變快」共同根因是 consumer
            //   partial-hold fallback（audio_engine.rs:1864-1874）+ stretch_play_pos 照常推進
            //   （1918-1935）。WSOLA 不經過 ring buffer，沒有 partial-hold 問題。
            //   驗證症狀是否一併消失；E 的 hold_total 可供 before/after 量化對照。
            //   producer 線程仍會啟動（~1634），ring 會被寫滿但 consumer 不讀→不影響聲音，
            //   只有 CPU/memory 小浪費，POC 驗證通過後會一併清理。
            //
            // 原條件（保留供 revert 參考）：
            //   let speed_is_one = (cur_speed - 1.0).abs() < 0.01;
            //   let use_producer_path = needs_stretch && speed_is_one
            //       && can_use_stream_processor && cur_pitch_st.abs() <= 12;
            let use_producer_path = USE_PRODUCER_PATH;

            if !needs_stretch {
                if stretch_active_prev {
                    // 從 producer 路徑切回 bypass：清空 ring buffer 與 carry，重置狀態
                    let discard = ring_cons.occupied_len();
                    ring_cons.skip(discard);
                    carry_buf.clear();
                    stretch_play_pos = cur_pos as f64;
                    ring_consume_frac = 0.0;
                    stretch_active_prev = false;
                }
                if wsola_active_prev {
                    // 從 WSOLA 路徑切回 bypass：WSOLA 下次進 set_input_pos 會自動同步，
                    // 這裡只標記狀態即可
                    wsola_active_prev = false;
                }

                // ── Bypass：原始直接讀取路徑（零 WSOLA 開銷）──
                let vocal_ref = vocal_cb.as_ref().map(|v| v.as_slice());
                let (mut new_pos, rms) = fill_backing_output(
                    data,
                    &backing_cb,
                    backing_channels,
                    out_channels,
                    rate_ratio,
                    total_frames,
                    cur_pos,
                    vol,
                    vocal_ref,
                    latency_ms,
                    source_sr,
                );

                if let Some((la, lb)) = unpack_loop(loop_range.load(Ordering::Relaxed)) {
                    if new_pos >= lb {
                        new_pos = la;
                    }
                }

                pos.store(new_pos, Ordering::Relaxed);
                backing_rms.store(rms.to_bits(), Ordering::Relaxed);

                if new_pos >= total_frames {
                    running_cb.store(false, Ordering::Relaxed);
                }
            } else if use_producer_path {
                // ── Ring buffer consumer 路徑（lock-free SPSC，處理純移調 ±12 半音）──
                //
                // 所有 DSP（timestretch pure stretch + AA LPF）已在 producer 線程完成，
                // 這裡只做：pop → cubic interpolation（with resample）→ 音量 → vocal mix → 輸出
                //
                // Ring 內容：timestretch 把時間拉長 pitch_ratio 倍的「原音高」音訊
                // Consumer 以 combined_resample = pitch_ratio × rate_ratio 為步長讀
                //   → 時間縮短 pitch_ratio 倍（抵銷拉長，回到原速）
                //   → 頻率 × pitch_ratio（達到移調）
                //   → 同時吃掉 rate_ratio（SR 轉換）
                //
                // 關鍵設計：
                // - local_buf = carry_buf（上次 callback 剩餘）+ 本次 pop 的新 sample
                // - cubic 需要 x0..x3 四幀 preview；但邏輯上只「消費」consumed_whole 幀
                // - 剩下的尾端幀（= preview 部分）存進 carry_buf，下次 callback 繼續用
                if wsola_active_prev {
                    // 從 WSOLA 切回 producer 路徑：WSOLA 狀態標記為 inactive，
                    // 下次若再進 WSOLA 會重新 set_input_pos；這裡不用主動 reset
                    wsola_active_prev = false;
                }
                let frame_count = data.len() / out_channels;
                let loop_snapshot = unpack_loop(loop_range.load(Ordering::Relaxed));

                if !stretch_active_prev || (cur_pos as f64 - stretch_play_pos).abs() > 4.0 {
                    // 通知 producer 跳到新位置（+1 偏移避免 0 值歧義）
                    producer_seek_to.store(cur_pos + 1, Ordering::Release);
                    // 清空 ring buffer 與 carry 中的舊資料
                    let discard = ring_cons.occupied_len();
                    ring_cons.skip(discard);
                    carry_buf.clear();
                    stretch_play_pos = cur_pos as f64;
                    ring_consume_frac = 0.0;
                }
                stretch_active_prev = true;

                // Consumer 讀 ring 的步長 = 時間壓縮率 × SR 轉換
                let combined_resample = pitch_ratio * rate_ratio;

                // cubic interpolation 需要 4 幀 preview（x0, x1, x2, x3）
                let preview_frames: usize = 4;
                // 本次 callback 需要的總幀數 = 最大 local_pos 對應的整數幀 + preview
                let needed_frames: usize =
                    (ring_consume_frac + frame_count as f64 * combined_resample).ceil() as usize
                        + preview_frames;
                let needed_samples = needed_frames * backing_channels;

                // local_buf = carry_buf + 新 pop 的資料
                local_buf.clear();
                local_buf.extend_from_slice(&carry_buf);
                if local_buf.len() < needed_samples {
                    let deficit = needed_samples - local_buf.len();
                    let available = ring_cons.occupied_len();
                    let to_read = deficit.min(available);
                    if to_read > 0 {
                        let old_len = local_buf.len();
                        local_buf.resize(old_len + to_read, 0.0);
                        let got = ring_cons.pop_slice(&mut local_buf[old_len..old_len + to_read]);
                        // pop_slice 回傳實際拷貝量，理論上等於 to_read（因為已經 min available）
                        if got < to_read {
                            local_buf.truncate(old_len + got);
                        }
                    }
                }

                let available_frames = local_buf.len() / backing_channels.max(1);
                let vocal_ref = vocal_cb.as_ref().map(|v| v.as_slice());
                let latency_frames = latency_ms / 1000.0 * source_sr as f64;

                let mut sum_sq = 0.0_f32;
                let mut sample_count = 0_usize;
                // P1-b: 追蹤 ring 完全無資料（輸出靜音）的幀數
                //   這種 underflow 下 stretch_play_pos 不該推進——否則 vocal mix 會漸漸
                //   對不齊。hold 最後一幀的輕微 underflow 則仍推進（避免無限卡住）。
                let mut silent_frames: usize = 0;
                // E: partial-hold（hold 最後有效幀）發生次數——baseline「波波波」的候選根因
                let mut hold_frames: usize = 0;

                for frame in 0..frame_count {
                    // local_pos：在 pitch-stretched ring 中的讀位置（combined_resample 步長）
                    let mut local_pos = ring_consume_frac + frame as f64 * combined_resample;
                    // timeline_pos：原 source frame 軸（給 vocal mix 對齊用）
                    // 注意：producer path 只在 cur_speed=1.0 運行，所以 source 推進 = frame × rate_ratio
                    let timeline_pos = stretch_play_pos + frame as f64 * rate_ratio * cur_speed;

                    if local_pos + 1.0 >= available_frames as f64 {
                        // Ring buffer 暫時不足——平滑 hold 最後一個有效幀
                        if available_frames >= 2 {
                            local_pos = (available_frames - 2) as f64;
                            hold_frames += 1;
                        } else {
                            for ch in 0..out_channels {
                                data[frame * out_channels + ch] = 0.0;
                            }
                            silent_frames += 1;
                            continue;
                        }
                    }

                    for ch in 0..out_channels {
                        let src_ch = ch.min(backing_channels - 1);
                        // 與 HouseLoop 搭配實測：cubic 的 -14dB stopband 反而比 Lanczos-3
                        // 更穩（HouseLoop 的 HPSS+elastic_timing 有微相位跳變，Lanczos-3
                        // 的 6-tap 支撐會把它擴散成 ringing）。
                        let mut s = sample_cubic_interleaved(
                            &local_buf,
                            backing_channels,
                            local_pos,
                            src_ch,
                        ) * vol;

                        if let Some(vocal) = vocal_ref {
                            let vocal_pos = timeline_pos + latency_frames;
                            if vocal_pos >= 0.0 && vocal_pos + 1.0 < vocal.len() as f64 {
                                s += sample_linear_mono(vocal, vocal_pos);
                            }
                        }

                        let s = s.clamp(-1.0, 1.0);
                        data[frame * out_channels + ch] = s;
                        sum_sq += s * s;
                        sample_count += 1;
                    }
                }

                // P1-b + E: underflow / partial-hold 統計 + log 節流
                //   silent_frames：ring 完全空 → 輸出 0.0（既有 P1-b 追蹤）
                //   hold_frames：ring 只剩 <2 幀 → hold 最後有效幀（E 新增追蹤）
                //   partial-hold 時 stretch_play_pos 仍照常推進（1918-1935），是「變快」的
                //   直接成因；callback ~86Hz 下連續 hold 即為「波波波」的候選根因。
                if silent_frames > 0 || hold_frames > 0 {
                    underflow_total = underflow_total.saturating_add(silent_frames as u64);
                    hold_total = hold_total.saturating_add(hold_frames as u64);
                    underflow_log_gate = underflow_log_gate.wrapping_add(1);
                    if underflow_log_gate.is_power_of_two() {
                        log::warn!(
                            "producer-path underflow: silent={}/{} (total {}), partial-hold={}/{} (total {})",
                            silent_frames,
                            frame_count,
                            underflow_total,
                            hold_frames,
                            frame_count,
                            hold_total
                        );
                    }
                }

                // P1-b: 實際產出的幀數 = frame_count - silent_frames
                //   underflow 靜音的那幾幀不該消費 ring、不該推進時間軸
                let effective_frames = frame_count.saturating_sub(silent_frames);

                // 計算邏輯消耗幀數（ring buffer 真實被消費的 frame 數）
                let consumed_f = ring_consume_frac + effective_frames as f64 * combined_resample;
                let consumed_whole = consumed_f.floor() as usize;
                ring_consume_frac = consumed_f - consumed_whole as f64;

                // 把 consumed_whole 之後的尾端幀存進 carry_buf 下次重用
                // 這樣確保 ring buffer 實際被消費的量 = consumed_whole，不會過度消耗
                carry_buf.clear();
                let consumed_samples = (consumed_whole * backing_channels).min(local_buf.len());
                if local_buf.len() > consumed_samples {
                    carry_buf.extend_from_slice(&local_buf[consumed_samples..]);
                }

                // P1-b: 推進播放位置時扣除 underflow 靜音幀，避免 vocal mix 對齊飄掉
                let source_advance = effective_frames as f64 * rate_ratio * cur_speed;
                stretch_play_pos = (stretch_play_pos + source_advance).min(total_frames as f64);
                let mut new_pos = stretch_play_pos.round() as u64;

                if let Some((la, lb)) = loop_snapshot {
                    if new_pos >= lb {
                        new_pos = la;
                        stretch_play_pos = la as f64;
                        // 通知 producer seek 到循環起點
                        producer_seek_to.store(la + 1, Ordering::Release);
                        let discard = ring_cons.occupied_len();
                        ring_cons.skip(discard);
                        carry_buf.clear();
                        ring_consume_frac = 0.0;
                    }
                }

                pos.store(new_pos, Ordering::Relaxed);

                let rms = if sample_count > 0 {
                    (sum_sq / sample_count as f32).sqrt()
                } else {
                    0.0
                };
                backing_rms.store(rms.to_bits(), Ordering::Relaxed);

                if new_pos >= total_frames {
                    running_cb.store(false, Ordering::Relaxed);
                }
            } else {
                // ── WSOLA 路徑：變速（含變速+移調）──
                //
                // WSOLA 的 stretch 方向由單元測試覆蓋（stretch=2.0 → 慢速），
                // 避開 timestretch 0.4.0 在大 stretch_ratio 下的不穩定與方向錯誤。
                if stretch_active_prev {
                    // 從 producer 切到 WSOLA：清空 ring buffer / carry，避免殘留
                    let discard = ring_cons.occupied_len();
                    ring_cons.skip(discard);
                    carry_buf.clear();
                    stretch_play_pos = cur_pos as f64;
                    ring_consume_frac = 0.0;
                    stretch_active_prev = false;
                }
                if !wsola_active_prev {
                    // 第一次進 WSOLA：強制把 WSOLA 的 input_pos 對齊到 cur_pos，
                    // 大跳躍會觸發 WsolaProcessor::reset_state() 清空 accum_buf
                    wsola.set_input_pos(cur_pos as f64);
                    // P1-a: 路徑切換也是 discontinuity，AA filter state 要清零
                    reset_pitch_aa_state(&mut pitch_aa_filters);
                    wsola_active_prev = true;
                }
                let frame_count = data.len() / out_channels;
                let stretch = pitch_ratio / cur_speed;

                // 合併 resample ratio（pitch 補償 + SR 轉換）
                let combined_resample = pitch_ratio * rate_ratio;

                // WSOLA 需要產生的幀數（resample 前）
                // +2 保留 cubic 插值的下一幀邊距。
                //
                // 設計筆記：曾試過 Lanczos-3（6-tap sinc）取代 cubic，但 WSOLA 輸出本身
                // 帶有微小的相位不連續（hop 邊界），Lanczos-3 的長支撐會把這些不連續
                // 擴散成更明顯的 ringing，反而讓變速雜音變多。cubic 的短支撐反而穩定。
                let wsola_frames_needed =
                    (frame_count as f64 * combined_resample).ceil() as usize + 2;

                // 同步 WSOLA 位置（僅在外部跳躍時——seek / loop）
                // 正常播放時不覆寫，保留 wsola 內部的 f64 小數精度
                let wsola_pos_int = wsola.input_pos() as u64;
                if cur_pos != wsola_pos_int {
                    wsola.set_input_pos(cur_pos as f64);
                    // P1-a: 外部 seek 也要 reset AA state，避免舊片段能量滲入
                    reset_pitch_aa_state(&mut pitch_aa_filters);
                }

                // 產生 WSOLA 輸出
                let wsola_samples = wsola_frames_needed * backing_channels;
                wsola_buf.resize(wsola_samples, 0.0);
                wsola.process(&backing_cb, &mut wsola_buf[..wsola_samples], stretch);

                // ── Anti-aliasing LPF（升調時需要） ──
                // 升調 = decimation，需要先濾掉 Nyquist/total_decimation 以上的頻率
                // total_decimation = pitch_ratio × rate_ratio（含 SR 轉換）
                if cur_pitch_st > 0 && cur_pitch_st != pitch_aa_last_st {
                    use biquad::{Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};
                    let fs = (source_sr as f32).hz();
                    // G' (2026-04-15): 截止頻率 margin 0.78 → 0.55 → 0.65（折衷）
                    //   F POC 確認 baseline 波波波+變快根因後，留下「升調某段有低頻雜雜雜」的
                    //   新 issue（訊號相關，hi-hat/cymbal/sibilance 觸發 aliasing）。
                    //   - 0.78（原值）：aliasing 邊界衰減 ~13 dB，雜音明顯
                    //   - 0.55（過緊）：衰減 ~31 dB 但音色悶悶的
                    //   - 0.65（折衷）：衰減 ~22 dB，+5 semi 下 cutoff ≈ 11.7 kHz，保留足夠高頻亮度
                    //
                    // 分母 pitch_ratio × max(rate_ratio, 1.0)：
                    //   - rate_ratio ≥ 1（source_sr ≥ out_sr）→ consumer 要多一層 decimation
                    //   - rate_ratio < 1（upsample）→ 不需額外 AA，分母 clamp 到 1.0
                    let total_decimation = pitch_ratio as f32 * (rate_ratio as f32).max(1.0);
                    let f_cut = ((source_sr as f32) * 0.5 / total_decimation * 0.65).hz();
                    if let Ok(coeffs) = Coefficients::<f32>::from_params(
                        Type::LowPass,
                        fs,
                        f_cut,
                        Q_BUTTERWORTH_F32,
                    ) {
                        for ch_filters in pitch_aa_filters.iter_mut() {
                            ch_filters[0] = Some(DirectForm1::<f32>::new(coeffs));
                            ch_filters[1] = Some(DirectForm1::<f32>::new(coeffs));
                            ch_filters[2] = Some(DirectForm1::<f32>::new(coeffs));
                            ch_filters[3] = Some(DirectForm1::<f32>::new(coeffs));
                            ch_filters[4] = Some(DirectForm1::<f32>::new(coeffs));
                        }
                    }
                    pitch_aa_last_st = cur_pitch_st;
                } else if cur_pitch_st <= 0 && pitch_aa_last_st > 0 {
                    // 降調或不移調時不需要 AA filter
                    for ch_filters in pitch_aa_filters.iter_mut() {
                        ch_filters[0] = None;
                        ch_filters[1] = None;
                        ch_filters[2] = None;
                        ch_filters[3] = None;
                        ch_filters[4] = None;
                    }
                    pitch_aa_last_st = cur_pitch_st;
                }

                // 對 WSOLA 輸出施加 anti-aliasing LPF（in-place，10 階 cascade）
                // [測試 A 結論 — 2026-04-15]: bypass 實測雜音僅「減少一點點」，而降調也有
                //   同樣雜音（但降調根本不走此 AA LPF 分支）→ 確認雜音根因不是 aliasing。
                //   保留 AA LPF（升調時仍有 +15 dB 衰減保險，不害事），繼續找 WSOLA 根因。
                if cur_pitch_st > 0 {
                    use biquad::Biquad;
                    let wsola_frame_count = wsola_buf.len() / backing_channels;
                    for f in 0..wsola_frame_count {
                        for src_ch in 0..backing_channels {
                            let idx = f * backing_channels + src_ch;
                            let mut s = wsola_buf[idx];
                            for stage in 0..5 {
                                if let Some(ref mut filt) = pitch_aa_filters[src_ch][stage] {
                                    s = filt.run(s);
                                }
                            }
                            wsola_buf[idx] = s;
                        }
                    }
                }

                // Resample + 寫入 output（含音量、channel mapping）
                let mut sum_sq = 0.0_f32;
                let mut sample_count = 0_usize;

                for frame in 0..frame_count {
                    let src_f = frame as f64 * combined_resample;

                    if src_f + 1.0 >= wsola_frames_needed as f64 {
                        for ch in 0..out_channels {
                            data[frame * out_channels + ch] = 0.0;
                        }
                        continue;
                    }

                    for ch in 0..out_channels {
                        let src_ch = ch.min(backing_channels - 1);
                        // 仍用 cubic（Catmull-Rom）：短支撐避免把 WSOLA hop 邊界的
                        // 微小相位跳變擴散成長 ringing。雜音來源是 WSOLA 本身，
                        // 要靠變速+移調以外的路徑（producer path）繞過才有效。
                        let s =
                            sample_cubic_interleaved(&wsola_buf, backing_channels, src_f, src_ch);

                        let s = (s * vol).clamp(-1.0, 1.0);
                        data[frame * out_channels + ch] = s;
                        sum_sq += s * s;
                        sample_count += 1;
                    }
                }

                // 更新位置（根據 WSOLA 實際消耗）
                let mut new_pos = wsola.input_pos() as u64;

                if let Some((la, lb)) = unpack_loop(loop_range.load(Ordering::Relaxed)) {
                    if new_pos >= lb {
                        new_pos = la;
                        wsola.set_input_pos(new_pos as f64);
                        // P1-a: loop 邊界跳躍也是 discontinuity，AA filter state 要清零
                        reset_pitch_aa_state(&mut pitch_aa_filters);
                    }
                }

                pos.store(new_pos, Ordering::Relaxed);

                let rms = if sample_count > 0 {
                    (sum_sq / sample_count as f32).sqrt()
                } else {
                    0.0
                };
                backing_rms.store(rms.to_bits(), Ordering::Relaxed);

                if new_pos >= total_frames {
                    running_cb.store(false, Ordering::Relaxed);
                }
            }
        },
        err_fn,
        None,
    );

    let output_stream = match stream {
        Ok(s) => s,
        Err(e) => {
            events::emit_error(&app, &format!("無法建立輸出串流：{}", e));
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    // ── 可選的輸入串流（預覽模式音高偵測）───────────────────────────
    //
    // 若 input_device_idx 有值，開啟麥克風做即時音高偵測。
    // 與錄音模式不同：不寫入 vocal_buffer，只更新 current_pitch。
    let _input_stream = if let Some(in_idx) = input_device_idx {
        build_preview_pitch_stream(&host, in_idx, source_sr, &shared, pitch_engine_pref)
    } else {
        None
    };

    if let Err(e) = output_stream.play() {
        events::emit_error(&app, &format!("無法啟動播放：{}", e));
        shared.running.store(false, Ordering::Relaxed);
        return;
    }

    // 啟動可選的 input stream
    if let Some(ref is) = _input_stream {
        if let Err(e) = is.play() {
            // input stream 失敗不中斷播放，只是沒有音高偵測
            log::warn!("預覽模式音高偵測啟動失敗：{}", e);
        }
    }

    let has_pitch_input = _input_stream.is_some();
    let state_name = if mix_vocal {
        "playing_back"
    } else {
        "previewing"
    };
    events::emit_state(&app, state_name);

    // Loop：emit 進度、RMS、音高
    let mut last_emitted_pitch_ts: f64 = -1.0;
    while running.load(Ordering::Relaxed) {
        let cur_pos = shared.playback_pos.load(Ordering::Relaxed);
        let elapsed = cur_pos as f64 / source_sr as f64;
        let rms = f32::from_bits(shared.backing_rms.load(Ordering::Relaxed));
        let mic_rms = if has_pitch_input {
            f32::from_bits(shared.mic_rms.load(Ordering::Relaxed))
        } else {
            0.0
        };

        events::emit_progress(&app, elapsed, duration_s);
        events::emit_rms(&app, rms, mic_rms);

        // 推送音高（預覽模式有 input stream 時）
        if has_pitch_input {
            if let Ok(pitch_opt) = shared.current_pitch.lock() {
                match pitch_opt.as_ref() {
                    Some(p) if p.timestamp != last_emitted_pitch_ts => {
                        last_emitted_pitch_ts = p.timestamp;
                        events::emit_pitch(
                            &app,
                            events::PitchPayload {
                                freq: p.freq,
                                note: p.note.clone(),
                                octave: p.octave,
                                cent: p.cent,
                                confidence: p.confidence,
                            },
                        );
                    }
                    None => {
                        if last_emitted_pitch_ts >= 0.0 {
                            last_emitted_pitch_ts = -1.0;
                            events::emit_pitch_none(&app);
                        }
                    }
                    _ => {}
                }
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    // 停止 stretch producer 線程
    producer_running.store(false, Ordering::Release);
    if let Some(handle) = producer_handle {
        let _ = handle.join();
    }

    drop(_input_stream);
    drop(output_stream);
    events::emit_state(&app, "idle");
    events::emit_finished(&app);
}

/// Stretch producer 背景線程：持續預先生成 stretched 音訊並寫入 ring buffer。
///
/// 所有計算密集的 DSP（timestretch + resample LPF）都在這裡完成，
/// 讓 CPAL output callback 成為純 consumer，零 DSP 運算。
///
/// 採用 lock-free SPSC：`ring_prod` 直接以擁有權傳入（無 Mutex），
/// 因為只有這個 producer 線程寫入，callback 是唯一 consumer。
#[allow(clippy::too_many_arguments)]
fn stretch_producer_worker(
    backing: Arc<Vec<f32>>,
    backing_channels: usize,
    source_sr: u32,
    out_sr: u32,
    total_frames: u64,
    mut ring_prod: HeapProd<f32>,
    producer_running: Arc<AtomicBool>,
    seek_to: Arc<AtomicU64>,
    speed_atomic: Arc<AtomicU32>,
    pitch_atomic: Arc<AtomicU32>,
    loop_range: Arc<AtomicU64>,
    main_running: Arc<AtomicBool>,
) {
    // Producer 僅在「純移調」時工作（speed=1.0，|pitch| <= 12）。
    //
    // 關鍵設計：採用「純時間拉伸」策略，避開 pitch_scale 的 streaming resampler bug。
    //   Producer: set_stretch_ratio(pitch_ratio), set_pitch_scale(1.0)
    //     → 輸出「音高不變、時間被拉長 pitch_ratio 倍」的音訊
    //   Consumer: cubic resample by combined_resample (= pitch_ratio × rate_ratio)
    //     → 同時壓縮時間軸（加快回原速）+ 升/降音高
    //
    // 為何不用 set_pitch_scale(pitch_ratio)：
    //   timestretch 0.4.0 的 pitch_scale 內部用線性串流 resampler，|pitch| >= 2
    //   時會累積時間誤差，聽起來音訊會逐漸變快。純 stretch 沒這個問題。
    //
    // Preset 選擇：HouseLoop + EnvelopePreset::Vocal
    //   實測 VocalChop 雖然雜音較少，但 consumer 用 combined_resample = pitch_ratio ×
    //   rate_ratio 的假設在 VocalChop 下會造成音訊「變快」，意味著該 preset 的實際
    //   輸出比例 K ≠ pitch_ratio（可能 crate 行為、或 elastic_timing 缺席造成）。
    //   HouseLoop 的 elastic_timing=true + anchor=0.1 配合 HPSS 在此架構下時間軸剛好
    //   匹配 consumer 的假設，是目前 proved-working 的穩定基線。
    //
    // 階段 1 實驗結論（2026-04-15）：
    //   曾嘗試 EnvelopePreset::Balanced + WindowType::BlackmanHarris 想消除「波波波」
    //   雜音，結果雜音無變化且正向移調變快。證實 preset 切換（envelope / window）
    //   會影響 stream mode 下的實際輸出時間軸（Codex 說只影響 DSP 的診斷不完整）。
    //   HouseLoop + Vocal 仍是目前唯一既穩定又時間軸正確的組合。
    //
    // 為何 ±7 半音安全：pitch_ratio 範圍 0.5–2.0 在 HouseLoop 都是 well-tested 工作
    // 區；超過 ±7 musical noise 明顯（前端 / 後端都做 clamp）。
    let params = StretchParams::new(1.0)
        .with_sample_rate(source_sr)
        .with_channels(backing_channels as u32)
        .with_preset(EdmPreset::HouseLoop)
        .with_envelope_preset(EnvelopePreset::Vocal);
    let mut processor = StreamProcessor::new(params);
    let mut output_resample_filters =
        build_output_resample_filters(backing_channels, source_sr, out_sr);
    // Pitch anti-aliasing LPF：升調時限制 chunk_buf 頻譜，避免 consumer 的 resample
    // 把 Nyquist 附近能量折射成 aliasing。每 channel 5 階 cascade（= 10 階 Butterworth）
    // (2026-04-15, 選項 1): 與 WSOLA 路徑同步升到 5 級；F POC 下此路徑雖未被 consumer
    //   取用（use_producer_path=false），但保持一致避免將來 revert F POC 時再追改。
    let mut pitch_aa_filters: Vec<[Option<biquad::DirectForm1<f32>>; 5]> = (0..backing_channels)
        .map(|_| [None, None, None, None, None])
        .collect();
    let mut pitch_aa_last_st: i32 = 0;
    let mut source_pos: u64 = 0;
    let mut chunk_buf: Vec<f32> = Vec::with_capacity(262_144);

    // 等 consumer 發出第一次 seek 信號確定起始位置
    while producer_running.load(Ordering::Relaxed) && main_running.load(Ordering::Relaxed) {
        let seek_val = seek_to.swap(0, Ordering::AcqRel);
        if seek_val > 0 {
            source_pos = seek_val - 1;
            processor.reset();
            output_resample_filters =
                build_output_resample_filters(backing_channels, source_sr, out_sr);
            // P1-a: seek 級 discontinuity，AA filter state 也要清零
            reset_pitch_aa_state(&mut pitch_aa_filters);
            break;
        }
        thread::sleep(Duration::from_micros(500));
    }

    while producer_running.load(Ordering::Relaxed) && main_running.load(Ordering::Relaxed) {
        // 檢查 seek 請求
        let seek_val = seek_to.swap(0, Ordering::AcqRel);
        if seek_val > 0 {
            source_pos = seek_val - 1;
            processor.reset();
            output_resample_filters =
                build_output_resample_filters(backing_channels, source_sr, out_sr);
            // P1-a: seek 級 discontinuity，AA filter state 也要清零
            reset_pitch_aa_state(&mut pitch_aa_filters);
        }

        // 讀取當前 speed/pitch 參數
        let cur_speed = (f32::from_bits(speed_atomic.load(Ordering::Relaxed)) as f64).max(0.1);
        let cur_pitch_st = pitch_atomic.load(Ordering::Relaxed) as i32;
        let speed_is_one = (cur_speed - 1.0).abs() < 0.01;
        // Producer 負責「純移調」情境（speed=1.0，|pitch| <= 12 半音）。
        // 變速（或變速+移調）由 callback 走 WSOLA 路徑。
        let should_produce = speed_is_one && cur_pitch_st != 0 && cur_pitch_st.abs() <= 12;

        if !should_produce {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        // 設定 stretch 參數：純時間拉伸（pitch 不變），由 consumer 負責 resample
        //   pitch_ratio = 2^(semitone / 12)，範圍 0.5–2.0（±12 半音）
        //   stretch_ratio = pitch_ratio → 時間拉長 pitch_ratio 倍
        //   Consumer 以 combined_resample = pitch_ratio × rate_ratio 讀
        //     → 讀得更快 pitch_ratio 倍，時間縮回原速，同時頻率 × pitch_ratio ✓
        let pitch_ratio = 2.0_f64.powf(cur_pitch_st as f64 / 12.0);
        if processor
            .set_stretch_ratio(pitch_ratio.clamp(0.5, 2.0))
            .is_err()
            || processor.set_pitch_scale(1.0).is_err()
        {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        // Rebuild pitch anti-aliasing LPF 當 cur_pitch_st 變動時
        // 升調 = consumer 會 decimation（combined_resample > 1）→ 需限制頻譜
        // 降調 = consumer 會 interpolation（combined_resample < 1）→ 無需 AA
        if cur_pitch_st > 0 && cur_pitch_st != pitch_aa_last_st {
            use biquad::{Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};
            let fs = (source_sr as f32).hz();
            // G' (2026-04-15): 截止頻率 margin 0.78 → 0.55 → 0.65（與 WSOLA path 一致）
            //   F POC 下 producer 線程不再被 consumer 讀（use_producer_path=false），
            //   但這裡同步改動以保持兩條路徑一致，將來 revert F POC 時不用再追改。
            //   0.65 折衷：aliasing 邊界衰減 ~22 dB，保留升調高頻亮度，
            //   實測 0.55 過悶、0.78 雜音明顯，0.65 為妥協值。
            //
            // 分母納入 max(rate_ratio, 1.0)：consumer 的 cubic resample 還會再做一次
            // SR 轉換 decimation，若 source_sr > out_sr 要把這層也算進來。
            let rate_ratio_f = (source_sr as f32 / out_sr as f32).max(1.0);
            let total_decimation = pitch_ratio as f32 * rate_ratio_f;
            let f_cut = ((source_sr as f32) * 0.5 / total_decimation * 0.65).hz();
            if let Ok(coeffs) =
                Coefficients::<f32>::from_params(Type::LowPass, fs, f_cut, Q_BUTTERWORTH_F32)
            {
                for ch_filters in pitch_aa_filters.iter_mut() {
                    ch_filters[0] = Some(DirectForm1::<f32>::new(coeffs));
                    ch_filters[1] = Some(DirectForm1::<f32>::new(coeffs));
                    ch_filters[2] = Some(DirectForm1::<f32>::new(coeffs));
                    ch_filters[3] = Some(DirectForm1::<f32>::new(coeffs));
                    ch_filters[4] = Some(DirectForm1::<f32>::new(coeffs));
                }
            }
            pitch_aa_last_st = cur_pitch_st;
        } else if cur_pitch_st <= 0 && pitch_aa_last_st > 0 {
            for ch_filters in pitch_aa_filters.iter_mut() {
                ch_filters[0] = None;
                ch_filters[1] = None;
                ch_filters[2] = None;
                ch_filters[3] = None;
                ch_filters[4] = None;
            }
            pitch_aa_last_st = cur_pitch_st;
        }

        // 檢查 ring buffer 空間（lock-free 直接讀）
        let free_samples = ring_prod.vacant_len();

        // P2-b: producer 輸出量 ≈ chunk_in × pitch_ratio，最大 × 2.0
        //   min_free = 8192 × channels：約 ~0.18s 的 worst-case 邊距（chunk_in 4096 × 2x）
        //   降低延遲（從 16384 的 ~0.37s 減半），同時保留 chunk 不被截斷的空間
        //   若壓力測試出 underflow（log::warn 會回報），再調回 12288 × channels
        let min_free = 8192 * backing_channels;
        if free_samples < min_free {
            // Ring buffer 快滿了，等 consumer 消費
            thread::sleep(Duration::from_micros(500));
            continue;
        }

        // 計算本輪可以處理的範圍
        let feed_limit = unpack_loop(loop_range.load(Ordering::Relaxed))
            .map(|(_, lb)| lb)
            .unwrap_or(total_frames);

        if source_pos >= feed_limit || source_pos >= total_frames {
            // 已到結尾或循環邊界，等待 seek 信號
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        let chunk_end = (source_pos + 4096).min(feed_limit).min(total_frames);
        if chunk_end <= source_pos {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        let src_start = source_pos as usize * backing_channels;
        let src_end = chunk_end as usize * backing_channels;
        chunk_buf.clear();
        if processor
            .process_into(&backing[src_start..src_end], &mut chunk_buf)
            .is_err()
        {
            thread::sleep(Duration::from_millis(1));
            continue;
        }

        // Anti-aliasing LPF（source_sr > out_sr 時）
        if source_sr > out_sr && !chunk_buf.is_empty() {
            use biquad::Biquad;
            let chunk_frames = chunk_buf.len() / backing_channels;
            for frame in 0..chunk_frames {
                for src_ch in 0..backing_channels {
                    let idx = frame * backing_channels + src_ch;
                    let mut s = chunk_buf[idx];
                    for stage in 0..2 {
                        if let Some(ref mut filt) = output_resample_filters[src_ch][stage] {
                            s = filt.run(s);
                        }
                    }
                    chunk_buf[idx] = s;
                }
            }
        }

        // Pitch AA LPF（升調時）：進一步把 chunk_buf 頻譜限制在 Nyquist/pitch_ratio 以下
        // 這樣 consumer 以 combined_resample 做 cubic resample 時不會 alias
        // (2026-04-15, 選項 1): 5 級 cascade = 10 階 Butterworth
        if cur_pitch_st > 0 && !chunk_buf.is_empty() {
            use biquad::Biquad;
            let chunk_frames = chunk_buf.len() / backing_channels;
            for frame in 0..chunk_frames {
                for src_ch in 0..backing_channels {
                    let idx = frame * backing_channels + src_ch;
                    let mut s = chunk_buf[idx];
                    for stage in 0..5 {
                        if let Some(ref mut filt) = pitch_aa_filters[src_ch][stage] {
                            s = filt.run(s);
                        }
                    }
                    chunk_buf[idx] = s;
                }
            }
        }

        // 寫入 ring buffer（lock-free push）
        if !chunk_buf.is_empty() {
            let writable = ring_prod.vacant_len().min(chunk_buf.len());
            if writable > 0 {
                ring_prod.push_slice(&chunk_buf[..writable]);
            }
        }

        source_pos = chunk_end;
    }
}

/// 為預覽模式建立可選的輸入串流（純音高偵測，不寫入 vocal_buffer）。
///
/// 與 run_recording 的 input stream 類似，但：
/// - 不做重採樣（pitch 分析用原始 in_sr）
/// - 不寫入 vocal_buffer
/// - 只更新 shared.current_pitch 和 shared.mic_rms
fn build_preview_pitch_stream(
    host: &cpal::Host,
    input_device_idx: usize,
    source_sr: u32,
    shared: &SharedState,
    pitch_engine_pref: &str,
) -> Option<cpal::Stream> {
    let input_device = host
        .input_devices()
        .ok()
        .and_then(|mut devs| devs.nth(input_device_idx))
        .or_else(|| host.default_input_device())?;

    let input_config = build_input_config(&input_device, source_sr).ok()?;
    let in_channels = input_config.channels as usize;
    let in_sr = input_config.sample_rate.0;

    // ── CREPE / YIN 選擇 ──
    let crepe_model_dir = crepe_engine::find_crepe_model_dir();
    let use_crepe = match pitch_engine_pref {
        "crepe" => crepe_model_dir.is_some(),
        "yin" => false,
        _ => crepe_model_dir.is_some(),
    };
    let crepe_model_path = crepe_model_dir.unwrap_or_default();

    // CREPE state
    let mut crepe_resampler = StreamingResampler::new(in_sr, crepe_engine::CREPE_SAMPLE_RATE);
    let mut crepe_buf: Vec<f32> = Vec::with_capacity(crepe_engine::CREPE_FRAME_SIZE * 2);
    let crepe_confidence_threshold = 0.3;

    // YIN fallback state
    let pitch_buf_size: usize = if in_sr > 48000 {
        if in_sr > 96000 {
            16384
        } else {
            8192
        }
    } else {
        PITCH_BUF_SIZE
    };
    let pitch_hop_size: usize = pitch_buf_size / 2;
    let mut pitch_buf: Vec<f32> = Vec::with_capacity(pitch_buf_size);
    let mut pitch_detector = PitchDetector::new(in_sr, pitch_buf_size, 50.0, 1000.0, 0.15, 0.01);

    // 即時平滑 state
    let mut smooth_freq: f64 = 0.0;
    let mut smooth_active: bool = false;
    const SMOOTH_ALPHA: f64 = 0.35;

    let mic_gain = shared.mic_gain.clone();
    let mic_rms_atomic = shared.mic_rms.clone();
    let current_pitch_share = shared.current_pitch.clone();
    let pos_for_pitch = shared.playback_pos.clone();

    if use_crepe {
        log::info!(
            "[preview_pitch] CREPE realtime, in_sr={in_sr} Hz → 16kHz, model={}",
            crepe_model_path.display()
        );
    } else {
        log::info!("[preview_pitch] YIN fallback, in_sr={in_sr} Hz, buf_size={pitch_buf_size}");
    }

    let in_err_fn = move |err| log::error!("Preview input stream error: {}", err);

    let stream = input_device
        .build_input_stream(
            &input_config,
            move |data: &[f32], _info| {
                let frame_count = data.len() / in_channels;
                let gain = f32::from_bits(mic_gain.load(Ordering::Relaxed));

                let mut sum_sq = 0.0_f32;
                let mut sample_count = 0_usize;
                let mut raw_mono_batch: Vec<f32> = Vec::with_capacity(frame_count);

                for frame in 0..frame_count {
                    let mut mono = 0.0_f32;
                    for ch in 0..in_channels {
                        mono += data[frame * in_channels + ch];
                    }
                    mono = ((mono / in_channels as f32) * gain).clamp(-1.0, 1.0);
                    sum_sq += mono * mono;
                    sample_count += 1;
                    raw_mono_batch.push(mono);
                }

                // 音高分析（與 run_recording 相同邏輯）
                if use_crepe {
                    let resampled = crepe_resampler.process(&raw_mono_batch);
                    crepe_buf.extend_from_slice(&resampled);

                    while crepe_buf.len() >= crepe_engine::CREPE_FRAME_SIZE {
                        let cur_pos = pos_for_pitch.load(Ordering::Relaxed);
                        let timestamp = cur_pos as f64 / source_sr as f64;

                        let frame_data = &crepe_buf[..crepe_engine::CREPE_FRAME_SIZE];
                        match crepe_engine::detect_realtime(
                            frame_data,
                            timestamp,
                            crepe_confidence_threshold,
                            &crepe_model_path,
                        ) {
                            Ok(Some(sample)) => {
                                if smooth_active {
                                    smooth_freq = SMOOTH_ALPHA * sample.freq
                                        + (1.0 - SMOOTH_ALPHA) * smooth_freq;
                                } else {
                                    smooth_freq = sample.freq;
                                    smooth_active = true;
                                }
                                let smoothed = PitchSample {
                                    freq: smooth_freq,
                                    ..sample
                                };
                                if let Ok(mut p) = current_pitch_share.lock() {
                                    *p = Some(smoothed);
                                }
                            }
                            Ok(None) => {
                                smooth_active = false;
                                if let Ok(mut p) = current_pitch_share.lock() {
                                    *p = None;
                                }
                            }
                            Err(e) => {
                                log::warn!("[preview_pitch] CREPE error: {}", e);
                            }
                        }

                        crepe_buf.drain(..crepe_engine::CREPE_FRAME_SIZE / 2);
                    }
                } else {
                    // YIN fallback
                    pitch_buf.extend_from_slice(&raw_mono_batch);
                    while pitch_buf.len() >= pitch_buf_size {
                        let cur_pos = pos_for_pitch.load(Ordering::Relaxed);
                        let timestamp = cur_pos as f64 / source_sr as f64;

                        if let Some(sample) =
                            pitch_detector.detect(&pitch_buf[..pitch_buf_size], timestamp)
                        {
                            if let Ok(mut p) = current_pitch_share.lock() {
                                *p = Some(sample);
                            }
                        } else if let Ok(mut p) = current_pitch_share.lock() {
                            *p = None;
                        }

                        pitch_buf.drain(..pitch_hop_size);
                    }
                }

                // 更新 mic RMS
                if sample_count > 0 {
                    let rms = (sum_sq / sample_count as f32).sqrt();
                    mic_rms_atomic.store(rms.to_bits(), Ordering::Relaxed);
                }
            },
            in_err_fn,
            None,
        )
        .ok()?;

    Some(stream)
}

/// 錄音 worker：同時播放伴奏 + 從麥克風收音
#[allow(clippy::too_many_arguments)]
fn run_recording(
    app: AppHandle,
    backing: Arc<Vec<f32>>,
    vocal_buffer: Arc<Mutex<Vec<f32>>>,
    pitch_track: Arc<Mutex<PitchTrack>>,
    backing_channels: usize,
    source_sr: u32,
    total_frames: u64,
    duration_s: f64,
    input_device_idx: Option<usize>,
    output_device_idx: Option<usize>,
    shared: SharedState,
    pitch_engine_pref: &str,
) {
    let host = cpal::default_host();

    // ── 輸出（伴奏播放）──
    let output_device = output_device_idx
        .and_then(|idx| {
            host.output_devices()
                .ok()
                .and_then(|mut devs| devs.nth(idx))
        })
        .or_else(|| host.default_output_device());

    let output_device = match output_device {
        Some(d) => d,
        None => {
            events::emit_error(&app, "找不到輸出裝置");
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    let output_config = match build_output_config(&output_device, source_sr) {
        Ok(c) => c,
        Err(e) => {
            events::emit_error(&app, &format!("輸出設定錯誤：{}", e));
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };
    let out_channels = output_config.channels as usize;
    let out_sr = output_config.sample_rate.0;
    let rate_ratio = source_sr as f64 / out_sr as f64;

    // ── 輸入（麥克風收音）──
    let input_device = input_device_idx
        .and_then(|idx| host.input_devices().ok().and_then(|mut devs| devs.nth(idx)))
        .or_else(|| host.default_input_device());

    let input_device = match input_device {
        Some(d) => d,
        None => {
            events::emit_error(&app, "找不到輸入裝置");
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    // 診斷 log：把 CPAL 看到的所有 input 裝置資訊印出來，方便診斷音質問題
    let device_name = input_device
        .name()
        .unwrap_or_else(|_| "<unknown device>".to_string());
    println!("=== [audio_engine] 輸入裝置診斷 ===");
    println!("[input] 選到的裝置：{device_name}");
    println!("[input] 請求 sample rate: {source_sr} Hz (跟 backing 一致)");

    if let Ok(default_cfg) = input_device.default_input_config() {
        println!(
            "[input] 裝置預設 config: {} ch @ {} Hz, sample_format={:?}, buffer_size={:?}",
            default_cfg.channels(),
            default_cfg.sample_rate().0,
            default_cfg.sample_format(),
            default_cfg.buffer_size(),
        );
    }

    match input_device.supported_input_configs() {
        Ok(configs) => {
            println!("[input] 裝置支援的所有 configs:");
            for (i, c) in configs.enumerate() {
                println!(
                    "  #{i}: {} ch, sr {}~{} Hz, sample_format={:?}, buffer_size={:?}",
                    c.channels(),
                    c.min_sample_rate().0,
                    c.max_sample_rate().0,
                    c.sample_format(),
                    c.buffer_size(),
                );
            }
        }
        Err(e) => {
            println!("[input] 無法列舉 supported configs: {e:?}");
        }
    }

    let input_config = match build_input_config(&input_device, source_sr) {
        Ok(c) => c,
        Err(e) => {
            events::emit_error(&app, &format!("輸入設定錯誤：{}", e));
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };
    let in_channels = input_config.channels as usize;
    let in_sr = input_config.sample_rate.0;
    println!(
        "[input] 最終採用的 config: {in_channels} ch @ {in_sr} Hz \
         (buffer_size={:?})",
        input_config.buffer_size
    );
    println!("=== [audio_engine] 診斷結束 ===");

    // ── 建立輸出串流（播伴奏）──
    let pos = shared.playback_pos.clone();
    let running = shared.running.clone();
    let volume = shared.backing_volume.clone();
    let backing_rms = shared.backing_rms.clone();
    let loop_range_rec = shared.loop_range.clone();

    let backing_cb = backing.clone();
    let running_out = running.clone();

    // 🟢 Codex P1 #1：input / output callback 啟動時序對齊。
    //
    // CPAL 的 output stream 幾乎總是比 input stream 早幾個 buffer 週期開始
    // 產生 callback，這會讓 playback_pos 被推進數毫秒之後才輪到 input
    // 第一筆樣本塞進 vocal_buffer，造成「人聲比伴奏慢 X ms」的累積誤差。
    // 在續錄情境下尤其明顯（舊 N 秒對齊、新樣本卻從 N+δ 秒開始）。
    //
    // 解法：加一個 audio_ready AtomicBool。input 第一次收到樣本時設為 true，
    // output callback 在此之前輸出靜音且不推進 playback_pos。偏差收斂到
    // 最多一個 output buffer 週期（通常 ≤ 20ms），比原本 20-50ms 好很多。
    let audio_ready = Arc::new(AtomicBool::new(false));
    let audio_ready_out = audio_ready.clone();
    let audio_ready_in = audio_ready.clone();

    let out_err_fn = move |err| log::error!("Output stream error: {}", err);

    let output_stream = match output_device.build_output_stream(
        &output_config,
        move |data: &mut [f32], _info| {
            // 等 input callback 先就緒，避免 output 先推進 playback_pos 造成偏移
            if !audio_ready_out.load(Ordering::Acquire) {
                data.fill(0.0);
                return;
            }

            let cur_pos = pos.load(Ordering::Relaxed);
            let vol = f32::from_bits(volume.load(Ordering::Relaxed));

            // 🟡 Y4 修正：使用共用 helper 函式（錄音模式不混入人聲）
            let (mut new_pos, rms) = fill_backing_output(
                data,
                &backing_cb,
                backing_channels,
                out_channels,
                rate_ratio,
                total_frames,
                cur_pos,
                vol,
                None, // 錄音模式：不混入人聲
                0.0,
                source_sr,
            );

            // A-B 循環：到達 B 點時跳回 A 點（單次原子讀取）
            // 註：錄音模式下 R1 修正會強制停用循環，此段不會真正觸發
            if let Some((la, lb)) = unpack_loop(loop_range_rec.load(Ordering::Relaxed)) {
                if new_pos >= lb {
                    new_pos = la;
                }
            }

            pos.store(new_pos, Ordering::Relaxed);
            backing_rms.store(rms.to_bits(), Ordering::Relaxed);

            if new_pos >= total_frames {
                running_out.store(false, Ordering::Relaxed);
            }
        },
        out_err_fn,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            events::emit_error(&app, &format!("無法建立輸出串流：{}", e));
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    // ── 建立輸入串流（收麥克風）──
    let mic_gain = shared.mic_gain.clone();
    let mic_rms_atomic = shared.mic_rms.clone();
    let current_pitch_share = shared.current_pitch.clone();
    let vocal_buf = vocal_buffer.clone();
    let pitch_track_cb = pitch_track.clone();
    let pos_for_pitch = shared.playback_pos.clone();

    let in_err_fn = move |err| log::error!("Input stream error: {}", err);

    // ── 重採樣 state ─────────────────────────────────────────────
    //
    // 歷史 bug：之前的版本用純累積器 decimation 把 in_sr → source_sr，
    // 對 48k→44.1k 這類情境會產生嚴重 aliasing（高頻 fold-down 成金屬刺耳
    // 噪音）。Studio One 走 ASIO 原生 rate 正常，但我們走 CPAL/WASAPI 時
    // 若 in_sr != source_sr 就會踩雷。
    //
    // 修正：
    //   1. in_sr == source_sr → passthrough（最常見情境、零成本）
    //   2. in_sr > source_sr  → 先過 Butterworth LPF @ nyquist_target*0.9
    //                            再用線性插值 decimate
    //   3. in_sr < source_sr  → 線性插值 upsample（無 aliasing 風險）
    let needs_resample = in_sr != source_sr;
    let resample_mode_log = if !needs_resample {
        "passthrough".to_string()
    } else if in_sr > source_sr {
        format!("LPF + decimate {in_sr}→{source_sr}")
    } else {
        format!("linear upsample {in_sr}→{source_sr}")
    };
    log::info!("[run_recording] input resample mode: {}", resample_mode_log);

    // 若需要 down-sample，建立 anti-aliasing low-pass 濾波器
    // （cascade 2 級 biquad = 4 階 Butterworth）
    // 截止頻率 = target Nyquist × 0.9 留安全 margin
    let mut aa_stage1: Option<biquad::DirectForm1<f32>> = None;
    let mut aa_stage2: Option<biquad::DirectForm1<f32>> = None;
    if needs_resample && in_sr > source_sr {
        use biquad::{Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};
        let fs = (in_sr as f32).hz();
        let f_cut = ((source_sr as f32) * 0.5 * 0.9).hz();
        match Coefficients::<f32>::from_params(Type::LowPass, fs, f_cut, Q_BUTTERWORTH_F32) {
            Ok(coeffs) => {
                aa_stage1 = Some(DirectForm1::<f32>::new(coeffs));
                aa_stage2 = Some(DirectForm1::<f32>::new(coeffs));
                log::info!(
                    "[run_recording] anti-aliasing LPF @ {:.0} Hz \
                     (4-order Butterworth)",
                    (source_sr as f32) * 0.5 * 0.9
                );
            }
            Err(e) => {
                log::warn!(
                    "[run_recording] 無法建立 anti-aliasing LPF：{:?}，\
                     aliasing 風險仍在",
                    e
                );
            }
        }
    }

    // 線性插值重採樣 state：
    //   pending_sample 是上一個已 filter 的輸入 sample（視為 t=0）
    //   當前 sample 視為 t=1
    //   resample_phase 從 0 出發，每產生一個輸出 +step，直到 >= 1 才吃下一個輸入
    let mut has_pending = false;
    let mut pending_sample: f32 = 0.0;
    let mut resample_phase: f64 = 0.0;
    let step: f64 = if needs_resample {
        in_sr as f64 / source_sr as f64
    } else {
        1.0
    };

    // 🟢 Codex P2 #3：續錄邊界 fade-in。
    //
    // 續錄時 vocal_buffer 尾端的舊樣本與新收到的 mic 樣本在時域上可能不連續
    // （前次錄音結束那一瞬的樣本 vs 這次第一個 mic 樣本之間可能差任意相位），
    // 若振幅差大就會聽到 click/pop。對續錄前 ~5ms 套上 Hann 上半段 fade-in
    // 平滑銜接。第一次錄音（playback_pos == 0）不啟用。
    //
    // 5ms @ source_sr → 220 samples @44.1kHz / 240 @48kHz。
    let fade_total_samples: usize = if shared.playback_pos.load(Ordering::Relaxed) > 0 {
        ((source_sr as f64) * 0.005).round() as usize
    } else {
        0
    };
    let mut fade_remaining: usize = fade_total_samples;

    // ── CREPE 即時音高偵測 ─────────────────────────────────────────
    //
    // 取代 YIN PitchDetector：用 CREPE tiny ONNX 做即時推論。
    // 流程：native SR mono → StreamingResampler → 16kHz buffer → 每 1024 samples 呼叫一次 detect_realtime()
    //
    // Hop = 160 samples @16kHz = 10ms（與離線版相同），所以每次消耗 160 samples 後推論一次，
    // 不過即時模式為求簡潔，改用「累積滿 1024 就推論一次」的策略（= 64ms per frame）。
    let crepe_model_dir = crepe_engine::find_crepe_model_dir();
    let use_crepe = match pitch_engine_pref {
        "crepe" => crepe_model_dir.is_some(), // 指定 CREPE 但模型不存在仍 fallback
        "yin" => false,                       // 強制 YIN
        _ => crepe_model_dir.is_some(),       // "auto"：有模型就用 CREPE
    };
    let crepe_model_path = crepe_model_dir.unwrap_or_default();

    // 串流重採樣器：native SR → 16kHz
    let mut crepe_resampler = StreamingResampler::new(in_sr, crepe_engine::CREPE_SAMPLE_RATE);
    // 累積 @16kHz samples，每滿 1024 個就推論一次
    let mut crepe_buf: Vec<f32> = Vec::with_capacity(crepe_engine::CREPE_FRAME_SIZE * 2);
    let crepe_confidence_threshold = 0.3; // 即時模式用較寬鬆的門檻

    // YIN fallback：當 CREPE 模型不存在時仍可用
    let pitch_buf_size: usize = if in_sr > 48000 {
        if in_sr > 96000 {
            16384
        } else {
            8192
        }
    } else {
        PITCH_BUF_SIZE // 4096
    };
    let pitch_hop_size: usize = pitch_buf_size / 2;
    let mut pitch_buf: Vec<f32> = Vec::with_capacity(pitch_buf_size);
    let mut pitch_detector = PitchDetector::new(in_sr, pitch_buf_size, 50.0, 1000.0, 0.15, 0.01);

    // ── 即時音高平滑（指數移動平均）────────────────────────────────
    //
    // 避免即時曲線在 vibrato 或 noise 下跳動太劇烈。
    // alpha = 0.35 表示新偵測值佔 35%，舊值佔 65%。
    // 只在「前一幀也是 voiced」時啟用，避免靜音後第一個音被拉向舊頻率。
    let mut smooth_freq: f64 = 0.0;
    let mut smooth_active: bool = false;
    const SMOOTH_ALPHA: f64 = 0.35;

    if use_crepe {
        println!(
            "[run_recording] pitch detector: CREPE realtime, in_sr={in_sr} Hz → 16kHz, \
             frame_size={}, model={}",
            crepe_engine::CREPE_FRAME_SIZE,
            crepe_model_path.display()
        );
    } else {
        println!(
            "[run_recording] pitch detector: YIN fallback, in_sr={in_sr} Hz, \
             buf_size={pitch_buf_size}, hop_size={pitch_hop_size}"
        );
    }

    let input_stream = match input_device.build_input_stream(
        &input_config,
        move |data: &[f32], _info| {
            use biquad::Biquad;

            // 🟢 Codex P1 #1：首次拿到 mic 樣本時解鎖 output callback。
            // Release ordering 確保在此之前的所有 state 對 output 可見。
            audio_ready_in.store(true, Ordering::Release);

            let frame_count = data.len() / in_channels;
            let gain = f32::from_bits(mic_gain.load(Ordering::Relaxed));

            let mut sum_sq = 0.0_f32;
            let mut sample_count = 0_usize;
            let mut to_write: Vec<f32> = Vec::with_capacity(frame_count);
            // 收集原始 in_sr mono samples（給 pitch 分析用，不經 resample）
            let mut raw_mono_batch: Vec<f32> = Vec::with_capacity(frame_count);

            for frame in 0..frame_count {
                // Down-mix to mono
                let mut mono = 0.0_f32;
                for ch in 0..in_channels {
                    mono += data[frame * in_channels + ch];
                }
                mono = (mono / in_channels as f32) * gain;
                let mono = mono.clamp(-1.0, 1.0);

                sum_sq += mono * mono;
                sample_count += 1;
                raw_mono_batch.push(mono); // 原始 in_sr mono（給 pitch 分析用）

                // 先過 anti-aliasing LPF（若需要 down-sample 才建立了 filter）
                let filtered = match (aa_stage1.as_mut(), aa_stage2.as_mut()) {
                    (Some(s1), Some(s2)) => s2.run(s1.run(mono)),
                    _ => mono,
                };

                if !needs_resample {
                    // 同 rate：直接 passthrough，最常見情境零成本
                    to_write.push(filtered);
                    continue;
                }

                // 線性插值重採樣：
                //   pending 是 t=0 的舊輸入、filtered 是 t=1 的新輸入。
                //   phase ∈ [0, 1) 時在兩者之間插值產生輸出；phase 每次 +step。
                //   直到 phase >= 1 才更新 pending、吃下一個輸入。
                if !has_pending {
                    pending_sample = filtered;
                    has_pending = true;
                    resample_phase = 0.0;
                    continue;
                }

                while resample_phase < 1.0 {
                    let frac = resample_phase as f32;
                    let out = pending_sample + (filtered - pending_sample) * frac;
                    to_write.push(out);
                    resample_phase += step;
                }
                resample_phase -= 1.0;
                pending_sample = filtered;
            }

            // 🟢 Codex P2 #3：續錄邊界 Hann fade-in。
            //
            // fade_total_samples > 0 代表是續錄模式，對 vocal_buffer 新寫入的
            // 前 5ms 套上 Hann 上半段（0 → 1）增益曲線，平滑與前次錄音尾端
            // 的相位不連續，避免 click/pop。只作用於存進 vocal_buffer 的樣本，
            // 不影響 pitch 分析（pitch 看的是 raw_mono_batch，不 fade）。
            if fade_remaining > 0 && fade_total_samples > 0 {
                let total = fade_total_samples as f32;
                for s in to_write.iter_mut() {
                    if fade_remaining == 0 {
                        break;
                    }
                    let done = (fade_total_samples - fade_remaining) as f32;
                    let t = (done / total) * std::f32::consts::PI;
                    let gain = 0.5 * (1.0 - t.cos());
                    *s *= gain;
                    fade_remaining -= 1;
                }
            }

            // 寫入 vocal buffer
            if let Ok(mut v) = vocal_buf.lock() {
                v.extend_from_slice(&to_write);
            }

            // ── 音高分析 ──────────────────────────────────────────
            if use_crepe {
                // CREPE 即時模式：resample → 累積 → 每 1024 samples 推論
                let resampled = crepe_resampler.process(&raw_mono_batch);
                crepe_buf.extend_from_slice(&resampled);

                while crepe_buf.len() >= crepe_engine::CREPE_FRAME_SIZE {
                    let cur_pos = pos_for_pitch.load(Ordering::Relaxed);
                    let timestamp = cur_pos as f64 / source_sr as f64;

                    let frame = &crepe_buf[..crepe_engine::CREPE_FRAME_SIZE];
                    match crepe_engine::detect_realtime(
                        frame,
                        timestamp,
                        crepe_confidence_threshold,
                        &crepe_model_path,
                    ) {
                        Ok(Some(sample)) => {
                            // 指數平滑：降低即時顯示的跳動
                            if smooth_active {
                                smooth_freq =
                                    SMOOTH_ALPHA * sample.freq + (1.0 - SMOOTH_ALPHA) * smooth_freq;
                            } else {
                                smooth_freq = sample.freq;
                                smooth_active = true;
                            }

                            // pitch_track 保留原始值（離線分析用）
                            if let Ok(mut t) = pitch_track_cb.lock() {
                                t.append(sample.clone());
                            }

                            // current_pitch 用平滑值（即時顯示用）
                            let smoothed = PitchSample {
                                freq: smooth_freq,
                                ..sample
                            };
                            if let Ok(mut p) = current_pitch_share.lock() {
                                *p = Some(smoothed);
                            }
                        }
                        Ok(None) => {
                            smooth_active = false;
                            if let Ok(mut p) = current_pitch_share.lock() {
                                *p = None;
                            }
                        }
                        Err(e) => {
                            // CREPE 推論失��只 log，���中斷錄音
                            log::warn!("[recording] CREPE realtime error: {}", e);
                        }
                    }

                    // 滑動：hop = FRAME_SIZE/2 = 512 @16kHz = 32ms
                    crepe_buf.drain(..crepe_engine::CREPE_FRAME_SIZE / 2);
                }
            } else {
                // YIN fallback
                pitch_buf.extend_from_slice(&raw_mono_batch);
                while pitch_buf.len() >= pitch_buf_size {
                    let cur_pos = pos_for_pitch.load(Ordering::Relaxed);
                    let timestamp = cur_pos as f64 / source_sr as f64;

                    if let Some(sample) =
                        pitch_detector.detect(&pitch_buf[..pitch_buf_size], timestamp)
                    {
                        if let Ok(mut p) = current_pitch_share.lock() {
                            *p = Some(sample.clone());
                        }
                        if let Ok(mut t) = pitch_track_cb.lock() {
                            t.append(sample);
                        }
                    } else {
                        if let Ok(mut p) = current_pitch_share.lock() {
                            *p = None;
                        }
                    }

                    pitch_buf.drain(..pitch_hop_size);
                }
            }

            // 更新 mic RMS
            if sample_count > 0 {
                let rms = (sum_sq / sample_count as f32).sqrt();
                mic_rms_atomic.store(rms.to_bits(), Ordering::Relaxed);
            }
        },
        in_err_fn,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            events::emit_error(&app, &format!("無法建立輸入串流：{}", e));
            shared.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    if let Err(e) = output_stream.play() {
        events::emit_error(&app, &format!("無法啟動播放：{}", e));
        shared.running.store(false, Ordering::Relaxed);
        return;
    }
    if let Err(e) = input_stream.play() {
        events::emit_error(&app, &format!("無法啟動錄音：{}", e));
        shared.running.store(false, Ordering::Relaxed);
        return;
    }

    events::emit_state(&app, "recording");

    // Loop：emit 進度 + RMS + 音高
    let mut last_emitted_pitch_ts: f64 = -1.0;
    while running.load(Ordering::Relaxed) {
        let cur_pos = shared.playback_pos.load(Ordering::Relaxed);
        let elapsed = cur_pos as f64 / source_sr as f64;
        let b_rms = f32::from_bits(shared.backing_rms.load(Ordering::Relaxed));
        let m_rms = f32::from_bits(shared.mic_rms.load(Ordering::Relaxed));

        events::emit_progress(&app, elapsed, duration_s);
        events::emit_rms(&app, b_rms, m_rms);

        // 推送最新音高（避免重複推送同一個樣本）
        if let Ok(pitch_opt) = shared.current_pitch.lock() {
            match pitch_opt.as_ref() {
                Some(p) if p.timestamp != last_emitted_pitch_ts => {
                    last_emitted_pitch_ts = p.timestamp;
                    events::emit_pitch(
                        &app,
                        events::PitchPayload {
                            freq: p.freq,
                            note: p.note.clone(),
                            octave: p.octave,
                            cent: p.cent,
                            confidence: p.confidence,
                        },
                    );
                }
                None => {
                    if last_emitted_pitch_ts >= 0.0 {
                        last_emitted_pitch_ts = -1.0;
                        events::emit_pitch_none(&app);
                    }
                }
                _ => {}
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    drop(input_stream);
    drop(output_stream);
    events::emit_state(&app, "idle");
    events::emit_finished(&app);
}

// ── CPAL 設定輔助 ─────────────────────────────────────────────────

/// 嘗試找到匹配 source_sr 的輸出設定，否則用 device 預設
fn build_output_config(
    device: &cpal::Device,
    preferred_sr: u32,
) -> Result<cpal::StreamConfig, String> {
    let supported: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| e.to_string())?
        .collect();

    // 優先：F32 + channels >= 2 且支援 preferred_sr
    let mut target = None;
    for c in &supported {
        if c.sample_format() == cpal::SampleFormat::F32
            && c.channels() >= 2
            && c.min_sample_rate().0 <= preferred_sr
            && c.max_sample_rate().0 >= preferred_sr
        {
            target = Some(c.clone());
            break;
        }
    }

    // 次選：任何 F32 config（SR 不完全匹配也行）
    if target.is_none() {
        for c in &supported {
            if c.sample_format() == cpal::SampleFormat::F32 && c.channels() >= 2 {
                target = Some(c.clone());
                break;
            }
        }
    }

    // 再次選：channels >= 2 不限 format（回退到 device 預設 SR）
    if target.is_none() {
        for c in &supported {
            if c.channels() >= 2
                && c.min_sample_rate().0 <= preferred_sr
                && c.max_sample_rate().0 >= preferred_sr
            {
                target = Some(c.clone());
                break;
            }
        }
    }

    if let Some(range) = target {
        let sr = if range.min_sample_rate().0 <= preferred_sr
            && range.max_sample_rate().0 >= preferred_sr
        {
            preferred_sr
        } else {
            range.max_sample_rate().0
        };
        let cfg = range.with_sample_rate(cpal::SampleRate(sr)).config();
        if range.sample_format() != cpal::SampleFormat::F32 {
            log::warn!(
                "[audio] 輸出裝置不支援 F32，使用 {:?}（callback 仍假定 f32，可能需要轉換）",
                range.sample_format()
            );
        }
        return Ok(cfg);
    }

    // Fallback：device 預設
    let default = device.default_output_config().map_err(|e| e.to_string())?;
    if default.sample_format() != cpal::SampleFormat::F32 {
        log::warn!(
            "[audio] 輸出裝置預設格式為 {:?}，非 F32",
            default.sample_format()
        );
    }
    Ok(default.config())
}

/// 嘗試找到匹配 source_sr 的輸入設定，否則用 device 預設
fn build_input_config(
    device: &cpal::Device,
    preferred_sr: u32,
) -> Result<cpal::StreamConfig, String> {
    let supported: Vec<_> = device
        .supported_input_configs()
        .map_err(|e| e.to_string())?
        .collect();

    // 優先：F32 + 支援 preferred_sr
    let mut target = None;
    for c in &supported {
        if c.sample_format() == cpal::SampleFormat::F32
            && c.min_sample_rate().0 <= preferred_sr
            && c.max_sample_rate().0 >= preferred_sr
        {
            target = Some(c.clone());
            break;
        }
    }

    // 次選：任何支援 preferred_sr 的 config
    if target.is_none() {
        for c in &supported {
            if c.min_sample_rate().0 <= preferred_sr && c.max_sample_rate().0 >= preferred_sr {
                target = Some(c.clone());
                break;
            }
        }
    }

    if let Some(range) = target {
        let cfg = range
            .with_sample_rate(cpal::SampleRate(preferred_sr))
            .config();
        if range.sample_format() != cpal::SampleFormat::F32 {
            log::warn!(
                "[audio] 輸入裝置不支援 F32，使用 {:?}",
                range.sample_format()
            );
        }
        return Ok(cfg);
    }

    let default = device.default_input_config().map_err(|e| e.to_string())?;
    if default.sample_format() != cpal::SampleFormat::F32 {
        log::warn!(
            "[audio] 輸入裝置預設格式為 {:?}，非 F32",
            default.sample_format()
        );
    }
    Ok(default.config())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
        (sum_sq / samples.len() as f32).sqrt()
    }

    /// 產生指定頻率的正弦波（1 秒長度）
    fn sine(freq: f32, sample_rate: u32) -> Vec<f32> {
        let n = sample_rate as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn sample_cubic_interleaved_hits_exact_sample_on_integer_position() {
        let frames = vec![0.1_f32, 0.3, 0.5, 0.7, 0.9];
        let out = sample_cubic_interleaved(&frames, 1, 3.0, 0);
        assert!((out - 0.7).abs() < 1e-6, "got {out}");
    }

    #[test]
    fn sample_linear_mono_blends_fractional_position() {
        let frames = vec![0.0_f32, 1.0];
        let out = sample_linear_mono(&frames, 0.25);
        assert!((out - 0.25).abs() < 1e-6, "got {out}");
    }

    #[test]
    fn lanczos3_kernel_is_one_at_origin_and_zero_at_integers() {
        // sinc(0) = 1
        assert!((lanczos3_kernel(0.0) - 1.0).abs() < 1e-9);
        // sinc(k) = 0 for non-zero integer k within support
        for k in [1.0_f64, -1.0, 2.0, -2.0] {
            assert!(
                lanczos3_kernel(k).abs() < 1e-9,
                "L({k}) should be 0, got {}",
                lanczos3_kernel(k)
            );
        }
        // out of support → 0
        assert!(lanczos3_kernel(3.0).abs() < 1e-9);
        assert!(lanczos3_kernel(-3.5).abs() < 1e-9);
    }

    #[test]
    fn sample_lanczos3_interleaved_hits_exact_sample_on_integer_position() {
        let frames = vec![0.1_f32, 0.3, 0.5, 0.7, 0.9];
        let out = sample_lanczos3_interleaved(&frames, 1, 3.0, 0);
        assert!(
            (out - 0.7).abs() < 1e-5,
            "整數位置應直接命中樣本 0.7，實際 {out}"
        );
    }

    #[test]
    fn sample_lanczos3_interleaved_preserves_linear_ramp() {
        // 線性斜率訊號在 Lanczos-3 插值下應保留線性（DC + 低頻訊號無失真）
        let frames: Vec<f32> = (0..16).map(|i| i as f32 * 0.1).collect();
        let out = sample_lanczos3_interleaved(&frames, 1, 7.5, 0);
        // 位置 7.5 對應 0.75 附近（f(i) = 0.1i）
        assert!(
            (out - 0.75).abs() < 0.01,
            "線性斜坡 7.5 應約 0.75，實際 {out}"
        );
    }

    #[test]
    fn sample_lanczos3_interleaved_handles_stereo_channels() {
        // 交錯 stereo：[L0, R0, L1, R1, ...]
        let frames: Vec<f32> = (0..10)
            .flat_map(|i| [i as f32 * 0.1, i as f32 * 0.2])
            .collect();
        let left = sample_lanczos3_interleaved(&frames, 2, 4.0, 0);
        let right = sample_lanczos3_interleaved(&frames, 2, 4.0, 1);
        assert!((left - 0.4).abs() < 1e-5, "stereo L@4 應 0.4，實際 {left}");
        assert!(
            (right - 0.8).abs() < 1e-5,
            "stereo R@4 應 0.8，實際 {right}"
        );
    }

    #[test]
    fn highpass_attenuates_40hz_sine_heavily() {
        let sr = 44100;
        let input = sine(40.0, sr);
        let output = apply_highpass_80hz(&input, sr);

        // 跳過前 2000 樣本避免 filter transient 干擾量測
        let rms_in = rms(&input[2000..]);
        let rms_out = rms(&output[2000..]);
        let ratio = rms_out / rms_in;

        assert!(
            ratio < 0.3,
            "40Hz 應被衰減至原能量 < 30%（實際 {:.3}）",
            ratio
        );
    }

    #[test]
    fn highpass_preserves_440hz_sine() {
        let sr = 44100;
        let input = sine(440.0, sr);
        let output = apply_highpass_80hz(&input, sr);

        // 440Hz 遠高於 80Hz 截止點，應幾乎無衰減
        let rms_in = rms(&input[2000..]);
        let rms_out = rms(&output[2000..]);
        let ratio = rms_out / rms_in;

        assert!(
            ratio > 0.9 && ratio < 1.1,
            "440Hz 應幾乎無衰減（實際 {:.3}）",
            ratio
        );
    }

    // ── 校準輔助函式測試 ─────────────────────────────────────────

    #[test]
    fn compute_median_handles_odd_count() {
        let v = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert!((compute_median(&v) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn compute_median_handles_even_count() {
        let v = vec![10.0, 20.0, 30.0, 40.0];
        assert!((compute_median(&v) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn compute_median_handles_unsorted_input() {
        let v = vec![50.0, 10.0, 40.0, 20.0, 30.0];
        assert!((compute_median(&v) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn compute_median_handles_empty() {
        let v: Vec<f64> = vec![];
        assert_eq!(compute_median(&v), 0.0);
    }

    #[test]
    fn compute_rms_zero_for_silence() {
        let v = vec![0.0_f32; 1000];
        assert_eq!(compute_rms(&v), 0.0);
    }

    #[test]
    fn compute_rms_correct_for_sine() {
        let sr = 44100;
        let s = sine(440.0, sr);
        let r = compute_rms(&s);
        // 純 sine 波 RMS = amplitude / sqrt(2) ≈ 0.707
        assert!(
            (r - 0.7071).abs() < 0.02,
            "440Hz sine RMS 應約 0.707（實際 {:.4}）",
            r
        );
    }

    #[test]
    fn woodblock_click_has_short_duration_and_decay() {
        let sr = 44100;
        let click = generate_woodblock_click(sr);
        // 約 40ms 長度
        let expected_len = (sr as f32 * 0.040) as usize;
        assert_eq!(click.len(), expected_len);
        // 第一個樣本應該是非零（attack 即時）
        // 注意 sin(0) = 0，所以第二個樣本才會看到聲音
        assert!(click[10].abs() > 0.0);
        // 後段應顯著衰減（< 前段的 30%）
        let early_rms = rms(&click[..100]);
        let late_rms = rms(&click[click.len() - 100..]);
        assert!(
            late_rms < early_rms * 0.3,
            "尾段能量應 < 早段 30%（早 {:.3} / 晚 {:.3}）",
            early_rms,
            late_rms
        );
    }

    #[test]
    fn detect_onset_finds_synthetic_click_position() {
        let sr = 44100;
        // 1 秒長靜音 + 一個 click 在 500ms 處
        let mut buf = vec![0.0_f32; sr as usize];
        let click = generate_woodblock_click(sr);
        let click_pos = sr as usize / 2; // 500ms
        for (i, s) in click.iter().enumerate() {
            buf[click_pos + i] = *s;
        }

        let detected = detect_onset_in_window(&buf, sr, 0.0005).expect("應該偵測到 click");

        // 容忍 ±10ms 的偏差（onset detection 不是 sample-accurate）
        let tolerance = (sr as f32 * 0.010) as i64;
        let diff = (detected as i64 - click_pos as i64).abs();
        assert!(
            diff < tolerance,
            "偵測位置應接近 click 起點，偏差 {} samples (tolerance {})",
            diff,
            tolerance
        );
    }

    #[test]
    fn detect_onset_returns_none_for_silence() {
        let sr = 44100;
        let buf = vec![0.0_f32; sr as usize];
        let result = detect_onset_in_window(&buf, sr, 0.0005);
        assert!(result.is_none(), "純靜音不應偵測到 onset");
    }

    // ── pack_loop / unpack_loop ──

    #[test]
    fn pack_unpack_roundtrip() {
        let packed = pack_loop(1000, 5000);
        let (a, b) = unpack_loop(packed).expect("should unpack");
        assert_eq!(a, 1000);
        assert_eq!(b, 5000);
    }

    #[test]
    fn pack_unpack_zero() {
        let packed = pack_loop(0, 0);
        let (a, b) = unpack_loop(packed).expect("should unpack");
        assert_eq!(a, 0);
        assert_eq!(b, 0);
    }

    #[test]
    fn pack_unpack_max_u32() {
        // u32::MAX-1 因為全 1 = DISABLED
        let packed = pack_loop(u32::MAX - 1, u32::MAX - 1);
        let (a, b) = unpack_loop(packed).expect("should unpack");
        assert_eq!(a, u32::MAX as u64 - 1);
        assert_eq!(b, u32::MAX as u64 - 1);
    }

    #[test]
    fn unpack_disabled_returns_none() {
        assert!(unpack_loop(LOOP_PACKED_DISABLED).is_none());
    }

    // ── compute_rms 邊界 ──

    #[test]
    fn compute_rms_single_sample() {
        let v = vec![0.5_f32];
        assert!((compute_rms(&v) - 0.5).abs() < 1e-6);
    }

    // ── generate_woodblock_click ──

    #[test]
    fn woodblock_click_different_sample_rates() {
        for sr in [22050, 44100, 48000] {
            let click = generate_woodblock_click(sr);
            let expected_len = (sr as f32 * 0.040) as usize;
            assert_eq!(click.len(), expected_len, "sr={}", sr);
            // 不應有 NaN 或 Inf
            assert!(
                click.iter().all(|s| s.is_finite()),
                "sr={} contains non-finite",
                sr
            );
        }
    }
}
