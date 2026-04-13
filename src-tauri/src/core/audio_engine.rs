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
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::AppHandle;

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
    /// 自動偵測到的目標旋律來源標籤，例如 "ultrastar" / "midi" / "uvr_cache"，
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
    use biquad::{
        Biquad, Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32,
    };

    let fs = (sample_rate as f32).hz();
    let f0 = 80.0_f32.hz();

    // 常數參數，理論上不會失敗；真失敗代表 crate bug，直接 panic 也無妨
    let coeffs = Coefficients::<f32>::from_params(Type::HighPass, fs, f0, Q_BUTTERWORTH_F32)
        .expect("biquad 80Hz HPF coefficients");

    // 兩級 cascade → 4 階 Butterworth，讓 60-80Hz 過渡帶更陡
    let mut stage1 = DirectForm1::<f32>::new(coeffs);
    let mut stage2 = DirectForm1::<f32>::new(coeffs);

    mono.iter()
        .map(|&s| stage2.run(stage1.run(s)))
        .collect()
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
        self.backing_pitch_track
            .lock()
            .ok()
            .and_then(|t| t.clone())
    }

    // ── 載入 ───────────────────────────────────────────────────────

    /// 載入伴奏檔（支援 WAV / MP3 / MP4 / FLAC / OGG）
    pub fn load_backing(&mut self, path: &str) -> Result<LoadResult, AppError> {
        // 載入前先停止任何進行中的播放/錄音
        self.stop();

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
            video_path: if is_video { Some(path.to_string()) } else { None },
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
            let mut detector = PitchDetector::new(
                sample_rate,
                BACKING_BUF_SIZE,
                60.0,
                1200.0,
                0.20,
                0.01,
            );

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
        self.shared
            .mic_gain
            .store(mic.to_bits(), Ordering::Relaxed);
    }

    pub fn seek(&mut self, seconds: f64) {
        let frame = (seconds.max(0.0) * self.sample_rate as f64) as u64;
        self.shared.playback_pos.store(frame, Ordering::Relaxed);
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

    pub fn start_playback(&mut self, app: AppHandle, start_frame: Option<u64>, output_device: Option<usize>, latency_ms: f64) -> Result<(), AppError> {
        if self.state != EngineState::Idle {
            return Err(AppError::Audio("引擎正忙，請先停止".to_string()));
        }
        let backing = self
            .backing_data
            .clone()
            .ok_or_else(|| AppError::Audio("尚未載入伴奏".to_string()))?;

        self.spawn_playback_worker(app, backing, start_frame, true, output_device, latency_ms, None, "auto");
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

    pub fn start_recording(&mut self, app: AppHandle, input_device: Option<usize>, output_device: Option<usize>, pitch_engine_pref: &str) -> Result<(), AppError> {
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

        // 清空舊錄音資料 + 音高軌跡
        if let Ok(mut v) = vocal_buffer.lock() {
            v.clear();
        }
        if let Ok(mut t) = pitch_track.lock() {
            t.clear();
        }
        if let Ok(mut p) = shared.current_pitch.lock() {
            *p = None;
        }

        shared.playback_pos.store(0, Ordering::Relaxed);
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

    pub fn export(&self, dir: &str, prefix: &str, auto_balance: bool, latency_ms: f64) -> Result<ExportPaths, AppError> {
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
            
            // 目標：人聲約等於伴奏的 0.9 倍 (Lead vocal slightly below backing total energy dynamically)
            if v_rms > 0.001 {
                vocal_gain = (b_rms * 0.9) / v_rms;
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
            if l.abs() > max_peak { max_peak = l.abs(); }
            if r.abs() > max_peak { max_peak = r.abs(); }
            
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

        let accepted_offsets: Vec<f64> =
            beat_results.iter().filter(|r| r.accepted).map(|r| r.offset_ms).collect();

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

        let mean_ms: f64 =
            accepted_offsets.iter().sum::<f64>() / accepted_offsets.len() as f64;

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
        .and_then(|idx| host.output_devices().ok().and_then(|mut devs| devs.nth(idx)))
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
        vocal_buffer
            .lock()
            .ok()
            .map(|v| Arc::new(v.clone()))
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

    // WSOLA 處理器（由 closure 擁有，跨 callback 持續存活）
    let mut wsola = crate::core::wsola::WsolaProcessor::new(backing_channels);
    let mut wsola_buf: Vec<f32> = Vec::new();

    // Pitch-shift anti-aliasing LPF state（升調時需要）
    // 每聲道 3 級 biquad cascade = 6 階 Butterworth（抑制力約 -36 dB/oct）
    let mut pitch_aa_filters: Vec<[Option<biquad::DirectForm1<f32>>; 3]> =
        (0..backing_channels).map(|_| [None, None, None]).collect();
    let mut pitch_aa_last_st: i32 = 0; // 追蹤上次的 pitch_st 以避免重複建 filter

    let err_fn = move |err| log::error!("Output stream error: {}", err);

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _info| {
            let cur_pos = pos.load(Ordering::Relaxed);
            let vol = f32::from_bits(volume.load(Ordering::Relaxed));
            let cur_speed = f32::from_bits(speed_atomic.load(Ordering::Relaxed)) as f64;
            let cur_pitch_st = pitch_atomic.load(Ordering::Relaxed) as i32;
            let pitch_ratio = 2.0_f64.powf(cur_pitch_st as f64 / 12.0);

            let needs_wsola = (cur_speed - 1.0).abs() > 0.01 || cur_pitch_st != 0;

            if !needs_wsola {
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
            } else {
                // ── WSOLA 路徑：變速不變調 / 移調 ──
                let frame_count = data.len() / out_channels;
                let stretch = pitch_ratio / cur_speed;

                // 合併 resample ratio（pitch 補償 + SR 轉換）
                let combined_resample = pitch_ratio * rate_ratio;

                // WSOLA 需要產生的幀數（resample 前）
                // +8 為 Lanczos-4 插值核的額外邊距（需要前後各 4 幀）
                let wsola_frames_needed =
                    (frame_count as f64 * combined_resample).ceil() as usize + 8;

                // 同步 WSOLA 位置（僅在外部跳躍時——seek / loop）
                // 正常播放時不覆寫，保留 wsola 內部的 f64 小數精度
                let wsola_pos_int = wsola.input_pos() as u64;
                if cur_pos != wsola_pos_int {
                    wsola.set_input_pos(cur_pos as f64);
                }

                // 產生 WSOLA 輸出
                let wsola_samples = wsola_frames_needed * backing_channels;
                wsola_buf.resize(wsola_samples, 0.0);
                wsola.process(&backing_cb, &mut wsola_buf[..wsola_samples], stretch);

                // ── Anti-aliasing LPF（升調時需要） ──
                // 升調 = decimation，需要先濾掉 Nyquist/total_decimation 以上的頻率
                // total_decimation = pitch_ratio × rate_ratio（含 SR 轉換）
                if cur_pitch_st > 0 && cur_pitch_st != pitch_aa_last_st {
                    use biquad::{
                        Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32,
                    };
                    let fs = (source_sr as f32).hz();
                    // 截止頻率 = Nyquist / (pitch_ratio × rate_ratio) × 0.80
                    // 比之前更保守的 margin（0.80 vs 0.85），搭配 6 階 LPF
                    let total_decimation = pitch_ratio as f32 * rate_ratio as f32;
                    let f_cut =
                        ((source_sr as f32) * 0.5 / total_decimation * 0.80).hz();
                    if let Ok(coeffs) = Coefficients::<f32>::from_params(
                        Type::LowPass, fs, f_cut, Q_BUTTERWORTH_F32,
                    ) {
                        for ch_filters in pitch_aa_filters.iter_mut() {
                            ch_filters[0] = Some(DirectForm1::<f32>::new(coeffs));
                            ch_filters[1] = Some(DirectForm1::<f32>::new(coeffs));
                            ch_filters[2] = Some(DirectForm1::<f32>::new(coeffs));
                        }
                    }
                    pitch_aa_last_st = cur_pitch_st;
                } else if cur_pitch_st <= 0 && pitch_aa_last_st > 0 {
                    // 降調或不移調時不需要 AA filter
                    for ch_filters in pitch_aa_filters.iter_mut() {
                        ch_filters[0] = None;
                        ch_filters[1] = None;
                        ch_filters[2] = None;
                    }
                    pitch_aa_last_st = cur_pitch_st;
                }

                // 對 WSOLA 輸出施加 anti-aliasing LPF（in-place，6 階 cascade）
                if cur_pitch_st > 0 {
                    use biquad::Biquad;
                    let wsola_frame_count = wsola_buf.len() / backing_channels;
                    for f in 0..wsola_frame_count {
                        for src_ch in 0..backing_channels {
                            let idx = f * backing_channels + src_ch;
                            let mut s = wsola_buf[idx];
                            for stage in 0..3 {
                                if let Some(ref mut filt) = pitch_aa_filters[src_ch][stage] {
                                    s = filt.run(s);
                                }
                            }
                            wsola_buf[idx] = s;
                        }
                    }
                }

                // Resample + 寫入 output（含音量、channel mapping）
                // 使用 Catmull-Rom cubic 插值（4-tap），
                // 比線性插值品質好很多，且沒有 sinc overshoot 和 sin() 效能問題
                let mut sum_sq = 0.0_f32;
                let mut sample_count = 0_usize;

                for frame in 0..frame_count {
                    let src_f = frame as f64 * combined_resample;
                    let src_lo = src_f.floor() as usize;
                    let t = (src_f - src_lo as f64) as f32;

                    // 邊界檢查：Catmull-Rom 需要 src_lo-1 到 src_lo+2
                    if src_lo + 2 >= wsola_frames_needed {
                        for ch in 0..out_channels {
                            data[frame * out_channels + ch] = 0.0;
                        }
                        continue;
                    }

                    for ch in 0..out_channels {
                        let src_ch = ch.min(backing_channels - 1);

                        // 4 個取樣點：p0(前一個)、p1(當前)、p2(下一個)、p3(再下一個)
                        let i1 = src_lo * backing_channels + src_ch;
                        let p0 = if src_lo > 0 {
                            wsola_buf[i1 - backing_channels]
                        } else {
                            wsola_buf[i1] // 邊界 clamp
                        };
                        let p1 = wsola_buf[i1];
                        let p2 = wsola_buf[i1 + backing_channels];
                        let p3 = wsola_buf[i1 + 2 * backing_channels];

                        // Catmull-Rom spline（只用乘法和加法，零 sin() 呼叫）
                        let t2 = t * t;
                        let t3 = t2 * t;
                        let s = 0.5
                            * ((2.0 * p1)
                                + (-p0 + p2) * t
                                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);

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
        build_preview_pitch_stream(
            &host,
            in_idx,
            source_sr,
            &shared,
            pitch_engine_pref,
        )
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
    let state_name = if mix_vocal { "playing_back" } else { "previewing" };
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

    drop(_input_stream);
    drop(output_stream);
    events::emit_state(&app, "idle");
    events::emit_finished(&app);
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
        if in_sr > 96000 { 16384 } else { 8192 }
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
        log::info!(
            "[preview_pitch] YIN fallback, in_sr={in_sr} Hz, buf_size={pitch_buf_size}"
        );
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
        .and_then(|idx| host.output_devices().ok().and_then(|mut devs| devs.nth(idx)))
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

    let out_err_fn = move |err| log::error!("Output stream error: {}", err);

    let output_stream = match output_device.build_output_stream(
        &output_config,
        move |data: &mut [f32], _info| {
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
    log::info!(
        "[run_recording] input resample mode: {}",
        resample_mode_log
    );

    // 若需要 down-sample，建立 anti-aliasing low-pass 濾波器
    // （cascade 2 級 biquad = 4 階 Butterworth）
    // 截止頻率 = target Nyquist × 0.9 留安全 margin
    let mut aa_stage1: Option<biquad::DirectForm1<f32>> = None;
    let mut aa_stage2: Option<biquad::DirectForm1<f32>> = None;
    if needs_resample && in_sr > source_sr {
        use biquad::{
            Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32,
        };
        let fs = (in_sr as f32).hz();
        let f_cut = ((source_sr as f32) * 0.5 * 0.9).hz();
        match Coefficients::<f32>::from_params(
            Type::LowPass,
            fs,
            f_cut,
            Q_BUTTERWORTH_F32,
        ) {
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
        if in_sr > 96000 { 16384 } else { 8192 }
    } else {
        PITCH_BUF_SIZE // 4096
    };
    let pitch_hop_size: usize = pitch_buf_size / 2;
    let mut pitch_buf: Vec<f32> = Vec::with_capacity(pitch_buf_size);
    let mut pitch_detector = PitchDetector::new(
        in_sr,
        pitch_buf_size,
        50.0,
        1000.0,
        0.15,
        0.01,
    );

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
                                smooth_freq = SMOOTH_ALPHA * sample.freq
                                    + (1.0 - SMOOTH_ALPHA) * smooth_freq;
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
    let default = device
        .default_output_config()
        .map_err(|e| e.to_string())?;
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

    let default = device
        .default_input_config()
        .map_err(|e| e.to_string())?;
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

        let detected = detect_onset_in_window(&buf, sr, 0.0005)
            .expect("應該偵測到 click");

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
            assert!(click.iter().all(|s| s.is_finite()), "sr={} contains non-finite", sr);
        }
    }
}
