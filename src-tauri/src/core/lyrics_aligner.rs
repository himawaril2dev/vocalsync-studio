//! Energy-based lyrics-to-vocal alignment for first-pass subtitle timing.

use crate::core::lyrics_parser::LyricLine;
use crate::core::media_loader;
use crate::error::AppError;
use serde::Serialize;

const WINDOW_MS: u64 = 50;
const HOP_MS: u64 = 20;
const MERGE_GAP_MS: u64 = 340;
const MIN_SEGMENT_MS: u64 = 160;
const SEGMENT_PAD_MS: u64 = 80;

#[derive(Debug, Clone, Copy)]
struct Segment {
    start_ms: u64,
    end_ms: u64,
}

impl Segment {
    fn duration_ms(self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

#[derive(Debug, Serialize)]
pub struct LyricsAlignmentResult {
    pub lines: Vec<LyricLine>,
    pub detected_segments: usize,
    pub audio_duration_secs: f64,
    pub active_duration_secs: f64,
    pub confidence: String,
}

pub fn align_lyrics_to_audio(
    audio_path: &str,
    lines: &[LyricLine],
) -> Result<LyricsAlignmentResult, AppError> {
    if lines.is_empty() {
        return Err(AppError::Audio("沒有可對齊的歌詞行".to_string()));
    }

    let media = media_loader::load_media(audio_path)?;
    let channels = media.channels.max(1) as usize;
    let mono = downmix_interleaved_to_mono(&media.samples, channels);
    if mono.is_empty() {
        return Err(AppError::Audio("音檔沒有可分析的聲音".to_string()));
    }

    let duration_ms = (media.duration * 1000.0).round().max(1.0) as u64;
    let segments = detect_vocal_segments(&mono, media.sample_rate, duration_ms);
    let active_duration_ms = segments
        .iter()
        .map(|segment| segment.duration_ms())
        .sum::<u64>();

    let ranges = if segments.is_empty() {
        distribute_over_range(lines, 0, duration_ms)
    } else if segments.len() >= lines.len() {
        distribute_over_segments(lines, &segments)
    } else {
        let start = segments
            .first()
            .map(|segment| segment.start_ms)
            .unwrap_or(0);
        let end = segments
            .last()
            .map(|segment| segment.end_ms)
            .unwrap_or(duration_ms);
        distribute_over_range(lines, start, end.max(start + 1))
    };

    let mut aligned = Vec::with_capacity(lines.len());
    for (line, (start_ms, end_ms)) in lines.iter().zip(ranges) {
        let mut line = line.clone();
        line.start_ms = start_ms.min(duration_ms);
        line.end_ms = end_ms.max(start_ms + 1).min(duration_ms.max(start_ms + 1));
        aligned.push(line);
    }

    Ok(LyricsAlignmentResult {
        lines: aligned,
        detected_segments: segments.len(),
        audio_duration_secs: media.duration,
        active_duration_secs: active_duration_ms as f64 / 1000.0,
        confidence: alignment_confidence(
            segments.len(),
            lines.len(),
            active_duration_ms,
            duration_ms,
        )
        .to_string(),
    })
}

pub fn detect_first_vocal_onset_ms(audio_path: &str) -> Result<Option<u64>, AppError> {
    let media = media_loader::load_media(audio_path)?;
    let channels = media.channels.max(1) as usize;
    let mono = downmix_interleaved_to_mono(&media.samples, channels);
    if mono.is_empty() {
        return Ok(None);
    }

    let duration_ms = (media.duration * 1000.0).round().max(1.0) as u64;
    Ok(detect_vocal_segments(&mono, media.sample_rate, duration_ms)
        .first()
        .map(|segment| segment.start_ms))
}

fn downmix_interleaved_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn detect_vocal_segments(mono: &[f32], sample_rate: u32, duration_ms: u64) -> Vec<Segment> {
    let window = ((sample_rate as u64 * WINDOW_MS) / 1000).max(1) as usize;
    let hop = ((sample_rate as u64 * HOP_MS) / 1000).max(1) as usize;
    if mono.len() < window {
        return Vec::new();
    }

    let mut rms_values = Vec::new();
    let mut start = 0usize;
    while start + window <= mono.len() {
        let frame = &mono[start..start + window];
        let rms =
            (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len() as f32).sqrt();
        rms_values.push(rms);
        start += hop;
    }

    if rms_values.is_empty() {
        return Vec::new();
    }

    let rms_values = smooth(&rms_values, 2);
    let floor = percentile(&rms_values, 0.20);
    let high = percentile(&rms_values, 0.90);
    if high <= 0.000_01 {
        return Vec::new();
    }

    let dynamic = floor + (high - floor) * 0.28;
    let threshold = dynamic.max(high * 0.08).max(0.000_5).min(high * 0.75);

    let mut raw_segments = Vec::new();
    let mut active_start: Option<u64> = None;
    for (idx, rms) in rms_values.iter().enumerate() {
        let frame_start = idx as u64 * HOP_MS;
        let frame_end = (frame_start + WINDOW_MS).min(duration_ms);
        if *rms >= threshold {
            if active_start.is_none() {
                active_start = Some(frame_start);
            }
        } else if let Some(seg_start) = active_start.take() {
            raw_segments.push(Segment {
                start_ms: seg_start,
                end_ms: frame_end,
            });
        }
    }
    if let Some(seg_start) = active_start {
        raw_segments.push(Segment {
            start_ms: seg_start,
            end_ms: duration_ms,
        });
    }

    merge_segments(raw_segments, duration_ms)
}

fn smooth(values: &[f32], radius: usize) -> Vec<f32> {
    values
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            let start = idx.saturating_sub(radius);
            let end = (idx + radius + 1).min(values.len());
            values[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}

fn percentile(values: &[f32], p: f32) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let idx = ((sorted.len().saturating_sub(1)) as f32 * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn merge_segments(raw_segments: Vec<Segment>, duration_ms: u64) -> Vec<Segment> {
    let mut merged: Vec<Segment> = Vec::new();
    for segment in raw_segments {
        if segment.duration_ms() < MIN_SEGMENT_MS {
            continue;
        }
        let padded = Segment {
            start_ms: segment.start_ms.saturating_sub(SEGMENT_PAD_MS),
            end_ms: (segment.end_ms + SEGMENT_PAD_MS).min(duration_ms),
        };

        if let Some(last) = merged.last_mut() {
            if padded.start_ms <= last.end_ms + MERGE_GAP_MS {
                last.end_ms = last.end_ms.max(padded.end_ms);
                continue;
            }
        }
        merged.push(padded);
    }
    merged
}

fn distribute_over_segments(lines: &[LyricLine], segments: &[Segment]) -> Vec<(u64, u64)> {
    let weights = line_weights(lines);
    let total_weight = weights.iter().sum::<f64>().max(1.0);
    let total_active = segments
        .iter()
        .map(|segment| segment.duration_ms().max(1))
        .sum::<u64>()
        .max(1);

    let mut cumulative_segment_duration = Vec::with_capacity(segments.len());
    let mut acc = 0u64;
    for segment in segments {
        acc += segment.duration_ms().max(1);
        cumulative_segment_duration.push(acc);
    }

    let mut ranges = Vec::with_capacity(lines.len());
    let mut segment_start_idx = 0usize;
    let mut cumulative_weight = 0.0f64;

    for (line_idx, weight) in weights.iter().enumerate() {
        if line_idx == lines.len() - 1 {
            let start = segments[segment_start_idx].start_ms;
            let end = segments.last().unwrap().end_ms;
            ranges.push((start, end));
            break;
        }

        cumulative_weight += weight;
        let desired_active_ms = (cumulative_weight / total_weight * total_active as f64) as u64;
        let boundary_idx = cumulative_segment_duration
            .iter()
            .position(|cum| *cum >= desired_active_ms)
            .unwrap_or(segments.len() - 1);
        let remaining_lines = lines.len() - line_idx - 1;
        let max_end_idx = segments.len().saturating_sub(remaining_lines + 1);
        let end_idx = boundary_idx.clamp(segment_start_idx, max_end_idx);

        ranges.push((
            segments[segment_start_idx].start_ms,
            segments[end_idx].end_ms,
        ));
        segment_start_idx = (end_idx + 1).min(segments.len() - 1);
    }

    ranges
}

fn distribute_over_range(lines: &[LyricLine], start_ms: u64, end_ms: u64) -> Vec<(u64, u64)> {
    let weights = line_weights(lines);
    let total_weight = weights.iter().sum::<f64>().max(1.0);
    let duration = end_ms.saturating_sub(start_ms).max(lines.len() as u64);

    let mut ranges = Vec::with_capacity(lines.len());
    let mut current = start_ms;
    let mut cumulative_weight = 0.0f64;
    for (idx, weight) in weights.iter().enumerate() {
        let next = if idx == lines.len() - 1 {
            end_ms
        } else {
            cumulative_weight += weight;
            start_ms + (cumulative_weight / total_weight * duration as f64).round() as u64
        };
        ranges.push((current, next.max(current + 1)));
        current = next.max(current + 1);
    }
    ranges
}

fn line_weights(lines: &[LyricLine]) -> Vec<f64> {
    lines
        .iter()
        .map(|line| {
            let text_len = visible_char_count(&line.text) as f64;
            let translation_len = line
                .translation
                .as_deref()
                .map(visible_char_count)
                .unwrap_or(0) as f64;
            (text_len + translation_len * 0.55).max(4.0)
        })
        .collect()
}

fn visible_char_count(text: &str) -> usize {
    text.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn alignment_confidence(
    segment_count: usize,
    line_count: usize,
    active_duration_ms: u64,
    duration_ms: u64,
) -> &'static str {
    if segment_count == 0 || line_count == 0 {
        return "low";
    }
    let ratio = segment_count as f64 / line_count as f64;
    let active_ratio = active_duration_ms as f64 / duration_ms.max(1) as f64;
    if (0.75..=1.8).contains(&ratio) && active_ratio > 0.08 {
        "high"
    } else if (0.35..=3.0).contains(&ratio) && active_ratio > 0.04 {
        "medium"
    } else {
        "low"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> LyricLine {
        LyricLine {
            start_ms: 0,
            end_ms: 0,
            text: text.to_string(),
            translation: None,
        }
    }

    #[test]
    fn detects_energy_segments() {
        let sr = 1000;
        let mut mono = vec![0.0_f32; 5000];
        for sample in &mut mono[1000..1800] {
            *sample = 0.3;
        }
        for sample in &mut mono[2600..3400] {
            *sample = 0.25;
        }
        let segments = detect_vocal_segments(&mono, sr, 5000);
        assert_eq!(segments.len(), 2);
        assert!(segments[0].start_ms <= 1000);
        assert!(segments[1].end_ms >= 3400);
    }

    #[test]
    fn distributes_lines_over_segments() {
        let lines = vec![line("短句"), line("這是一句比較長的歌詞")];
        let segments = vec![
            Segment {
                start_ms: 1000,
                end_ms: 1800,
            },
            Segment {
                start_ms: 2200,
                end_ms: 3200,
            },
        ];
        let ranges = distribute_over_segments(&lines, &segments);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].0, 1000);
        assert_eq!(ranges[1].1, 3200);
    }

    #[test]
    fn distributes_more_lines_than_segments_over_active_range() {
        let lines = vec![line("a"), line("b"), line("c")];
        let ranges = distribute_over_range(&lines, 1000, 4000);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0].0, 1000);
        assert_eq!(ranges[2].1, 4000);
        assert!(ranges.windows(2).all(|pair| pair[0].1 <= pair[1].0));
    }
}
