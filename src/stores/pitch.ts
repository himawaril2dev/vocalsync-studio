import { writable } from "svelte/store";

export interface PitchSample {
  freq: number;
  note: string;
  octave: number;
  cent: number;
  confidence: number;
}

export interface PitchTrackSample {
  timestamp: number;
  freq: number;
  confidence: number;
  note: string;
  octave: number;
  cent: number;
}

export interface PitchTrack {
  samples: PitchTrackSample[];
}

/** 即時音高（錄音中持續更新，None 代表靜音）*/
export const currentPitch = writable<PitchSample | null>(null);

/** 完整人聲音高軌跡（錄音結束後查詢，供音高分析頁使用）*/
export const pitchTrack = writable<PitchTrack>({ samples: [] });

/** 伴奏旋律音高軌跡（載入伴奏後背景分析得到）*/
export const backingPitchTrack = writable<PitchTrack | null>(null);

/** 伴奏分析品質摘要 */
export interface BackingPitchQuality {
  total_frames: number;
  voiced_frames: number;
  voiced_ratio: number;
  mean_confidence: number;
  elapsed_secs: number;
}

export const backingPitchQuality = writable<BackingPitchQuality | null>(null);

/** 自由模式狀態：true 代表伴奏沒有可偵測的主旋律，前端應隱藏目標旋律線 */
export const freeMode = writable<boolean>(false);

/** 自由模式提示文字（給 UI 顯示「為什麼進入自由模式」）*/
export const freeModeReason = writable<string>("");

/**
 * 伴奏旋律分析中狀態：
 * - null = 未分析（idle）
 * - { duration } = 分析中，UI 應顯示「分析中…」橫幅
 */
export interface BackingPitchAnalyzing {
  duration: number;
}
export const backingPitchAnalyzing = writable<BackingPitchAnalyzing | null>(null);

/** 即時累積的人聲樣本（為了在 PitchTimeline 中畫線，每次 currentPitch 更新就 push）*/
export const liveVocalSamples = writable<PitchTrackSample[]>([]);

/** 清空即時人聲樣本（錄音開始時呼叫）*/
export function clearLiveVocalSamples() {
  liveVocalSamples.set([]);
}

// ── 調性偵測 ─────────────────────────────────────────────────────

/** 調性偵測結果（對應後端 KeyResult） */
export interface KeyResult {
  key: string;
  tonic: number;
  mode: string;
  correlation: number;
  all_correlations: KeyCorrelation[];
  chroma: number[];
  sample_count: number;
}

export interface KeyCorrelation {
  key: string;
  correlation: number;
}

/** 偵測到的調性（null = 尚未偵測 / 偵測失敗） */
export const detectedKey = writable<KeyResult | null>(null);

/** 調性偵測狀態 */
export type KeyDetectionStatus = "idle" | "detecting" | "done" | "error";
export const keyDetectionStatus = writable<KeyDetectionStatus>("idle");

/** 重置調性偵測狀態 */
export function resetKeyDetection() {
  detectedKey.set(null);
  keyDetectionStatus.set("idle");
}

/** 重置與伴奏相關的所有 store（載入新伴奏時呼叫）*/
export function resetBackingState() {
  backingPitchTrack.set(null);
  backingPitchQuality.set(null);
  backingPitchAnalyzing.set(null);
  freeMode.set(false);
  freeModeReason.set("");
  resetKeyDetection();
}
