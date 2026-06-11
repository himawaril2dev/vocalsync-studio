import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { get, writable } from "svelte/store";
import { lyricsFileName, lyricsLines, type LyricLine } from "./lyrics";
import { basename, loadedMedia } from "./media";
import {
  alignmentFineTuneMs,
  alignmentResult,
  applyAlignmentToMelody,
  currentMelody,
  finalOffsetSecs,
  guideVocalPath,
  melodySourcePath,
  melodyToPitchTrack,
  resetMelodyState,
  type AlignmentResult,
  type MelodyTrack,
} from "./melody";
import {
  autoBalanceMixin,
  autoBalanceVocalPreset,
  backingVolume,
  exportNamingMode,
  guideVocalEnabled,
  guideVolume,
  micGain,
  type AutoBalanceVocalPreset,
  type ExportNamingMode,
} from "./settings";
import {
  clearLoop,
  hasRecording,
  loopA,
  loopB,
  pitchSemitones,
  setLoopRange,
  setPitchSemitones,
  setSpeed,
  speed,
} from "./transport";
import { backingPitchTrack, clearLiveVocalSamples, resetBackingState } from "./pitch";

export const projectSessionReady = writable<boolean>(false);

const PROJECT_SESSION_VERSION = 1;

export interface ProjectSession {
  version: typeof PROJECT_SESSION_VERSION;
  backingPath: string | null;
  lyricsFileName: string;
  lyricsLines: LyricLine[];
  melody: MelodyTrack | null;
  melodySourcePath: string | null;
  guideVocalPath: string | null;
  guideVocalEnabled: boolean;
  alignmentResult: AlignmentResult | null;
  alignmentFineTuneMs: number;
}

export interface SongMixerSnapshot {
  backingVolume: number;
  micGain: number;
  guideVolume: number;
  autoBalanceMixin: boolean;
  autoBalanceVocalPreset: AutoBalanceVocalPreset;
  exportNamingMode: ExportNamingMode;
}

export interface SongPracticeSnapshot {
  loopA: number | null;
  loopB: number | null;
  speed: number;
  pitchSemitones: number;
}

export interface SongSessionSnapshot extends ProjectSession {
  mixer: SongMixerSnapshot;
  practice: SongPracticeSnapshot;
}

