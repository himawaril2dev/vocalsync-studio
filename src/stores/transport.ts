import { writable, derived, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export type TransportState =
  | "idle"
  | "previewing"
  | "recording"
  | "playing_back";

export const transportState = writable<TransportState>("idle");
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

/** 移調半音數（-24 ~ +24，0 = 不移調） */
export const pitchSemitones = writable<number>(0);

/** 是否有速度或音高調整（非預設值） */
export const hasSpeedOrPitch = derived(
  [speed, pitchSemitones],
  ([$s, $p]) => Math.abs($s - 1.0) > 0.01 || $p !== 0,
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
  const clamped = Math.round(Math.max(-24, Math.min(24, value)));
  pitchSemitones.set(clamped);
  try {
    await invoke("set_pitch_semitones", { semitones: clamped });
  } catch (e) {
    console.error("[pitch] set_pitch_semitones failed:", e);
  }
}

/** 重設速度和音高到預設值 */
export async function resetSpeedAndPitch(): Promise<void> {
  await setSpeed(1.0);
  await setPitchSemitones(0);
}
