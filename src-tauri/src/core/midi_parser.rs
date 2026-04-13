//! 解析 MIDI 檔案為 MelodyTrack（Phase 2 功能）
//!
//! # 實作細節
//! - 讀取標準 MIDI 檔 (`.mid`, `.midi`)
//! - 支援 Tempo Map：自動收集跨軌道的 SetTempo 事件，精確計算每個音符的絕對秒數。
//! - 軌道選擇：如果有多個軌道，自動選擇包含最多 NoteOn 事件的軌道作為主旋律，忽略其他伴奏軌。

use crate::core::melody_track::{MelodyNote, MelodySource, MelodyTrack};
use crate::error::AppError;
use midly::{MetaMessage, Smf, Timing, TrackEventKind};
use std::fs;
use std::path::Path;

/// Tempo 變更歷史記錄（用於將 Ticks 轉換為秒數）
#[derive(Debug, Clone)]
struct TempoChange {
    tick: u64,
    /// 該區段起點的絕對時間（秒）
    start_time_secs: f64,
    /// 該區段的每個 Tick 對應多少秒
    secs_per_tick: f64,
}

/// 將 .mid 檔案讀取為 MelodyTrack
pub fn load_midi(path: &str) -> Result<MelodyTrack, AppError> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(AppError::Audio(format!("MIDI 檔案不存在: {}", path)));
    }

    let bytes = fs::read(p).map_err(|e| AppError::Audio(format!("無法讀取 MIDI: {}", e)))?;
    let smf = Smf::parse(&bytes).map_err(|e| AppError::Audio(format!("MIDI 解析失敗: {}", e)))?;

    // 1. 取得 PPQ (Pulses Per Quarter Note)
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as f64,
        Timing::Timecode(fps, subframe) => {
            // 少見的影片 Timecode 模式，作 fallback 處理
            let f = fps.as_f32() as f64;
            f * subframe as f64
        }
    };

    if ppq == 0.0 {
        return Err(AppError::Audio("MIDI PPQ 為 0，無法解析時間軸".to_string()));
    }

    // 2. 收集所有軌道的 Tempo 變更
    let mut raw_tempos: Vec<(u64, u32)> = Vec::new();
    for track in &smf.tracks {
        let mut abs_tick = 0;
        for event in track {
            abs_tick += event.delta.as_int() as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(t)) = event.kind {
                raw_tempos.push((abs_tick, t.as_int()));
            }
        }
    }
    
    // 依時間排序並去重
    raw_tempos.sort_by_key(|&(tick, _)| tick);
    
    // 3. 建立連續的 Tempo Map
    let mut tempo_map: Vec<TempoChange> = Vec::new();
    let mut current_secs = 0.0;
    let mut last_tick = 0;
    // 預設 120 BPM = 每四分音符 500,000 微秒
    let mut current_us_per_quarter: u32 = 500_000;

    // 起點
    tempo_map.push(TempoChange {
        tick: 0,
        start_time_secs: 0.0,
        secs_per_tick: (current_us_per_quarter as f64 / 1_000_000.0) / ppq,
    });

    for (tick, us_per_quarter) in raw_tempos {
        if tick > last_tick {
            let delta_ticks = tick - last_tick;
            let secs_per_tick = (current_us_per_quarter as f64 / 1_000_000.0) / ppq;
            current_secs += delta_ticks as f64 * secs_per_tick;
            last_tick = tick;
        }

        current_us_per_quarter = us_per_quarter;
        tempo_map.push(TempoChange {
            tick,
            start_time_secs: current_secs,
            secs_per_tick: (current_us_per_quarter as f64 / 1_000_000.0) / ppq,
        });
    }

    // 將 tick 轉換成秒數的閉包
    let tick_to_secs = |tick: u64| -> f64 {
        // 找到最適合的（晚於該 tick）前一個 tempo change
        let mut active_change = &tempo_map[0];
        for change in &tempo_map {
            if change.tick <= tick {
                active_change = change;
            } else {
                break;
            }
        }
        let delta_ticks = tick - active_change.tick;
        active_change.start_time_secs + (delta_ticks as f64 * active_change.secs_per_tick)
    };

    // 4. 解析所有軌道的音符，尋找音符最多的那一軌
    let mut best_track_notes: Vec<MelodyNote> = Vec::new();
    let mut best_track_index = 0;
    let mut best_track_name: Option<String> = None;
    let mut max_note_count = 0;

    for (i, track) in smf.tracks.iter().enumerate() {
        let mut current_track_notes: Vec<MelodyNote> = Vec::new();
        let mut curr_track_name: Option<String> = None;
        let mut abs_tick = 0;
        
        // 簡單陣列記錄 128 個 MIDI pitch 目前開啟的 start_tick
        // MIDI 允許同音重複開啟，但此處簡化處理，同一個 key 一次只有一個 active
        let mut active_notes: [Option<u64>; 128] = [None; 128];

        for event in track {
            abs_tick += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(name_bytes)) => {
                    curr_track_name = Some(String::from_utf8_lossy(name_bytes).to_string());
                }
                TrackEventKind::Midi { channel: _, message } => match message {
                    midly::MidiMessage::NoteOn { key, vel } => {
                        let pitch = key.as_int();
                        if vel.as_int() > 0 {
                            active_notes[pitch as usize] = Some(abs_tick);
                        } else {
                            // Velocity 0 視為 NoteOff
                            if let Some(start_tick) = active_notes[pitch as usize].take() {
                                push_note_from_ticks(
                                    &mut current_track_notes,
                                    start_tick,
                                    abs_tick,
                                    pitch,
                                    &tick_to_secs,
                                );
                            }
                        }
                    }
                    midly::MidiMessage::NoteOff { key, vel: _ } => {
                        let pitch = key.as_int();
                        if let Some(start_tick) = active_notes[pitch as usize].take() {
                            push_note_from_ticks(
                                &mut current_track_notes,
                                start_tick,
                                abs_tick,
                                pitch,
                                &tick_to_secs,
                            );
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if current_track_notes.len() > max_note_count {
            max_note_count = current_track_notes.len();
            best_track_notes = current_track_notes;
            best_track_index = i;
            best_track_name = curr_track_name;
        }
    }

    if best_track_notes.is_empty() {
        return Err(AppError::Audio("MIDI 檔案中未發現任何音符".to_string()));
    }

    // 依起始時間排序（因為可能有多音符重疊或未按時間順序）
    best_track_notes.sort_by(|a, b| {
        a.start_secs
            .partial_cmp(&b.start_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total_duration_secs = best_track_notes.last().map_or(0.0, |n| n.end_secs());

    Ok(MelodyTrack {
        source: MelodySource::Midi {
            mid_path: path.to_string(),
            track_index: best_track_index,
            track_name: best_track_name,
        },
        notes: best_track_notes,
        total_duration_secs,
        raw_pitch_track: None,
    })
}

fn push_note_from_ticks<F>(
    notes: &mut Vec<MelodyNote>,
    start_tick: u64,
    end_tick: u64,
    pitch: u8,
    tick_to_secs: &F,
) where
    F: Fn(u64) -> f64,
{
    // 過濾極短的神經質音符（例如零長度）
    if end_tick <= start_tick {
        return;
    }

    let start_secs = tick_to_secs(start_tick);
    let end_secs = tick_to_secs(end_tick);
    let duration_secs = end_secs - start_secs;

    if duration_secs < 0.05 {
        return; // 小於 50ms 視為 ghost note 丟棄
    }

    notes.push(MelodyNote::from_midi(
        start_secs,
        duration_secs,
        pitch,
        None,
        false,
        false,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_map_converts_correctly() {
        // Mock PPQ = 480
        // 預設 120BPM => 500,000 us/q => 每 quarter note 0.5 秒。
        // 480 ticks = 0.5s => 1 tick = 0.5 / 480 = 0.0010416666... 秒
        let mut map: Vec<TempoChange> = Vec::new();
        map.push(TempoChange {
            tick: 0,
            start_time_secs: 0.0,
            secs_per_tick: (500_000.0 / 1_000_000.0) / 480.0,
        });
        
        // 960 tick 後變更為 60 BPM = 1,000,000 us/q => 每 quarter note 1.0 秒
        // 960 tick = 2 quarter notes = 1.0s
        map.push(TempoChange {
            tick: 960,
            start_time_secs: 1.0,
            secs_per_tick: (1_000_000.0 / 1_000_000.0) / 480.0,
        });

        let tick_to_secs = |tick: u64| -> f64 {
            let mut active_change = &map[0];
            for change in &map {
                if change.tick <= tick {
                    active_change = change;
                } else {
                    break;
                }
            }
            let delta_ticks = tick - active_change.tick;
            active_change.start_time_secs + (delta_ticks as f64 * active_change.secs_per_tick)
        };

        assert!((tick_to_secs(0) - 0.0).abs() < 1e-6);
        assert!((tick_to_secs(480) - 0.5).abs() < 1e-6);
        assert!((tick_to_secs(960) - 1.0).abs() < 1e-6);
        
        // 此後每個 quarter note = 1.0s。 1440 = 960 + 480 => 1.0 + 1.0 = 2.0s
        assert!((tick_to_secs(1440) - 2.0).abs() < 1e-6);
    }
}
