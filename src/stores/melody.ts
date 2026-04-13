/**
 * 目標旋律 store — 統一 UltraStar / MIDI / 人聲分離三種來源的前端狀態。
 *
 * Phase 1：只實作 UltraStar 路徑。
 * Phase 2 / 3 擴充 source 變體時不需要改 store interface。
 */
import { writable } from "svelte/store";
import type { PitchTrack, PitchTrackSample } from "./pitch";

// ── 後端 Rust MelodyTrack 的 TypeScript 鏡像 ───────────────────────

export interface MelodyNote {
  start_secs: number;
  duration_secs: number;
  midi_pitch: number;
  freq_hz: number;
  lyric: string | null;
  is_golden: boolean;
  is_freestyle: boolean;
}

/**
 * Melody 來源元資料。
 *
 * 使用 serde 的 `tag = "type"` externally tagged union，
 * 所以每個變體都有 `type` discriminator。
 */
export type MelodySource =
  | {
      type: "ultra_star";
      txt_path: string;
      title: string | null;
      artist: string | null;
      bpm: number;
    }
  | {
      type: "midi";
      mid_path: string;
      track_index: number;
      track_name: string | null;
    }
  | {
      type: "vocal_separation";
      cache_path: string;
      model: string;
      file_hash: string;
    }
  | {
      type: "imported_vocals";
      vocals_path: string;
      note_count: number;
      voiced_ratio: number;
    };

export interface MelodyTrack {
  source: MelodySource;
  notes: MelodyNote[];
  total_duration_secs: number;
  /** AI 引擎（CREPE）的原始音高樣本，保留自然曲線 */
  raw_pitch_track?: PitchTrackSample[];
}

// ── Store 實例 ─────────────────────────────────────────────────────

/** 目前載入的目標旋律（null 代表沒有可用旋律） */
export const currentMelody = writable<MelodyTrack | null>(null);

/**
 * 自動偵測到的來源標籤，從 `LoadResult.melody_source` 填入。
 * 可能值：`"ultrastar"` / `"midi"` / `"uvr_cache"` / `null`
 */
export const detectedMelodySourceKind = writable<string | null>(null);

/** 旋律載入狀態的使用者可讀文字（給 SetupTab 顯示） */
export const melodyStatus = writable<string>("尚未載入目標旋律");

/** 重置與旋律相關的所有 store（載入新伴奏時呼叫） */
export function resetMelodyState(): void {
  currentMelody.set(null);
  detectedMelodySourceKind.set(null);
  melodyStatus.set("尚未載入目標旋律");
  alignmentResult.set(null);
  alignmentFineTuneMs.set(0);
  melodySourcePath.set(null);
}

// ── Phase 3-new-a：雙檔對齊狀態 ────────────────────────────────────

/**
 * 後端 `audio_aligner::AlignmentResult` 的 TypeScript 鏡像。
 *
 * `offset_secs` convention：target (練唱伴奏) 的 t=0 對應 reference (melody 來源)
 * 的哪個時間位置。套用到 melody：
 * `aligned_time = melody_time - offset_secs`
 */
export interface AlignmentResult {
  offset_secs: number;
  peak_correlation: number;
  /** 信心指標：> 5 為高信心、< 2 應警示使用者手動微調 */
  peak_to_mean_ratio: number;
  sample_rate: number;
  reference_duration_secs: number;
  target_duration_secs: number;
}

/** 自動對齊結果（null = 尚未對齊或雙檔之一尚未載入） */
export const alignmentResult = writable<AlignmentResult | null>(null);

/** 使用者手動微調的 offset (毫秒)，加到自動對齊結果之上 */
export const alignmentFineTuneMs = writable<number>(0);

/** 目前 melody 來源的檔案路徑（給對齊用，不是 UltraStar 情況也可能是 null） */
export const melodySourcePath = writable<string | null>(null);

/**
 * 計算**最終**的 offset（秒）：自動對齊值 + 使用者微調值。
 *
 * 前端在渲染 melody 時用這個值：`aligned_time = melody_time - finalOffsetSecs`
 */
