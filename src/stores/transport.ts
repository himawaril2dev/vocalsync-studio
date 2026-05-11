import { writable, derived, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export type TransportState =
  | "idle"
  | "previewing"
  | "recording"
  | "playing_back"
  | "paused";

/** paused 狀態下記住「繼續要回到哪個模式」。非 paused 時為 null。 */
export type ResumeMode = "previewing" | "recording" | "playing_back";

export const transportState = writable<TransportState>("idle");
/** 只在 transportState === "paused" 時有值，指示繼續時要重啟哪一種模式。 */
export const pausedResumeMode = writable<ResumeMode | null>(null);

/** 進入 paused 時記住當時的 elapsed 秒數。
 *  續錄時用來偵測使用者是否拖進度條向前跳（target < pausedAt），
 *  這種操作會捨棄後段錄音，前端要先 dialog 確認。 */
export const pausedAtElapsed = writable<number | null>(null);

/** 當前是否處於「進行中」狀態（排除 idle 與 paused）。*/
export const isTransportRunning = derived(
  transportState,
  ($t) => $t === "previewing" || $t === "recording" || $t === "playing_back",
);

/** 當前是否處於 paused。*/
export const isTransportPaused = derived(
  transportState,
  ($t) => $t === "paused",
);

export const elapsed = writable<number>(0);
export const duration = writable<number>(0);
export const backingRms = writable<number>(0);
export const micRms = writable<number>(0);
export const hasRecording = writable<boolean>(false);

// ── A-B 循環 ──────────────────────────────────────────────────────

/** A 點（秒），null 代表未設定 */
export const loopA = writable<number | null>(null);
/** B 點（秒），null 代表未設定 */
export const loopB = writable<number | null>(null);

/** A-B 循環是否啟用（兩端都設定了才算） */
export const loopActive = derived(
  [loopA, loopB],
  ([$a, $b]) => $a !== null && $b !== null,
);

/**
 * 設定 A 點為目前播放位置。
 * 如果已設定的 A 點 >= 已設定的 B 點，自動清除 B 點。
 */
export async function setLoopA(): Promise<void> {
  const t = get(elapsed);
  const b = get(loopB);
  if (b !== null && t >= b) {
    loopB.set(null);
  }
  loopA.set(t);
  await syncLoopToBackend();
}

/**
 * 設定 B 點為目前播放位置。
 * 如果 B 點 <= A 點，回傳 false（操作被忽略）。
 */
export async function setLoopB(): Promise<boolean> {
  const t = get(elapsed);
  const a = get(loopA);
  if (a !== null && t <= a) return false;
  loopB.set(t);
  await syncLoopToBackend();
  return true;
}

/** 直接設定 A-B 循環範圍（秒），用於歌詞行點擊 */
export async function setLoopRange(startSec: number, endSec: number): Promise<void> {
  if (endSec <= startSec) return;
  loopA.set(startSec);
  loopB.set(endSec);
  await syncLoopToBackend();
}

/** 清除 A-B 循環 */
export async function clearLoop(): Promise<void> {
  loopA.set(null);
  loopB.set(null);
  try {
    await invoke("clear_loop");
  } catch (e) {
    console.error("[loop] clear_loop failed:", e);
  }
}

/** 將前端 loop 狀態同步到 Rust 後端 */
async function syncLoopToBackend(): Promise<void> {
  const a = get(loopA);
  const b = get(loopB);
  try {
    if (a !== null && b !== null) {
      await invoke("set_loop_points", { aSecs: a, bSecs: b });
    } else {
      await invoke("clear_loop");
    }
  } catch (e) {
    console.error("[loop] sync failed:", e);
  }
}

// ── 變速不變調 / 移調 ──────────────────────────────────────────────

/** 播放速度（0.25 ~ 4.0，1.0 = 正常） */
export const speed = writable<number>(1.0);

/** 移調半音數上下限（半音）。
 *
 *  ±7 的依據：karaoke 轉調實務需求約 ±5-7 半音；超過此範圍 HouseLoop phase
 *  vocoder 品質明顯下降（musical noise 增多、formant 失真）。
 *  後端（`audio_commands.rs::PITCH_SEMITONES_MIN/MAX`）會再做一次防禦性 clamp。
 */
export const PITCH_SEMITONES_MIN = -7;
export const PITCH_SEMITONES_MAX = 7;

/** 移調半音數（-7 ~ +7，0 = 不移調） */
export const pitchSemitones = writable<number>(0);

/** 速度是否偏離預設值 1.0x */
export const hasNonDefaultSpeed = derived(
  speed,
  ($s) => Math.abs($s - 1.0) > 0.01,
);

/** 移調是否偏離預設值 0 半音 */
export const hasNonDefaultPitch = derived(
  pitchSemitones,
  ($p) => $p !== 0,
);

/** 設定播放速度並同步到後端 */
export async function setSpeed(value: number): Promise<void> {
  const clamped = Math.max(0.25, Math.min(4.0, value));
  speed.set(clamped);
  try {
    await invoke("set_speed", { speed: clamped });
  } catch (e) {
    console.error("[speed] set_speed failed:", e);
  }
}

/** 設定移調半音數並同步到後端 */
export async function setPitchSemitones(value: number): Promise<void> {
  const clamped = Math.round(
    Math.max(PITCH_SEMITONES_MIN, Math.min(PITCH_SEMITONES_MAX, value)),
  );
  pitchSemitones.set(clamped);
  try {
    await invoke("set_pitch_semitones", { semitones: clamped });
  } catch (e) {
    console.error("[pitch] set_pitch_semitones failed:", e);
  }
}

/** 重設速度到預設值 1.0x */
export async function resetSpeed(): Promise<void> {
  await setSpeed(1.0);
}

/** 重設移調到預設值 0 半音 */
export async function resetPitch(): Promise<void> {
  await setPitchSemitones(0);
}
