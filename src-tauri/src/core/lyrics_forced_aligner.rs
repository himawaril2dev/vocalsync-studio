//! Text-guided lyrics alignment from timed speech-recognition transcripts.

use crate::core::lyrics_parser::{self, LyricLine};
use crate::error::AppError;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

const MAX_START_LOOKAHEAD: usize = 10;
const MAX_SEGMENT_SPAN: usize = 72;
const MATCH_THRESHOLD: f64 = 0.34;

#[derive(Debug, Clone, Serialize)]
pub struct TimedTranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LyricsTranscriptAlignmentResult {
    pub lines: Vec<LyricLine>,
    pub transcript_segments: usize,
    pub matched_lines: usize,
    pub average_score: f64,
    pub confidence: String,
    pub detected_intro_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct MatchCandidate {
    start_idx: usize,
    end_idx: usize,
    score: f64,
}

pub fn align_lyrics_to_timed_transcript(
    transcript_path: &str,
    lines: &[LyricLine],
) -> Result<LyricsTranscriptAlignmentResult, AppError> {
    if lines.is_empty() {
        return Err(AppError::Audio("No lyrics lines to align".into()));
    }

    let segments = load_timed_transcript(transcript_path)?;
    align_lyrics_to_segments(lines, &segments)
}

pub fn align_lyrics_to_timed_transcript_with_intro(
    transcript_path: &str,
    lines: &[LyricLine],
    first_vocal_onset_ms: Option<u64>,
) -> Result<LyricsTranscriptAlignmentResult, AppError> {
    if lines.is_empty() {
        return Err(AppError::Audio("No lyrics lines to align".into()));
    }

    let segments = load_timed_transcript(transcript_path)?;
    align_lyrics_to_segments_with_intro(lines, &segments, first_vocal_onset_ms)
}

pub fn load_timed_transcript(path: &str) -> Result<Vec<TimedTranscriptSegment>, AppError> {
    let bytes = fs::read(path)
        .map_err(|err| AppError::Audio(format!("Failed to read transcript: {}", err)))?;
    let content = decode_text(&bytes);
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut segments = match ext.as_str() {
        "json" => parse_whisper_json(&content)?,
        "srt" => lyrics_parser::parse_srt_text(&content)
            .into_iter()
            .map(segment_from_lyric)
            .collect(),
        "vtt" => lyrics_parser::parse_vtt_text(&content)
            .into_iter()
            .map(segment_from_lyric)
            .collect(),
        "lrc" => lyrics_parser::parse_lrc_text(&content)
            .into_iter()
            .map(segment_from_lyric)
            .collect(),
        _ => {
            return Err(AppError::Audio(
                "Timed transcript must be JSON, SRT, VTT, or LRC".into(),
            ))
        }
    };

    segments.retain(|segment| {
        segment.end_ms > segment.start_ms && !normalize_text(&segment.text).is_empty()
    });
    segments.sort_by_key(|segment| segment.start_ms);

    if segments.is_empty() {
        return Err(AppError::Audio(
            "Timed transcript did not contain usable timed text".into(),
        ));
    }

    Ok(segments)
}

pub fn align_lyrics_to_segments(
    lines: &[LyricLine],
    segments: &[TimedTranscriptSegment],
) -> Result<LyricsTranscriptAlignmentResult, AppError> {
    align_lyrics_to_segments_with_intro(lines, segments, None)
}

pub fn align_lyrics_to_segments_with_intro(
    lines: &[LyricLine],
    segments: &[TimedTranscriptSegment],
    first_vocal_onset_ms: Option<u64>,
) -> Result<LyricsTranscriptAlignmentResult, AppError> {
    if segments.is_empty() {
        return Err(AppError::Audio("No transcript segments to align".into()));
    }

    let normalized_segments: Vec<String> = segments
        .iter()
        .map(|segment| normalize_text(&segment.text))
        .collect();
    let mut matches: Vec<Option<MatchCandidate>> = vec![None; lines.len()];
    let mut cursor = 0usize;
    let mut matched_lines = 0usize;
    let mut score_sum = 0.0;

    for (line_idx, line) in lines.iter().enumerate() {
        let line_text = lyric_match_text(line);
        let normalized_line = normalize_text(&line_text);
        if normalized_line.is_empty() {
            continue;
        }

        if let Some(candidate) = best_candidate(&normalized_line, &normalized_segments, cursor) {
            if candidate.score >= MATCH_THRESHOLD {
                matches[line_idx] = Some(candidate);
                cursor = (candidate.end_idx + 1).min(segments.len());
                matched_lines += 1;
                score_sum += candidate.score;
            }
        }
    }

    let mut aligned = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let mut line = line.clone();
        if let Some(candidate) = matches[idx] {
            line.start_ms = segments[candidate.start_idx].start_ms;
            line.end_ms = segments[candidate.end_idx].end_ms.max(line.start_ms + 1);
        } else {
            line.start_ms = 0;
            line.end_ms = 0;
        }
        aligned.push(line);
    }

