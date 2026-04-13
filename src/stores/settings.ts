import { writable } from "svelte/store";

export const inputDeviceIndex = writable<number | null>(null);
export const outputDeviceIndex = writable<number | null>(null);

// 延遲補償設定 (預設 0)
export const latencyMs = writable<number>(0);

// 伴奏音量 (預設 0.1 = 10%)
export const backingVolume = writable<number>(0.1);

// 麥克風增益 (預設 1.0)
export const micGain = writable<number>(1.0);

// 自動標準化混音 (預設 true)
export const autoBalanceMixin = writable<boolean>(true);

// 音高偵測引擎偏好: "auto" | "crepe" | "yin"
export type PitchEngineType = "auto" | "crepe" | "yin";
export const pitchEngine = writable<PitchEngineType>("auto");

// ── 校準流程狀態 ──────────────────────────────────────────────────

export interface CalibrationBeat {
  beatIdx: number;
  isWarmup: boolean;
  detected: boolean;
  accepted: boolean;
  offsetMs: number;
}

export interface CalibrationStatus {
  /** 是否正在進行校準（含 visualizer 顯示）*/
  isRunning: boolean;
  /** 後端 emit 的時間軸參數（拿到 calibration:started 後設定）*/
  bpm: number;
  warmupBeats: number;
  measurementBeats: number;
  prepMs: number;
  beatIntervalMs: number;
  /** 逐拍偵測結果（按 beatIdx 排序）*/
  beats: CalibrationBeat[];
  /** 最終延遲（成功時）*/
  finalLatencyMs: number | null;
  /** 量測標準差（成功時）*/
  stdDevMs: number | null;
  /** 失敗訊息（失敗時）*/
  error: string | null;
}

const defaultCalibration: CalibrationStatus = {
  isRunning: false,
  bpm: 70,
  warmupBeats: 2,
  measurementBeats: 6,
  prepMs: 1500,
  beatIntervalMs: 857,
  beats: [],
  finalLatencyMs: null,
  stdDevMs: null,
  error: null,
};

export const calibrationStatus = writable<CalibrationStatus>(defaultCalibration);

/** 重置成空白狀態（呼叫於每次校準前）*/
export function resetCalibrationStatus(): void {
  calibrationStatus.set({ ...defaultCalibration });
}
