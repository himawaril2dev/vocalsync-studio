import { writable, derived } from "svelte/store";
import { elapsed } from "./transport";

export interface LyricLine {
  start_ms: number;
  end_ms: number;
  text: string;
  /** 翻譯文字（雙語歌詞時使用） */
  translation?: string;
}

export const lyricsLines = writable<LyricLine[]>([]);
export const lyricsFileName = writable<string>("");

/** 根據 elapsed（秒）計算當前歌詞行的索引；無歌詞或在歌詞外回傳 -1 */
export const currentLyricIndex = derived(
  [lyricsLines, elapsed],
  ([$lines, $elapsed]) => {
    if ($lines.length === 0) return -1;
    const nowMs = $elapsed * 1000;
    // 二分搜尋（可能行數很多）
    let lo = 0;
    let hi = $lines.length - 1;
    let result = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const line = $lines[mid];
      if (nowMs < line.start_ms) {
        hi = mid - 1;
      } else if (nowMs >= line.end_ms) {
        lo = mid + 1;
      } else {
        result = mid;
        break;
      }
    }
    // 若沒命中，找最近一個 start_ms <= nowMs 的行
    if (result === -1 && $lines.length > 0) {
      for (let i = $lines.length - 1; i >= 0; i--) {
        if ($lines[i].start_ms <= nowMs) {
          result = i;
          break;
        }
      }
    }
    return result;
  },
);