    fill_unmatched_ranges(&mut aligned, &matches, segments);
    let detected_intro_ms = apply_first_vocal_onset(&mut aligned, first_vocal_onset_ms);

    let average_score = if matched_lines == 0 {
        0.0
    } else {
        score_sum / matched_lines as f64
    };
    let confidence = transcript_alignment_confidence(matched_lines, lines.len(), average_score);

    Ok(LyricsTranscriptAlignmentResult {
        lines: aligned,
        transcript_segments: segments.len(),
        matched_lines,
        average_score,
        confidence: confidence.into(),
        detected_intro_ms,
    })
}

fn apply_first_vocal_onset(lines: &mut [LyricLine], onset_ms: Option<u64>) -> Option<u64> {
    let onset_ms = onset_ms?;
    if lines.is_empty() {
        return None;
    }
    let next_start_ms = lines.get(1).map(|line| line.start_ms);
    let first = lines.first_mut()?;
    if onset_ms <= first.start_ms + 250 {
        return Some(onset_ms);
    }

    first.start_ms = onset_ms;
    if first.end_ms <= first.start_ms {
        first.end_ms = next_start_ms
            .filter(|next_start| *next_start > first.start_ms)
            .unwrap_or(first.start_ms + 1000);
    }
    Some(onset_ms)
}

fn decode_text(bytes: &[u8]) -> String {
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    String::from_utf8_lossy(bytes).to_string()
}

fn segment_from_lyric(line: LyricLine) -> TimedTranscriptSegment {
    TimedTranscriptSegment {
        start_ms: line.start_ms,
        end_ms: line.end_ms,
        text: line.text,
    }
}

fn parse_whisper_json(content: &str) -> Result<Vec<TimedTranscriptSegment>, AppError> {
    let value: Value = serde_json::from_str(content)
        .map_err(|err| AppError::Audio(format!("Failed to parse transcript JSON: {}", err)))?;

    let mut segments = Vec::new();
    collect_json_segments(&value, &mut segments);
    Ok(segments)
}

fn collect_json_segments(value: &Value, out: &mut Vec<TimedTranscriptSegment>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_segment_or_children(item, out);
            }
        }
        Value::Object(map) => {
            for key in [
                "segments",
                "word_segments",
                "words",
                "transcription",
                "transcript",
            ] {
                if let Some(child) = map.get(key) {
                    let before = out.len();
                    collect_json_segments(child, out);
                    if out.len() > before {
                        return;
                    }
                }
            }
            if out.is_empty() {
                collect_segment_or_children(value, out);
            }
        }
        _ => {}
    }
}

fn collect_segment_or_children(value: &Value, out: &mut Vec<TimedTranscriptSegment>) {
    if let Some(words) = value.get("words").and_then(Value::as_array) {
        let before = out.len();
        for word in words {
            if let Some(segment) = parse_json_segment(word) {
                out.push(segment);
            }
        }
        if out.len() > before {
            return;
        }
    }

    if let Some(segment) = parse_json_segment(value) {
        out.push(segment);
        return;
    }

    if let Value::Object(map) = value {
        for key in ["segments", "word_segments", "transcription", "transcript"] {
            if let Some(child) = map.get(key) {
                collect_json_segments(child, out);
            }
        }
    }
}

