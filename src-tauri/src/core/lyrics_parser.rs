//! LRC / SRT / VTT 歌詞解析器
//!
//! 統一輸出 `Vec<LyricLine>`，每行包含：
//! - start_ms / end_ms：時間範圍（毫秒）
//! - text：歌詞文字
//!
//! 支援格式：
//! - 標準 LRC：`[mm:ss.xx]歌詞`
//! - 多時間標記 LRC：`[00:12.50][01:30.20]同一行歌詞`
//! - SRT 字幕：`00:01:23,456 --> 00:01:25,789`
//! - WebVTT：`00:01:23.456 --> 00:01:25.789`（YouTube 自動字幕常用格式）

use crate::error::AppError;
use serde::Serialize;
use std::fs;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// 翻譯文字（雙語歌詞時使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
}

/// 載入歌詞檔（自動依副檔名選擇解析器）
pub fn load_lyrics(path: &str) -> Result<Vec<LyricLine>, AppError> {
    // 嘗試讀檔（支援 UTF-8 / UTF-8 with BOM / Big5 等常見編碼）
    let raw_bytes =
        fs::read(path).map_err(|e| AppError::Audio(format!("無法讀取歌詞檔：{}", e)))?;
    let content = decode_text(&raw_bytes);

    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut lines = match ext.as_str() {
        "srt" => parse_srt(&content),
        "vtt" => parse_vtt(&content),
        _ => parse_lrc(&content), // 預設當 LRC 處理（lrc / txt / 其他）
    };

    if lines.is_empty() {
        return Err(AppError::Audio("歌詞檔為空或格式不正確".to_string()));
    }

    // 自動偵測雙語歌詞並拆分翻譯
    detect_and_split_bilingual(&mut lines);

    Ok(lines)
}

/// 解析字串內容為 LRC（給其他模組用）
pub fn parse_lrc_text(content: &str) -> Vec<LyricLine> {
    parse_lrc(content)
}

/// 解析字串內容為 VTT（給字幕整合用）
pub fn parse_vtt_text(content: &str) -> Vec<LyricLine> {
    parse_vtt(content)
}

/// 解析字串內容為 SRT（給字幕整合用）
pub fn parse_srt_text(content: &str) -> Vec<LyricLine> {
    parse_srt(content)
}

// ── 字元編碼處理 ──────────────────────────────────────────────────

fn decode_text(bytes: &[u8]) -> String {
    // 處理 UTF-8 BOM
    let stripped = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };

    // 嘗試 UTF-8
    if let Ok(s) = std::str::from_utf8(stripped) {
        return s.to_string();
    }

    // Fallback：用 lossy 模式（無法解碼的字元變成 �）
    String::from_utf8_lossy(stripped).to_string()
}

// ── LRC 解析 ──────────────────────────────────────────────────────

fn parse_lrc(content: &str) -> Vec<LyricLine> {
    let mut entries: Vec<(u64, String)> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // 收集所有時間標記與該行的歌詞
        // metadata tag（[ar:歌手] 等）會被 parse_lrc_time 自動拒絕，無需特別判斷
        let (timestamps, text) = extract_lrc_timestamps(line);
        if timestamps.is_empty() || text.is_empty() {
            continue;
        }

        for ts in timestamps {
            entries.push((ts, text.clone()));
        }
    }

    // 依時間排序
    entries.sort_by_key(|e| e.0);

    // 計算 end_ms（下一行的 start_ms，或最後一行 +5 秒）
    let mut lines: Vec<LyricLine> = Vec::with_capacity(entries.len());
    for (i, (start, text)) in entries.iter().enumerate() {
        let end = entries.get(i + 1).map(|(s, _)| *s).unwrap_or(start + 5000);
        lines.push(LyricLine {
            start_ms: *start,
            end_ms: end,
            text: text.clone(),
            translation: None,
        });
    }

    lines
}

