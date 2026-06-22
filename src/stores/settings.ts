import { writable } from "svelte/store";

export const inputDeviceIndex = writable<number | null>(null);
export const outputDeviceIndex = writable<number | null>(null);
export const latencyMs = writable<number>(0);

export const DEFAULT_BACKING_VOLUME = 0.1;
export const backingVolume = writable<number>(DEFAULT_BACKING_VOLUME);

export const DEFAULT_MIC_GAIN = 1.0;
export const micGain = writable<number>(DEFAULT_MIC_GAIN);

export const DEFAULT_GUIDE_VOLUME = 0.25;
export const guideVolume = writable<number>(DEFAULT_GUIDE_VOLUME);
export const guideVocalEnabled = writable<boolean>(false);

export type ExportNamingMode = "manual" | "auto";
export const DEFAULT_EXPORT_NAMING_MODE: ExportNamingMode = "auto";
export const exportNamingMode = writable<ExportNamingMode>(DEFAULT_EXPORT_NAMING_MODE);

export function resetBackingVolume(): void {
  backingVolume.set(DEFAULT_BACKING_VOLUME);
}

export function resetMicGain(): void {
  micGain.set(DEFAULT_MIC_GAIN);
}

export function resetGuideVolume(): void {
  guideVolume.set(DEFAULT_GUIDE_VOLUME);
}

export const autoBalanceMixin = writable<boolean>(true);

export type AutoBalanceVocalPreset = "natural" | "clear" | "forward";
export const DEFAULT_AUTO_BALANCE_VOCAL_PRESET: AutoBalanceVocalPreset = "natural";
export const autoBalanceVocalPreset = writable<AutoBalanceVocalPreset>(
  DEFAULT_AUTO_BALANCE_VOCAL_PRESET,
);

export type PitchEngineType = "auto" | "crepe" | "yin";
export const pitchEngine = writable<PitchEngineType>("auto");