fn parse_json_segment(value: &Value) -> Option<TimedTranscriptSegment> {
    let text = value
        .get("text")
        .or_else(|| value.get("word"))
        .or_else(|| value.get("sentence"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }

    let start_ms = json_time_ms(value, &["start_ms", "offset_start_ms", "from_ms"])
        .or_else(|| json_time_secs(value, &["start", "start_time", "from"]))
        .or_else(|| json_nested_time(value, "offsets", "from", true))
        .or_else(|| json_nested_time(value, "timestamps", "from", false))?;

    let end_ms = json_time_ms(value, &["end_ms", "offset_end_ms", "to_ms"])
        .or_else(|| json_time_secs(value, &["end", "end_time", "to"]))
        .or_else(|| json_nested_time(value, "offsets", "to", true))
        .or_else(|| json_nested_time(value, "timestamps", "to", false))?;

    if end_ms <= start_ms {
        return None;
    }

    Some(TimedTranscriptSegment {
        start_ms,
        end_ms,
        text,
    })
}

fn json_time_ms(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(item) = value.get(*key) {
            if let Some(ms) = json_value_to_ms(item, true) {
                return Some(ms);
            }
        }
    }
    None
}

fn json_time_secs(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(item) = value.get(*key) {
            if let Some(ms) = json_value_to_ms(item, false) {
                return Some(ms);
            }
        }
    }
    None
}

fn json_nested_time(value: &Value, object_key: &str, key: &str, number_is_ms: bool) -> Option<u64> {
    value
        .get(object_key)
        .and_then(|nested| nested.get(key))
        .and_then(|item| json_value_to_ms(item, number_is_ms))
}

fn json_value_to_ms(value: &Value, number_is_ms: bool) -> Option<u64> {
    if let Some(num) = value.as_f64() {
        if !num.is_finite() || num < 0.0 {
            return None;
        }
        return Some(if number_is_ms {
            num.round() as u64
        } else {
            (num * 1000.0).round() as u64
        });
    }
    value.as_str().and_then(parse_time_string_ms)
}

fn parse_time_string_ms(value: &str) -> Option<u64> {
    let value = value.trim().replace(',', ".");
    if value.is_empty() {
        return None;
    }

    if !value.contains(':') {
        let seconds = value.parse::<f64>().ok()?;
        return Some((seconds * 1000.0).round().max(0.0) as u64);
    }

    let parts: Vec<&str> = value.split(':').collect();
    let seconds = match parts.len() {
        2 => parts[0].parse::<f64>().ok()? * 60.0 + parts[1].parse::<f64>().ok()?,
        3 => {
            parts[0].parse::<f64>().ok()? * 3600.0
                + parts[1].parse::<f64>().ok()? * 60.0
                + parts[2].parse::<f64>().ok()?
        }
        _ => return None,
    };
    Some((seconds * 1000.0).round().max(0.0) as u64)
}

fn lyric_match_text(line: &LyricLine) -> String {
    if let Some(translation) = &line.translation {
        format!("{} {}", line.text, translation)
    } else {
        line.text.clone()
    }
}

fn best_candidate(
    line_norm: &str,
    segment_norms: &[String],
    cursor: usize,
) -> Option<MatchCandidate> {
    if cursor >= segment_norms.len() {
        return None;
    }

    let mut best: Option<MatchCandidate> = None;
    let max_start = (cursor + MAX_START_LOOKAHEAD).min(segment_norms.len());
    for start_idx in cursor..max_start {
        let mut acc = String::new();
        let max_end = (start_idx + MAX_SEGMENT_SPAN).min(segment_norms.len());
        for (end_idx, segment_norm) in segment_norms
            .iter()
            .enumerate()
            .take(max_end)
            .skip(start_idx)
        {
            acc.push_str(segment_norm);
            if acc.is_empty() {
                continue;
            }

            let raw_score = text_similarity(line_norm, &acc);
            let skip_penalty = (start_idx.saturating_sub(cursor) as f64 * 0.025).min(0.18);
            let score = (raw_score - skip_penalty).max(0.0);

            if best
                .map(|candidate| score > candidate.score)
                .unwrap_or(true)
            {
                best = Some(MatchCandidate {
                    start_idx,
                    end_idx,
                    score,
                });
            }

            if acc.chars().count() > line_norm.chars().count() * 3 + 24 && raw_score < 0.45 {
                break;
            }
        }
    }

    best
}