/// 從一行抽取所有時間標記，回傳 (timestamps_ms, text)
///
/// 寬鬆策略：
/// - 連續處理所有 `[...]` 標記
/// - 能 parse 為時間 → 收為時間戳
/// - 不能 parse → 視為 metadata，跳過該標記但繼續處理下一個
/// - 剩下的非標記文字視為歌詞文本
fn extract_lrc_timestamps(line: &str) -> (Vec<u64>, String) {
    let mut timestamps = Vec::new();
    let mut rest = line;

    loop {
        rest = rest.trim_start();
        if !rest.starts_with('[') {
            break;
        }
        let close = match rest.find(']') {
            Some(i) => i,
            None => break,
        };
        let tag = &rest[1..close];
        // 嘗試解析為時間；失敗則當 metadata 跳過（仍繼續下個標記）
        if let Some(ms) = parse_lrc_time(tag) {
            timestamps.push(ms);
        }
        rest = &rest[close + 1..];
    }

    (timestamps, rest.trim().to_string())
}

/// 解析 LRC 時間：`mm:ss.xx` 或 `mm:ss.xxx` 或 `mm:ss`
fn parse_lrc_time(s: &str) -> Option<u64> {
    let colon = s.find(':')?;
    let min: u64 = s[..colon].parse().ok()?;
    let rest = &s[colon + 1..];

    let (sec_str, ms_str) = if let Some(dot) = rest.find('.') {
        (&rest[..dot], &rest[dot + 1..])
    } else {
        (rest, "0")
    };

    let sec: u64 = sec_str.parse().ok()?;
    // 補齊到毫秒（xx → x*10, xxx → 直接取）
    let ms: u64 = match ms_str.len() {
        0 => 0,
        1 => ms_str.parse::<u64>().ok()? * 100,
        2 => ms_str.parse::<u64>().ok()? * 10,
        _ => ms_str[..3].parse().ok()?,
    };

    Some(min * 60_000 + sec * 1000 + ms)
}

// ── SRT 解析 ──────────────────────────────────────────────────────

fn parse_srt(content: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    // 🟡 Y5 修正：統一換行符（Windows \r\n → \n）
    let normalized = content.replace("\r\n", "\n");
    let blocks: Vec<&str> = normalized.split("\n\n").collect();

    for block in blocks {
        let block_lines: Vec<&str> = block.lines().collect();
        if block_lines.len() < 3 {
            continue;
        }

        // block_lines[0] 是序號（忽略）
        // block_lines[1] 是時間：00:01:23,456 --> 00:01:25,789
        let time_line = block_lines[1];
        let parts: Vec<&str> = time_line.split(" --> ").collect();
        if parts.len() != 2 {
            continue;
        }

        let start_ms = match parse_srt_time(parts[0]) {
            Some(v) => v,
            None => continue,
        };
        let end_ms = match parse_srt_time(parts[1]) {
            Some(v) => v,
            None => continue,
        };

        let text = block_lines[2..].join(" ");
        if text.is_empty() {
            continue;
        }

        lines.push(LyricLine {
            start_ms,
            end_ms,
            text,
            translation: None,
        });
    }

    lines
}

/// 解析 SRT 時間：`HH:MM:SS,mmm`
fn parse_srt_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: u64 = parts[0].parse().ok()?;
    let m: u64 = parts[1].parse().ok()?;
    let sec_part = parts[2].replace(',', ".");
    let (sec_str, ms_str) = sec_part.split_once('.').unwrap_or((sec_part.as_str(), "0"));
    let sec: u64 = sec_str.parse().ok()?;
    let ms: u64 = match ms_str.len() {
        0 => 0,
        1 => ms_str.parse::<u64>().ok()? * 100,
        2 => ms_str.parse::<u64>().ok()? * 10,
        _ => ms_str[..3].parse().ok()?,
    };
    Some(h * 3_600_000 + m * 60_000 + sec * 1000 + ms)
}

// ── VTT 解析 ─────────────────────────────────────────────────────

