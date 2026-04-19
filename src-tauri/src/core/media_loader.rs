//! 媒體載入器：使用 symphonia 解碼 MP3/MP4/FLAC/OGG/WAV
//!
//! 統一輸出格式：交錯立體聲 f32 樣本（與 audio_engine 的 backing_data 相容）
//! 對 MP4 影片：只解碼音訊軌，影片路徑由前端 <video> 標籤處理

use crate::error::AppError;
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct LoadedMedia {
    /// 交錯立體聲樣本（[L, R, L, R, ...]）
    pub samples: Vec<f32>,
    /// 取樣率
    pub sample_rate: u32,
    /// 聲道數（固定為 2，已 down-mix）
    pub channels: u16,
    /// 總時長（秒）
    pub duration: f64,
}

/// 載入任意支援格式（MP3 / MP4 / FLAC / OGG / WAV）的音訊
pub fn load_media(path: &str) -> Result<LoadedMedia, AppError> {
    let path_obj = Path::new(path);

    let file = File::open(path_obj).map_err(|e| AppError::Audio(format!("無法開啟檔案：{}", e)))?;

    // Hint：讓 symphonia 用副檔名加速 probe
    let mut hint = Hint::new();
    if let Some(ext) = path_obj.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AppError::Audio(format!("無法辨識格式：{}", e)))?;

    let mut format = probed.format;

    // 找第一條音訊軌
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AppError::Audio("找不到音訊軌".to_string()))?;

    let track_id = track.id;
    // sample_rate / channels 從 codec_params 取，若 None 則延後到第一個 packet 解碼後再取
    let mut sample_rate: u32 = track.codec_params.sample_rate.unwrap_or(0);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AppError::Audio(format!("無法建立解碼器：{}", e)))?;

    // 解碼所有 packet → 累積到 stereo interleaved buffer
    let mut samples: Vec<f32> = Vec::new();
    let mut detected_channels: usize = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                continue;
            }
            Err(e) => {
                return Err(AppError::Audio(format!("讀取 packet 失敗：{}", e)));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue, // 略過壞 packet
            Err(e) => {
                return Err(AppError::Audio(format!("解碼失敗：{}", e)));
            }
        };

        // 從第一個成功解碼的 packet 取得 spec 資訊
        let spec = decoded.spec();
        if sample_rate == 0 {
            sample_rate = spec.rate;
        }
        if detected_channels == 0 {
            detected_channels = spec.channels.count();
            log::info!(
                "Detected audio spec from first packet: {}ch, {}Hz",
                detected_channels,
                spec.rate
            );
        }

        append_stereo_interleaved(&decoded, detected_channels, &mut samples);
    }

    if samples.is_empty() {
        return Err(AppError::Audio("解碼後無資料".to_string()));
    }

    if sample_rate == 0 {
        return Err(AppError::Audio("無法判斷取樣率".to_string()));
    }

    let total_frames = samples.len() / 2;
    let duration = total_frames as f64 / sample_rate as f64;

    log::info!(
        "Decoded media: {} samples ({}ch source → 2ch stereo), {} Hz, {:.1}s",
        samples.len(),
        detected_channels,
        sample_rate,
        duration
    );

    Ok(LoadedMedia {
        samples,
        sample_rate,
        channels: 2,
        duration,
    })
}

/// 將 symphonia 的 AudioBufferRef 轉為交錯立體聲 f32 樣本，append 到 out
fn append_stereo_interleaved(buf: &AudioBufferRef, src_channels: usize, out: &mut Vec<f32>) {
    macro_rules! handle_buf {
        ($buf:expr, $convert:expr) => {{
            let frames = $buf.frames();
            let ch_count = $buf.spec().channels.count();
            for f in 0..frames {
                let l_raw = $buf.chan(0)[f];
                let l = $convert(l_raw);
                let r = if ch_count >= 2 {
                    $convert($buf.chan(1)[f])
                } else {
                    l
                };
                out.push(l);
                out.push(r);
            }
            let _ = src_channels; // 使用一次以避免 unused warning
        }};
    }

    match buf {
        AudioBufferRef::F32(b) => handle_buf!(b, |s: f32| s),
        AudioBufferRef::F64(b) => handle_buf!(b, |s: f64| s as f32),
        AudioBufferRef::S8(b) => handle_buf!(b, |s: i8| s as f32 / 128.0),
        AudioBufferRef::S16(b) => handle_buf!(b, |s: i16| s as f32 / 32768.0),
        AudioBufferRef::S24(b) => handle_buf!(b, |s: symphonia::core::sample::i24| {
            s.inner() as f32 / 8_388_608.0
        }),
        AudioBufferRef::S32(b) => {
            handle_buf!(b, |s: i32| s as f32 / 2_147_483_648.0)
        }
        AudioBufferRef::U8(b) => handle_buf!(b, |s: u8| (s as f32 - 128.0) / 128.0),
        AudioBufferRef::U16(b) => {
            handle_buf!(b, |s: u16| (s as f32 - 32768.0) / 32768.0)
        }
        AudioBufferRef::U24(b) => handle_buf!(b, |s: symphonia::core::sample::u24| {
            (s.inner() as f32 - 8_388_608.0) / 8_388_608.0
        }),
        AudioBufferRef::U32(b) => handle_buf!(b, |s: u32| {
            (s as f64 - 2_147_483_648.0) as f32 / 2_147_483_648.0
        }),
    }
}