fn text_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    if a == b {
        return 1.0;
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a.contains(b) {
        return (b_chars.len() as f64 / a_chars.len() as f64).max(0.72);
    }
    if b.contains(a) {
        return (a_chars.len() as f64 / b_chars.len() as f64).max(0.82);
    }

    let lcs = lcs_len(&a_chars, &b_chars) as f64;
    let coverage = lcs / a_chars.len().max(1) as f64;
    let precision = lcs / b_chars.len().max(1) as f64;
    let dice = (2.0 * lcs) / (a_chars.len() + b_chars.len()).max(1) as f64;
    dice.max(coverage * 0.68 + precision * 0.32)
}

fn lcs_len(a: &[char], b: &[char]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut curr = vec![0usize; b.len() + 1];
    for a_ch in a {
        for (j, b_ch) in b.iter().enumerate() {
            curr[j + 1] = if a_ch == b_ch {
                prev[j] + 1
            } else {
                prev[j + 1].max(curr[j])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    prev[b.len()]
}

fn fill_unmatched_ranges(
    lines: &mut [LyricLine],
    matches: &[Option<MatchCandidate>],
    segments: &[TimedTranscriptSegment],
) {
    let transcript_start = segments
        .first()
        .map(|segment| segment.start_ms)
        .unwrap_or(0);
    let transcript_end = segments
        .last()
        .map(|segment| segment.end_ms)
        .unwrap_or(transcript_start + lines.len() as u64 * 2000);

    let mut idx = 0usize;
    while idx < lines.len() {
        if matches[idx].is_some() {
            idx += 1;
            continue;
        }

        let block_start = idx;
        while idx < lines.len() && matches[idx].is_none() {
            idx += 1;
        }
        let block_end = idx;

        let range_start = if block_start > 0 {
            lines[block_start - 1].end_ms
        } else {
            transcript_start
        };
        let range_end = if block_end < lines.len() {
            lines[block_end].start_ms
        } else {
            transcript_end
        };
        distribute_block(&mut lines[block_start..block_end], range_start, range_end);
    }
}

fn distribute_block(lines: &mut [LyricLine], start_ms: u64, end_ms: u64) {
    if lines.is_empty() {
        return;
    }
    let line_count = lines.len();
    let end_ms = end_ms.max(start_ms + line_count as u64);
    let weights = line_weights(lines);
    let total = weights.iter().sum::<f64>().max(1.0);
    let duration = end_ms.saturating_sub(start_ms).max(line_count as u64);
    let mut current = start_ms;
    let mut cumulative = 0.0;

    for (idx, line) in lines.iter_mut().enumerate() {
        let next = if idx == line_count - 1 {
            end_ms
        } else {
            cumulative += weights[idx];
            start_ms + (cumulative / total * duration as f64).round() as u64
        };
        line.start_ms = current;
        line.end_ms = next.max(current + 1);
        current = line.end_ms;
    }
}

fn line_weights(lines: &[LyricLine]) -> Vec<f64> {
    lines
        .iter()
        .map(|line| {
            normalize_text(&lyric_match_text(line))
                .chars()
                .count()
                .max(4) as f64
        })
        .collect()
}

fn transcript_alignment_confidence(
    matched_lines: usize,
    line_count: usize,
    average_score: f64,
) -> &'static str {
    if line_count == 0 {
        return "low";
    }
    let coverage = matched_lines as f64 / line_count as f64;
    if coverage >= 0.72 && average_score >= 0.58 {
        "high"
    } else if coverage >= 0.42 && average_score >= 0.42 {
        "medium"
    } else {
        "low"
    }
}

fn normalize_text(text: &str) -> String {
    let mut normalized = String::new();
    for ch in text.chars() {
        if let Some(ascii) = fullwidth_ascii_to_ascii(ch) {
            normalized.push(ascii.to_ascii_lowercase());
        } else if ch.is_alphanumeric() || is_cjk(ch) {
            for lower in ch.to_lowercase() {
                normalized.push(lower);
            }
        }
    }
    normalized
}

fn fullwidth_ascii_to_ascii(ch: char) -> Option<char> {
    let code = ch as u32;
    if (0xFF01..=0xFF5E).contains(&code) {
        char::from_u32(code - 0xFEE0)
    } else {
        None
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> LyricLine {
        LyricLine {
            start_ms: 0,
            end_ms: 0,
            text: text.into(),
            translation: None,
        }
    }

    fn segment(start_ms: u64, end_ms: u64, text: &str) -> TimedTranscriptSegment {
        TimedTranscriptSegment {
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    #[test]
    fn parses_openai_whisper_segments() {
        let json = r#"{
          "segments": [
            {"start": 1.25, "end": 2.5, "text": "hello world"},
            {"start": 2.5, "end": 4.0, "text": "next line"}
          ]
        }"#;

        let parsed = parse_whisper_json(json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].start_ms, 1250);
        assert_eq!(parsed[1].end_ms, 4000);
    }

    #[test]
    fn parses_whisper_cpp_transcription_offsets() {
        let json = r#"{
          "transcription": [
            {"offsets": {"from": 100, "to": 900}, "text": "春眠"},
            {"offsets": {"from": 900, "to": 1800}, "text": "不覺曉"}
          ]
        }"#;

        let parsed = parse_whisper_json(json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].start_ms, 100);
        assert_eq!(parsed[1].end_ms, 1800);
    }

    #[test]
    fn aligns_lyrics_to_timed_transcript_text() {
        let lines = vec![line("hello world"), line("this is the chorus")];
        let segments = vec![
            segment(1000, 1800, "hello"),
            segment(1800, 2600, "world"),
            segment(3000, 3600, "this is"),
            segment(3600, 4600, "the chorus"),
        ];

        let aligned = align_lyrics_to_segments(&lines, &segments).unwrap();
        assert_eq!(aligned.matched_lines, 2);
        assert_eq!(aligned.lines[0].start_ms, 1000);
        assert_eq!(aligned.lines[0].end_ms, 2600);
        assert_eq!(aligned.lines[1].start_ms, 3000);
        assert_eq!(aligned.lines[1].end_ms, 4600);
    }

    #[test]
    fn applies_detected_intro_to_first_line_start() {
        let lines = vec![line("hello world"), line("this is the chorus")];
        let segments = vec![
            segment(0, 2600, "hello world"),
            segment(3000, 4600, "this is the chorus"),
        ];

        let aligned = align_lyrics_to_segments_with_intro(&lines, &segments, Some(1250)).unwrap();

        assert_eq!(aligned.detected_intro_ms, Some(1250));
        assert_eq!(aligned.lines[0].start_ms, 1250);
        assert_eq!(aligned.lines[0].end_ms, 2600);
        assert_eq!(aligned.lines[1].start_ms, 3000);
    }

    #[test]
    fn fills_unmatched_lines_between_anchors() {
        let lines = vec![line("first line"), line("unknown lyric"), line("last line")];
        let segments = vec![
            segment(1000, 2000, "first line"),
            segment(5000, 6000, "last line"),
        ];

        let aligned = align_lyrics_to_segments(&lines, &segments).unwrap();
        assert_eq!(aligned.matched_lines, 2);
        assert_eq!(aligned.lines[1].start_ms, 2000);
        assert_eq!(aligned.lines[1].end_ms, 5000);
    }
}