/// 解析 WebVTT 字幕。
///
/// 格式範例（YouTube 自動字幕常見）：
/// ```text
/// WEBVTT
/// Kind: captions
/// Language: zh-TW
///
/// 00:00:01.000 --> 00:00:04.500
/// 第一行歌詞
///
/// 00:00:04.500 --> 00:00:08.000
/// 第二行歌詞
/// ```
///
/// 特殊處理：
/// - 跳過 `WEBVTT` 標頭和 metadata（`Kind:`, `Language:` 等）
/// - 跳過 cue settings（`align:start position:0%` 等附在時間行後面的參數）
/// - 移除 HTML 標籤（YouTube 自動字幕常有 `<c>` / `</c>` 等標記）
/// - 去除重複行（YouTube 自動字幕常有逐字捲動造成的大量重複）
fn parse_vtt(content: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    // 🟡 Y5 修正：統一換行符（Windows \r\n → \n）
    let normalized = content.replace("\r\n", "\n");
    let blocks: Vec<&str> = normalized.split("\n\n").collect();

    for block in blocks {
        let block_lines: Vec<&str> = block.lines().collect();
        if block_lines.is_empty() {
            continue;
        }

        // 找時間行（可能是第一行或第二行，第一行可能是 cue ID）
        let mut time_line_idx = None;
        for (i, line) in block_lines.iter().enumerate() {
            if line.contains("-->") {
                time_line_idx = Some(i);
                break;
            }
        }

        let time_idx = match time_line_idx {
            Some(i) => i,
            None => continue,
        };

        let time_line = block_lines[time_idx];

        // 解析時間（忽略 cue settings）
        let arrow_parts: Vec<&str> = time_line.split("-->").collect();
        if arrow_parts.len() != 2 {
            continue;
        }

        let start_ms = match parse_vtt_time(arrow_parts[0].trim()) {
            Some(v) => v,
            None => continue,
        };
        // B 部分可能有 cue settings：`00:00:04.500 align:start`
        let end_part = arrow_parts[1].trim();
        let end_time_str = end_part.split_whitespace().next().unwrap_or(end_part);
        let end_ms = match parse_vtt_time(end_time_str) {
            Some(v) => v,
            None => continue,
        };

        // 文字行（時間行之後的所有行）
        let text_lines = &block_lines[time_idx + 1..];
        let raw_text = text_lines.join(" ");
        let text = strip_vtt_tags(&raw_text).trim().to_string();
        if text.is_empty() {
            continue;
        }

        // 🔴 R3 修正：改善去重邏輯，處理 YouTube 自動字幕三種常見重複模式：
        // 1. 完全相同（"歌詞A" → "歌詞A"）
        // 2. 逐字捲動/漸進式（"你" → "你好" → "你好嗎"）→ 後者取代前者
        // 3. 間隔重複由呼叫方或後處理處理（不在此層）
        if let Some(prev) = lines.last_mut() {
            let time_close = start_ms.abs_diff(prev.end_ms) < 100
                || (start_ms >= prev.start_ms && start_ms < prev.end_ms);

            if time_close {
                // 完全相同 → 跳過
                if prev.text == text {
                    continue;
                }
                // 新行是前一行的延伸（逐字捲動）→ 取代前一行，延長時間
                if text.starts_with(&prev.text) || prev.text.starts_with(&text) {
                    // 保留較長的那個
                    if text.len() >= prev.text.len() {
                        prev.text = text;
                    }
                    prev.end_ms = end_ms;
                    continue;
                }
            }
        }

        lines.push(LyricLine {
            start_ms,
            end_ms,
            text,
            translation: None,
        });
    }

    lines
}

/// 解析 VTT 時間：`HH:MM:SS.mmm` 或 `MM:SS.mmm`
fn parse_vtt_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        // MM:SS.mmm
        2 => {
            let m: u64 = parts[0].parse().ok()?;
            let (sec, ms) = parse_sec_ms(parts[1])?;
            Some(m * 60_000 + sec * 1000 + ms)
        }
        // HH:MM:SS.mmm
        3 => {
            let h: u64 = parts[0].parse().ok()?;
            let m: u64 = parts[1].parse().ok()?;
            let (sec, ms) = parse_sec_ms(parts[2])?;
            Some(h * 3_600_000 + m * 60_000 + sec * 1000 + ms)
        }
        _ => None,
    }
}

/// 從 `SS.mmm` 或 `SS,mmm` 格式解析秒和毫秒
fn parse_sec_ms(s: &str) -> Option<(u64, u64)> {
    let s = s.replace(',', ".");
    let (sec_str, ms_str) = s.split_once('.').unwrap_or((&s, "0"));
    let sec: u64 = sec_str.parse().ok()?;
    let ms: u64 = match ms_str.len() {
        0 => 0,
        1 => ms_str.parse::<u64>().ok()? * 100,
        2 => ms_str.parse::<u64>().ok()? * 10,
        _ => ms_str[..3].parse().ok()?,
    };
    Some((sec, ms))
}

