/**
 * 監聽後端推送的事件，並同步到對應的 store。
 * 在 App.svelte 啟動時呼叫一次即可。
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  transportState,
  elapsed,
  duration,
  backingRms,
  micRms,
  type TransportState,
} from "../stores/transport";
import { latencyMs } from "../stores/settings";
import {
  currentPitch,
  backingPitchTrack,
  backingPitchQuality,
  backingPitchAnalyzing,
  freeMode,
  freeModeReason,
  liveVocalSamples,
  detectedKey,
  keyDetectionStatus,
  type BackingPitchQuality,
  type PitchSample,
  type PitchTrack,
  type KeyResult,
} from "../stores/pitch";
import { calibrationStatus } from "../stores/settings";
import { showToast } from "../stores/toast";
import { get } from "svelte/store";

interface CalibrationStartedPayload {
  bpm: number;
  warmup_beats: number;
  measurement_beats: number;
  prep_ms: number;
  beat_interval_ms: number;
}

interface CalibrationBeatPayload {
  beat_idx: number;
  is_warmup: boolean;
  detected: boolean;
  accepted: boolean;
  offset_ms: number;
}

interface CalibrationCompletePayload {
  latency_ms: number;
  valid_beats: number;
  measurement_beats: number;
  std_dev_ms: number;
}

interface CalibrationFailedPayload {
  reason: string;
}

interface BackingPitchAnalyzingPayload {
  duration: number;
}

interface BackingPitchNotDetectedPayload {
  voiced_ratio: number;
  mean_confidence: number;
  elapsed_secs: number;
  reason: string;
}

let unlisteners: UnlistenFn[] = [];

export async function setupEventListeners(): Promise<void> {
  // 清除舊的
  await teardownEventListeners();

  unlisteners.push(
    await listen<{ elapsed: number; duration: number }>(
      "audio:progress",
      (e) => {
        elapsed.set(e.payload.elapsed);
        duration.set(e.payload.duration);
      },
    ),
  );

  unlisteners.push(
    await listen<{ backing_rms: number; mic_rms: number }>(
      "audio:rms",
      (e) => {
        backingRms.set(e.payload.backing_rms);
        micRms.set(e.payload.mic_rms);
      },
    ),
  );

  unlisteners.push(
    await listen<{ state: string }>("audio:state_changed", (e) => {
      transportState.set(e.payload.state as TransportState);
    }),
  );

  unlisteners.push(
    await listen<PitchSample | null>("audio:pitch", (e) => {
      currentPitch.set(e.payload);
      // 同步累積即時人聲樣本（給 PitchTimeline 畫線用）
      // 套用 latency compensation：麥克風訊號的實際演唱時刻 = 播放位置 - 延遲
      // 使用就地 push + shift 避免每次都建立新陣列造成 GC 壓力
      if (e.payload) {
        const latencyOffset = get(latencyMs) / 1000;
        const t = get(elapsed) - latencyOffset;
        liveVocalSamples.update((arr) => {
          if (arr.length >= 600) {
            arr.shift();
          }
          arr.push({
            timestamp: t,
            freq: e.payload!.freq,
            confidence: e.payload!.confidence,
            note: e.payload!.note,
            octave: e.payload!.octave,
            cent: e.payload!.cent,
          });
          return arr;
        });
      }
    }),
  );

  // 伴奏旋律分析啟動 → 顯示「分析中」橫幅
  unlisteners.push(
    await listen<BackingPitchAnalyzingPayload>(
      "backing_pitch:analyzing",
      (e) => {
        backingPitchAnalyzing.set({ duration: e.payload.duration });
        // 進入分析狀態時暫時清掉舊的灰藍線與自由模式狀態
        backingPitchTrack.set(null);
        freeMode.set(false);
        freeModeReason.set("");
      },
    ),
  );

  // 伴奏旋律分析完成（品質可靠）→ 主動拉取軌跡，並關閉自由模式
  unlisteners.push(
    await listen<BackingPitchQuality>(
      "backing_pitch:ready",
      async (e) => {
        backingPitchAnalyzing.set(null);
        try {
          const track = await invoke<PitchTrack | null>(
            "get_backing_pitch_track",
          );
          if (track) {
            backingPitchTrack.set(track);
            backingPitchQuality.set(e.payload);
            freeMode.set(false);
            freeModeReason.set("");
          }
        } catch (err) {
          // 伴奏旋律拉取失敗時優雅降級為自由模式
          console.warn("[backing_pitch] 旋律軌跡載入失敗，降級為自由模式", err);
          backingPitchTrack.set(null);
          freeMode.set(true);
          freeModeReason.set("旋律軌跡載入失敗，已切換為自由模式");
        }
      },
    ),
  );

  // 伴奏無主旋律 → 切換自由模式（隱藏目標旋律線、僅顯示人聲）
  unlisteners.push(
    await listen<BackingPitchNotDetectedPayload>(
      "backing_pitch:not_detected",
      (e) => {
        backingPitchAnalyzing.set(null);
        backingPitchTrack.set(null);
        backingPitchQuality.set({
          total_frames: 0,
          voiced_frames: 0,
          voiced_ratio: e.payload.voiced_ratio,
          mean_confidence: e.payload.mean_confidence,
          elapsed_secs: e.payload.elapsed_secs,
        });
        freeMode.set(true);
        freeModeReason.set(e.payload.reason);
      },
    ),
  );

  unlisteners.push(
    await listen("audio:finished", async () => {
      transportState.set("idle");

      // 錄音結束後自動偵測調性：拉取完整 pitch track → 偵測
      try {
        const track = await invoke<PitchTrack | null>("get_pitch_track");
        if (track && track.samples.length >= 30) {
          keyDetectionStatus.set("detecting");
          const result = await invoke<KeyResult | null>(
            "detect_key_from_pitch_track",
            { track },
          );
          detectedKey.set(result);
          keyDetectionStatus.set(result ? "done" : "idle");
        }
      } catch (err) {
        console.warn("[key_detection] 調性偵測失敗", err);
      }
    }),
  );

  unlisteners.push(
    await listen<{ message: string }>("audio:error", (e) => {
      console.error("[audio:error]", e.payload.message);
      showToast(e.payload.message, "error", 5000);
      transportState.set("idle");
    }),
  );

  // ── 校準事件 ──────────────────────────────────────────────────
  // 流程：started → beat_detected (×N) → complete | failed
  // started 在 Rust thread::spawn 之前 emit，是視覺時間軸的真正起點
  unlisteners.push(
    await listen<CalibrationStartedPayload>("calibration:started", (e) => {
      calibrationStatus.update((s) => ({
        ...s,
        isRunning: true,
        bpm: e.payload.bpm,
        warmupBeats: e.payload.warmup_beats,
        measurementBeats: e.payload.measurement_beats,
        prepMs: e.payload.prep_ms,
        beatIntervalMs: e.payload.beat_interval_ms,
        beats: [],
        finalLatencyMs: null,
        stdDevMs: null,
        error: null,
      }));
    }),
  );

  unlisteners.push(
    await listen<CalibrationBeatPayload>("calibration:beat_detected", (e) => {
      calibrationStatus.update((s) => {
        const beats = [...s.beats];
        // 用 beat_idx 對應位置插入或覆寫
        const existingIdx = beats.findIndex(
          (b) => b.beatIdx === e.payload.beat_idx,
        );
        const next = {
          beatIdx: e.payload.beat_idx,
          isWarmup: e.payload.is_warmup,
          detected: e.payload.detected,
          accepted: e.payload.accepted,
          offsetMs: e.payload.offset_ms,
        };
        if (existingIdx >= 0) {
          beats[existingIdx] = next;
        } else {
          beats.push(next);
          beats.sort((a, b) => a.beatIdx - b.beatIdx);
        }
        return { ...s, beats };
      });
    }),
  );

  unlisteners.push(
    await listen<CalibrationCompletePayload>("calibration:complete", (e) => {
      calibrationStatus.update((s) => ({
        ...s,
        isRunning: false,
        finalLatencyMs: e.payload.latency_ms,
        stdDevMs: e.payload.std_dev_ms,
        error: null,
      }));
    }),
  );

  unlisteners.push(
    await listen<CalibrationFailedPayload>("calibration:failed", (e) => {
      calibrationStatus.update((s) => ({
        ...s,
        isRunning: false,
        finalLatencyMs: null,
        error: e.payload.reason,
      }));
    }),
  );

}

export async function teardownEventListeners(): Promise<void> {
  for (const fn of unlisteners) {
    fn();
  }
  unlisteners = [];
}