export function finalOffsetSecs(
  auto: AlignmentResult | null,
  fineTuneMs: number,
): number {
  const autoSecs = auto?.offset_secs ?? 0;
  return autoSecs + fineTuneMs / 1000;
}

/**
 * 把 offset 套用到 MelodyTrack，回傳新的 MelodyTrack（immutable）。
 *
 * 實務上用於把對齊後的 melody 餵給 PitchTimeline 的 backingPitchTrack 兼容層。
 */
export function applyAlignmentToMelody(
  track: MelodyTrack,
  offsetSecs: number,
): MelodyTrack {
  if (offsetSecs === 0) return track;
  return {
    ...track,
    notes: track.notes.map((n) => ({
      ...n,
      start_secs: n.start_secs - offsetSecs,
    })),
    raw_pitch_track: track.raw_pitch_track?.map((s) => ({
      ...s,
      timestamp: s.timestamp - offsetSecs,
    })),
  };
}

/** 信心度分級（給 UI 顯示 badge） */
export function alignmentConfidence(
  result: AlignmentResult | null,
): "high" | "medium" | "low" | "none" {
  if (!result) return "none";
  if (result.peak_to_mean_ratio >= 5) return "high";
  if (result.peak_to_mean_ratio >= 2) return "medium";
  return "low";
}

// ── Helper：MelodyTrack → PitchTrack ───────────────────────────────

/**
 * 把離散音符展開成 PitchTrack 的密集樣本。
 *
 * 這是 Rust 端 `MelodyTrack::to_pitch_track` 的 TypeScript 鏡像。
 * 給 PitchTimeline 的 `drawSegmentedLine` 消費，不用改 UI 即可顯示目標旋律。
 *
 * 策略：
 * - 每個音符按 `hopSecs` 間隔產生樣本
 * - 音符之間的 rest 區段不產生樣本 → PitchTimeline 內建的
 *   「gap > 0.3s 斷段」邏輯會自然把它們斷開成獨立線段
 * - 自由音符（`is_freestyle`）不產生樣本
 * - 極短音符（< hopSecs）至少產一個樣本在起點
 *
 * @param track 來源旋律軌
 * @param hopSecs 取樣間隔秒數，0.02 = 50 Hz 取樣率，讓長音符有更密的線條
 */
export function melodyToPitchTrack(
  track: MelodyTrack,
  hopSecs = 0.02,
): PitchTrack {
  if (hopSecs <= 0) {
    throw new Error("hopSecs must be positive");
  }

  // AI 引擎的原始音高樣本：直接回傳，保留自然曲線
  if (track.raw_pitch_track && track.raw_pitch_track.length > 0) {
    return { samples: track.raw_pitch_track };
  }

  const samples: PitchTrackSample[] = [];

  for (const note of track.notes) {
    if (note.is_freestyle) continue;

    const { note: noteName, octave, cent } = midiToNoteName(note.midi_pitch);
    const end = note.start_secs + note.duration_secs;
    let t = note.start_secs;
    let emitted = false;

    while (t < end) {
      samples.push({
        timestamp: t,
        freq: note.freq_hz,
        confidence: 1.0,
        note: noteName,
        octave,
        cent,
      });
      t += hopSecs;
      emitted = true;
    }

    if (!emitted) {
      samples.push({
        timestamp: note.start_secs,
        freq: note.freq_hz,
        confidence: 1.0,
        note: noteName,
        octave,
        cent,
      });
    }
  }

  return { samples };
}

/**
 * 把 MIDI 音高轉成音名 / 八度 / cent（對應 Rust 的 freq_to_note）。
 *
 * 因為 to_pitch_track 輸出的樣本都是精確對齊 MIDI 音高，cent 永遠是 0。
 */
function midiToNoteName(midi: number): {
  note: string;
  octave: number;
  cent: number;
} {
  const noteNames = [
    "C",
    "C#",
    "D",
    "D#",
    "E",
    "F",
    "F#",
    "G",
    "G#",
    "A",
    "A#",
    "B",
  ];
  const rounded = Math.round(midi);
  const noteIdx = ((rounded % 12) + 12) % 12;
  const octave = Math.floor(rounded / 12) - 1;
  return {
    note: noteNames[noteIdx],
    octave,
    cent: 0,
  };
}