/// 移除 VTT 中常見的 HTML 標籤（`<c>`, `</c>`, `<b>`, `<i>` 等）
fn strip_vtt_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

// ── 雙語歌詞偵測與拆分 ──────────────────────────────────────────

/// 半形斜線 ` / ` 或全形斜線 `／` 分隔符
const SLASH_SEPARATORS: &[&str] = &[" / ", "／"];

/// 自動偵測雙語歌詞並拆分翻譯。
///
/// 策略：
/// 1. 若 >50% 的行包含斜線分隔符（` / ` 或 `／`），視為雙��� LRC，
///    按分隔符拆分 text 和 translation。
/// 2. 若有連續相同時間戳的行對（同一 start_ms），合併為一行 + 翻譯。
fn detect_and_split_bilingual(lines: &mut Vec<LyricLine>) {
    if lines.is_empty() {
        return;
    }

    // ── 策略 1：斜線分隔 ──
    let slash_count = lines
        .iter()
        .filter(|l| SLASH_SEPARATORS.iter().any(|sep| l.text.contains(sep)))
        .count();

    if slash_count > lines.len() / 2 {
        for line in lines.iter_mut() {
            for sep in SLASH_SEPARATORS {
                if let Some(pos) = line.text.find(sep) {
                    let original = line.text[..pos].trim().to_string();
                    let translation = line.text[pos + sep.len()..].trim().to_string();
                    if !translation.is_empty() {
                        line.text = original;
                        line.translation = Some(translation);
                    }
                    break; // 每行只拆一次
                }
            }
        }
        return;
    }

    // ── 策略 2：相同時間戳行對 ──
    // 找出 start_ms 完全相同的連續行對，第二行視為翻譯
    let mut i = 0;
    let mut merged: Vec<LyricLine> = Vec::with_capacity(lines.len());

    while i < lines.len() {
        if i + 1 < lines.len() && lines[i].start_ms == lines[i + 1].start_ms {
            let mut line = lines[i].clone();
            line.translation = Some(lines[i + 1].text.clone());
            // end_ms 取較長的那個
            line.end_ms = line.end_ms.max(lines[i + 1].end_ms);
            merged.push(line);
            i += 2;
        } else {
            merged.push(lines[i].clone());
            i += 1;
        }
    }

    // 若有合併發生（merged 比 lines 短），替換原始資料
    if merged.len() < lines.len() {
        *lines = merged;
    }
}

/// 將歌詞匯出為 LRC 格式字串。
///
/// 若行有翻譯，以 ` / ` 分隔符合併。
pub fn export_lrc(lines: &[LyricLine]) -> String {
    let mut output = String::new();
    for line in lines {
        let min = line.start_ms / 60_000;
        let sec = (line.start_ms % 60_000) / 1000;
        let ms = (line.start_ms % 1000) / 10; // LRC 精度到百分之一秒

        let text = if let Some(ref tr) = line.translation {
            format!("{} / {}", line.text, tr)
        } else {
            line.text.clone()
        };

        output.push_str(&format!("[{:02}:{:02}.{:02}]{}\n", min, sec, ms, text));
    }
    output
}

/// 掃描目錄中的字幕檔案（.srt, .vtt, .lrc）。
///
/// 若提供 `base_name`（下載檔案的主檔名，不含副檔名），則只回傳檔名以
/// `base_name` 開頭的字幕檔，避免誤關聯同目錄下的舊字幕。
/// `base_name` 為 None 時回傳所有字幕檔（向後相容）。
pub fn find_subtitle_files(dir: &str) -> Vec<String> {
    find_subtitle_files_filtered(dir, None)
}

