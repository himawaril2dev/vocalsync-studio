//! 中央聲道消除（Phase 3-new-b）
//!
//! 利用 L-R 差分消除立體聲混音中居中（Center-panned）的人聲，
//! 保留左右聲道差異的樂器部分。
//!
//! 適用場景：流行歌曲（人聲通常 center-panned）
//! 不適用場景：mono 錄音、Live 錄音、stereo reverb 很重的混音
//!
//! 零下載、零 license、零模型依賴的 MVP 路徑。

use crate::core::media_loader;
use crate::error::AppError;

/// 對立體聲交錯樣本進行中央聲道消除。
///
/// 輸入：交錯立體聲 `[L0, R0, L1, R1, ...]`
/// 輸出：mono 樣本 `[(L0-R0)/2, (L1-R1)/2, ...]`
///
/// 中央成分（L≈R 的部分，通常是人聲）被消除，
/// 側邊成分（L≠R 的部分，通常是樂器）被保留。
pub fn cancel_center_stereo(interleaved: &[f32]) -> Vec<f32> {
    let frame_count = interleaved.len() / 2;
    let mut mono = Vec::with_capacity(frame_count);

    for i in 0..frame_count {
        let left = interleaved[i * 2];
        let right = interleaved[i * 2 + 1];
        mono.push((left - right) * 0.5);
    }

    mono
}

/// 從音檔載入 → 中央聲道消除 → 回傳 mono 樣本與取樣率。
///
/// 若輸入為 mono（channels < 2），回傳錯誤——
/// 因為 L-R 消除無法對單聲道運作。
pub fn load_and_cancel_center(path: &str) -> Result<(Vec<f32>, u32), AppError> {
    let media = media_loader::load_media(path)?;

    if media.channels < 2 {
        return Err(AppError::Audio(
            "中央聲道消除需要立體聲音檔，此檔案為單聲道。".to_string(),
        ));
    }

    let mono = cancel_center_stereo(&media.samples);
    Ok((mono, media.sample_rate))
}

// ── 測試 ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancels_center_content() {
        // 中央成分（L=R）應被完全消除
        let stereo = vec![0.5, 0.5, -0.3, -0.3, 1.0, 1.0];
        let mono = cancel_center_stereo(&stereo);
        assert_eq!(mono.len(), 3);
        for sample in &mono {
            assert!(sample.abs() < 1e-6, "中央成分應被消除");
        }
    }

    #[test]
    fn preserves_side_content() {
        // 純側邊成分（L=-R）應完整保留
        let stereo = vec![0.8, -0.8, -0.4, 0.4];
        let mono = cancel_center_stereo(&stereo);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.8).abs() < 1e-6, "側邊成分應保留");
        assert!((mono[1] - (-0.4)).abs() < 1e-6, "側邊成分應保留");
    }

    #[test]
    fn handles_mixed_content() {
        // 混合：vocal(center) + guitar(side)
        // L = vocal + guitar, R = vocal - guitar
        let vocal = 0.6;
        let guitar = 0.3;
        let left = vocal + guitar; // 0.9
        let right = vocal - guitar; // 0.3
        let stereo = vec![left, right];
        let mono = cancel_center_stereo(&stereo);
        // (L-R)/2 = (0.9-0.3)/2 = 0.3 = guitar
        assert!((mono[0] - guitar).abs() < 1e-6);
    }

    #[test]
    fn handles_empty_input() {
        let mono = cancel_center_stereo(&[]);
        assert!(mono.is_empty());
    }
}