interface LoadResult {
  duration: number;
  sample_rate: number;
  is_video: boolean;
  video_path: string | null;
  melody_source: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function sanitizeLyricsLines(value: unknown): LyricLine[] {
  if (!Array.isArray(value)) return [];
  return value.filter((line): line is LyricLine => {
    if (!isRecord(line)) return false;
    return (
      typeof line.start_ms === "number" &&
      typeof line.end_ms === "number" &&
      typeof line.text === "string"
    );
  });
}

function normalizeAutoBalanceVocalPreset(value: unknown): AutoBalanceVocalPreset {
  if (value === "natural" || value === "clear" || value === "forward") return value;
  return "forward";
}

function normalizeExportNamingMode(value: unknown): ExportNamingMode {
  return value === "manual" ? "manual" : "auto";
}

function finiteNumber(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.max(min, Math.min(max, value));
}

function sanitizeProjectSession(value: unknown): ProjectSession | null {
  if (!isRecord(value) || value.version !== PROJECT_SESSION_VERSION) return null;
  return {
    version: PROJECT_SESSION_VERSION,
    backingPath: typeof value.backingPath === "string" ? value.backingPath : null,
    lyricsFileName: typeof value.lyricsFileName === "string" ? value.lyricsFileName : "",
    lyricsLines: sanitizeLyricsLines(value.lyricsLines),
    melody:
      isRecord(value.melody) && Array.isArray(value.melody.notes)
        ? (value.melody as MelodyTrack)
        : null,
    melodySourcePath:
      typeof value.melodySourcePath === "string" ? value.melodySourcePath : null,
    guideVocalPath:
      typeof value.guideVocalPath === "string" ? value.guideVocalPath : null,
    guideVocalEnabled: value.guideVocalEnabled === true,
    alignmentResult: isRecord(value.alignmentResult)
      ? (value.alignmentResult as AlignmentResult)
      : null,
    alignmentFineTuneMs: finiteNumber(value.alignmentFineTuneMs, 0, -600_000, 600_000),
  };
}

export function sanitizeSongSession(value: unknown): SongSessionSnapshot | null {
  const project = sanitizeProjectSession(value);
  if (!project || !isRecord(value)) return null;
  const mixer = isRecord(value.mixer) ? value.mixer : {};
  const practice = isRecord(value.practice) ? value.practice : {};

  return {
    ...project,
    mixer: {
      backingVolume: finiteNumber(mixer.backingVolume, get(backingVolume), 0, 1),
      micGain: finiteNumber(mixer.micGain, get(micGain), 0, 3),
      guideVolume: finiteNumber(mixer.guideVolume, get(guideVolume), 0, 1),
      autoBalanceMixin:
        typeof mixer.autoBalanceMixin === "boolean"
          ? mixer.autoBalanceMixin
          : get(autoBalanceMixin),
      autoBalanceVocalPreset: normalizeAutoBalanceVocalPreset(
        mixer.autoBalanceVocalPreset,
      ),
      exportNamingMode: normalizeExportNamingMode(mixer.exportNamingMode),
    },
    practice: {
      loopA: typeof practice.loopA === "number" ? practice.loopA : null,
      loopB: typeof practice.loopB === "number" ? practice.loopB : null,
      speed: finiteNumber(practice.speed, get(speed), 0.25, 4),
      pitchSemitones: Math.round(
        finiteNumber(practice.pitchSemitones, get(pitchSemitones), -7, 7),
      ),
    },
  };
}

export function createProjectSessionSnapshot(): ProjectSession {
  return {
    version: PROJECT_SESSION_VERSION,
    backingPath: get(loadedMedia)?.file_path ?? null,
    lyricsFileName: get(lyricsFileName),
    lyricsLines: get(lyricsLines),
    melody: get(currentMelody),
    melodySourcePath: get(melodySourcePath),
    guideVocalPath: get(guideVocalPath),
    guideVocalEnabled: get(guideVocalEnabled),
    alignmentResult: get(alignmentResult),
    alignmentFineTuneMs: get(alignmentFineTuneMs),
  };
}

export function createSongSessionSnapshot(): SongSessionSnapshot {
  return {
    ...createProjectSessionSnapshot(),
    mixer: {
      backingVolume: get(backingVolume),
      micGain: get(micGain),
      guideVolume: get(guideVolume),
      autoBalanceMixin: get(autoBalanceMixin),
      autoBalanceVocalPreset: get(autoBalanceVocalPreset),
      exportNamingMode: get(exportNamingMode),
    },
    practice: {
      loopA: get(loopA),
      loopB: get(loopB),
      speed: get(speed),
      pitchSemitones: get(pitchSemitones),
    },
  };
}

export function suggestSongTitle(): string {
  const mediaName = get(loadedMedia)?.file_name;
  if (!mediaName) return "";
  return mediaName.replace(/\.[^.]+$/, "").trim();
}

function refreshBackingPitchFromCurrentMelody(): void {
  const melody = get(currentMelody);
  if (!melody) {
    backingPitchTrack.set(null);
    return;
  }
  const offsetSecs = finalOffsetSecs(get(alignmentResult), get(alignmentFineTuneMs));
  backingPitchTrack.set(melodyToPitchTrack(applyAlignmentToMelody(melody, offsetSecs)));
}

async function clearRuntimeMediaState(): Promise<void> {
  await invoke("pause_playback").catch(() => {});
  await invoke("clear_recording").catch(() => {});
  await invoke("clear_backing").catch(() => {});
  await invoke("clear_guide_vocal").catch(() => {});
  loadedMedia.set(null);
  resetBackingState();
  resetMelodyState();
  await clearLoop();
  hasRecording.set(false);
  clearLiveVocalSamples();
}

async function loadBackingPath(path: string): Promise<void> {
  const result = await invoke<LoadResult>("load_backing", { path });
  loadedMedia.set({
    file_path: path,
    file_name: basename(path),
    duration: result.duration,
    sample_rate: result.sample_rate,
    is_video: result.is_video,
    video_path: result.video_path,
    video_url: result.video_path ? convertFileSrc(result.video_path) : null,
  });
}

async function applyPracticeSnapshot(practice: SongPracticeSnapshot): Promise<void> {
  await setSpeed(practice.speed);
  await setPitchSemitones(practice.pitchSemitones);
  if (
    typeof practice.loopA === "number" &&
    typeof practice.loopB === "number" &&
    practice.loopB > practice.loopA
  ) {
    await setLoopRange(practice.loopA, practice.loopB);
  } else {
    await clearLoop();
  }
}

export async function applySongSessionSnapshot(snapshot: SongSessionSnapshot): Promise<void> {
  await clearRuntimeMediaState();

  lyricsFileName.set(snapshot.lyricsFileName);
  lyricsLines.set(snapshot.lyricsLines);

  if (snapshot.backingPath) {
    await loadBackingPath(snapshot.backingPath);
  }

  currentMelody.set(snapshot.melody);
  melodySourcePath.set(snapshot.melodySourcePath);
  alignmentResult.set(snapshot.alignmentResult);
  alignmentFineTuneMs.set(snapshot.alignmentFineTuneMs);
  refreshBackingPitchFromCurrentMelody();

  backingVolume.set(snapshot.mixer.backingVolume);
  micGain.set(snapshot.mixer.micGain);
  guideVolume.set(snapshot.mixer.guideVolume);
  autoBalanceMixin.set(snapshot.mixer.autoBalanceMixin);
  autoBalanceVocalPreset.set(snapshot.mixer.autoBalanceVocalPreset);
  exportNamingMode.set(snapshot.mixer.exportNamingMode);

  if (snapshot.guideVocalPath) {
    const offsetSecs = finalOffsetSecs(snapshot.alignmentResult, snapshot.alignmentFineTuneMs);
    await invoke("load_guide_vocal", {
      path: snapshot.guideVocalPath,
      offsetSecs,
    });
    guideVocalPath.set(snapshot.guideVocalPath);
    guideVocalEnabled.set(snapshot.guideVocalEnabled);
    await invoke("set_guide_vocal_enabled", { enabled: snapshot.guideVocalEnabled }).catch(
      () => {},
    );
  } else {
    guideVocalPath.set(null);
    guideVocalEnabled.set(false);
  }

  await applyPracticeSnapshot(snapshot.practice);
}