pub fn find_subtitle_files_filtered(dir: &str, base_name: Option<&str>) -> Vec<String> {
    let dir_path = std::path::Path::new(dir);
    if !dir_path.is_dir() {
        return Vec::new();
    }

    let subtitle_exts = ["srt", "vtt", "lrc"];
    let mut found = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !subtitle_exts.contains(&ext.as_str()) {
                continue;
            }
            // 若指定 base_name，過濾不匹配的檔案
            if let Some(base) = base_name {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !stem.starts_with(base) {
                    continue;
                }
            }
            if let Some(s) = path.to_str() {
                found.push(s.to_string());
            }
        }
    }

    found.sort();
    found
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_lrc() {
        let lrc = "[ti:測試]\n[ar:測試者]\n[00:10.50]第一行\n[00:15.00]第二行\n[00:20.00]第三行";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].start_ms, 10500);
        assert_eq!(lines[0].end_ms, 15000);
        assert_eq!(lines[0].text, "第一行");
        assert_eq!(lines[2].end_ms, 25000); // 最後一行 +5s
    }

    #[test]
    fn parses_multi_timestamp_lrc() {
        let lrc = "[00:10.00][00:30.00]副歌";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 10000);
        assert_eq!(lines[1].start_ms, 30000);
    }

    #[test]
    fn parses_single_digit_minutes() {
        // 單位數分鐘 [m:ss.xx] 是常見格式
        let lrc = "[ar:測試]\n[0:05.00]第一句\n[1:30.50]第二句";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 5000);
        assert_eq!(lines[1].start_ms, 90500);
    }

    #[test]
    fn parses_no_milliseconds() {
        let lrc = "[00:10]第一句\n[00:20]第二句";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 10000);
    }

    #[test]
    fn rejects_metadata_correctly() {
        let lrc = "[ti:歌名]\n[ar:歌手]\n[al:專輯]\n[by:作者]\n[00:10.00]真正歌詞";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "真正歌詞");
    }

    #[test]
    fn parses_srt() {
        let srt = "1\n00:00:10,500 --> 00:00:15,000\nFirst line\n\n2\n00:00:15,000 --> 00:00:20,000\nSecond line";
        let lines = parse_srt(srt);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 10500);
        assert_eq!(lines[0].end_ms, 15000);
    }

    #[test]
    fn parses_basic_vtt() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: zh-TW\n\n00:00:01.000 --> 00:00:04.500\n第一行歌詞\n\n00:00:04.500 --> 00:00:08.000\n第二行歌詞";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 1000);
        assert_eq!(lines[0].end_ms, 4500);
        assert_eq!(lines[0].text, "第一行歌詞");
        assert_eq!(lines[1].start_ms, 4500);
        assert_eq!(lines[1].end_ms, 8000);
    }

    #[test]
    fn parses_vtt_with_cue_id_and_settings() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:04.500 align:start position:0%\nHello world\n\n2\n00:00:05.000 --> 00:00:08.000\nSecond line";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].end_ms, 4500);
    }

    #[test]
    fn vtt_strips_html_tags() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:04.500\n<c>tagged</c> text <b>bold</b>";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "tagged text bold");
    }

    #[test]
    fn vtt_deduplicates_consecutive_lines() {
        // YouTube 自動字幕常有逐字捲動造成重複
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\n歌詞一\n\n00:00:03.000 --> 00:00:05.000\n歌詞一\n\n00:00:05.000 --> 00:00:08.000\n歌詞二";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "歌詞一");
        assert_eq!(lines[1].text, "歌詞二");
    }

    #[test]
    fn vtt_handles_mm_ss_format() {
        // 短片可能沒有小時位
        let vtt = "WEBVTT\n\n01:30.000 --> 02:00.000\nShort format";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_ms, 90000);
        assert_eq!(lines[0].end_ms, 120000);
    }

    #[test]
    fn strips_vtt_tags_correctly() {
        assert_eq!(strip_vtt_tags("hello <c>world</c>"), "hello world");
        assert_eq!(strip_vtt_tags("<b>bold</b>"), "bold");
        assert_eq!(strip_vtt_tags("no tags"), "no tags");
        assert_eq!(strip_vtt_tags("<c.colorE5E5E5>text</c>"), "text");
    }

    // ── 雙語歌詞偵測 ──

    #[test]
    fn detects_slash_bilingual_lrc() {
        let lrc =
            "[00:10.00]こんにちは / 你好\n[00:15.00]さようなら / 再見\n[00:20.00]ありがとう / 謝謝";
        let mut lines = parse_lrc(lrc);
        detect_and_split_bilingual(&mut lines);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "こんにちは");
        assert_eq!(lines[0].translation.as_deref(), Some("你好"));
        assert_eq!(lines[2].text, "ありがとう");
        assert_eq!(lines[2].translation.as_deref(), Some("謝謝"));
    }

    #[test]
    fn detects_fullwidth_slash_bilingual() {
        let lrc = "[00:10.00]Hello／你好\n[00:15.00]World／世界\n[00:20.00]Test／測試";
        let mut lines = parse_lrc(lrc);
        detect_and_split_bilingual(&mut lines);
        assert_eq!(lines[0].text, "Hello");
        assert_eq!(lines[0].translation.as_deref(), Some("你好"));
    }

    #[test]
    fn no_bilingual_when_few_slashes() {
        // 只有 1/3 行有斜線，不到 50%，不應拆分
        let lrc = "[00:10.00]Normal text\n[00:15.00]Also normal / maybe\n[00:20.00]Third line";
        let mut lines = parse_lrc(lrc);
        detect_and_split_bilingual(&mut lines);
        // 不應拆分（< 50% 包含 ` / `���
        assert!(lines[1].translation.is_none());
        assert_eq!(lines[1].text, "Also normal / maybe");
    }

    #[test]
    fn detects_same_timestamp_bilingual() {
        // 相同時間戳的連續行，第二行為翻譯
        let lrc = "[00:10.00]こんにちは\n[00:10.00]你好\n[00:15.00]さようなら\n[00:15.00]再見";
        let mut lines = parse_lrc(lrc);
        detect_and_split_bilingual(&mut lines);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "こんにちは");
        assert_eq!(lines[0].translation.as_deref(), Some("你好"));
        assert_eq!(lines[1].text, "さようなら");
        assert_eq!(lines[1].translation.as_deref(), Some("再見"));
    }

    // ── 邊界情況 ──

    #[test]
    fn parse_lrc_empty_string() {
        let lines = parse_lrc("");
        assert!(lines.is_empty());
    }

    #[test]
    fn parse_lrc_only_metadata() {
        let lrc = "[ti:歌名]\n[ar:歌手]\n[al:專輯]";
        let lines = parse_lrc(lrc);
        assert!(lines.is_empty());
    }

    #[test]
    fn parse_srt_malformed_timestamp() {
        // 不完整的 SRT — 缺少 --> 分隔符
        let srt = "1\n00:00:10,500 00:00:15,000\nBroken line\n\n2\n00:00:15,000 --> 00:00:20,000\nGood line";
        let lines = parse_srt(srt);
        // 第一段壞掉的不應 panic，第二段應正常解析
        assert!(lines.len() >= 1);
        assert_eq!(lines.last().unwrap().text, "Good line");
    }

    #[test]
    fn parse_vtt_empty_cues() {
        let vtt =
            "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\n\n\n00:00:05.000 --> 00:00:08.000\n真正歌詞";
        let lines = parse_vtt(vtt);
        // 空白 cue 應被跳過
        assert!(lines.iter().any(|l| l.text == "真正歌詞"));
    }

    #[test]
    fn parse_lrc_time_edge_values() {
        // 零秒
        assert_eq!(parse_lrc_time("00:00.00"), Some(0));
        // 99 分鐘 59 秒（極端值）
        assert_eq!(
            parse_lrc_time("99:59.99"),
            Some(99 * 60 * 1000 + 59 * 1000 + 990)
        );
    }

    #[test]
    fn parse_srt_time_edge_values() {
        assert_eq!(parse_srt_time("00:00:00,000"), Some(0));
        assert_eq!(parse_srt_time("01:30:45,123"), Some(5445123));
    }

    #[test]
    fn decode_text_strips_utf8_bom() {
        let with_bom = b"\xEF\xBB\xBF[00:10.00]BOM test";
        let decoded = decode_text(with_bom);
        assert!(
            decoded.starts_with("[00:10.00]"),
            "BOM should be stripped, got: {}",
            &decoded[..20.min(decoded.len())]
        );
    }

    #[test]
    fn export_lrc_roundtrip() {
        let lines = vec![
            LyricLine {
                start_ms: 10500,
                end_ms: 15000,
                text: "第一行".to_string(),
                translation: None,
            },
            LyricLine {
                start_ms: 15000,
                end_ms: 20000,
                text: "日本語".to_string(),
                translation: Some("中文翻譯".to_string()),
            },
        ];
        let lrc = export_lrc(&lines);
        assert!(lrc.contains("[00:10.50]第一行"));
        assert!(lrc.contains("[00:15.00]日本語 / 中文翻譯"));
    }
}
