import { writable, derived } from "svelte/store";
import { elapsed } from "./transport";

export interface LyricLine {
  start_ms: number;
  end_ms: number;
  text: string;
  /** 翻譯文字（雙語歌詞時使用） */
  translation?: string;
}

export type LyricBoundary = "start" | "end";

export type LyricBoundaryResult =
  | {
      ok: true;
      boundary: LyricBoundary;
      index: number;
      time_ms: number;
      line: LyricLine;
    }
  | {
      ok: false;
      reason: "line_missing" | "end_before_start";
      index: number;
      time_ms: number;
      start_ms?: number;
    };

const DEFAULT_SYNC_LINE_DURATION_MS = 3000;

export const lyricsLines = writable<LyricLine[]>([]);
export const lyricsFileName = writable<string>("");

function roundSyncTimeMs(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.round(value / 10) * 10);
}

function fallbackEndMs(lines: LyricLine[], index: number, startMs: number): number {
  const nextStart = lines[index + 1]?.start_ms;
  if (typeof nextStart === "number" && nextStart > startMs) return nextStart;
  return startMs + DEFAULT_SYNC_LINE_DURATION_MS;
}

export function setLyricBoundary(
  index: number,
  boundary: LyricBoundary,
  currentMs: number,
): LyricBoundaryResult {
  const timeMs = roundSyncTimeMs(currentMs);
  let result: LyricBoundaryResult = {
    ok: false,
    reason: "line_missing",
    index,
    time_ms: timeMs,
  };

  lyricsLines.update((lines) => {
    const line = lines[index];
    if (!line) return lines;

    if (boundary === "end" && timeMs <= line.start_ms) {
      result = {
        ok: false,
        reason: "end_before_start",
        index,
        time_ms: timeMs,
        start_ms: line.start_ms,
      };
      return lines;
    }

    const next = lines.slice();
    if (boundary === "start") {
      const endMs =
        line.end_ms > timeMs ? line.end_ms : fallbackEndMs(lines, index, timeMs);
      next[index] = {
        ...line,
        start_ms: timeMs,
        end_ms: endMs,
      };
    } else {
      next[index] = {
        ...line,
        end_ms: timeMs,
      };
    }

    result = {
      ok: true,
      boundary,
      index,
      time_ms: timeMs,
      line: next[index],
    };
    return next;
  });

  return result;
}

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
