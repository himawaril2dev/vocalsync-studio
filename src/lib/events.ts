import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { get } from "svelte/store";
import {
  backingRms,
  duration,
  elapsed,
  micRms,
  pausedAtElapsed,
  pausedResumeMode,
  transportState,
  type TransportState,
} from "../stores/transport";
import {
  backingPitchAnalyzing,
  backingPitchQuality,
  backingPitchTrack,
  currentPitch,
  freeMode,
  freeModeReason,
  liveVocalSamples,
  type BackingPitchQuality,
  type PitchSample,
  type PitchTrack,
} from "../stores/pitch";
import { showToast } from "../stores/toast";

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
  await teardownEventListeners();

  unlisteners.push(
    await listen<{ elapsed: number; duration: number }>("audio:progress", (e) => {
      elapsed.set(e.payload.elapsed);
      duration.set(e.payload.duration);
    }),
  );

  unlisteners.push(
    await listen<{ backing_rms: number; mic_rms: number }>("audio:rms", (e) => {
      backingRms.set(e.payload.backing_rms);
      micRms.set(e.payload.mic_rms);
    }),
  );

  unlisteners.push(
    await listen<{ state: string }>("audio:state_changed", (e) => {
      const state = e.payload.state;
      if (state === "idle") return;
      transportState.set(state as TransportState);
    }),
  );

  unlisteners.push(
    await listen<PitchSample | null>("audio:pitch", (e) => {
      currentPitch.set(e.payload);
      if (!e.payload) return;

      const timestamp = get(elapsed);
      liveVocalSamples.update((samples) => {
        if (samples.length >= 600) {
          samples.shift();
        }
        samples.push({
          timestamp,
          freq: e.payload!.freq,
          confidence: e.payload!.confidence,
          note: e.payload!.note,
          octave: e.payload!.octave,
          cent: e.payload!.cent,
        });
        return samples;
      });
    }),
  );

  unlisteners.push(
    await listen<BackingPitchAnalyzingPayload>("backing_pitch:analyzing", (e) => {
      backingPitchAnalyzing.set({ duration: e.payload.duration });
      backingPitchTrack.set(null);
      freeMode.set(false);
      freeModeReason.set(null);
    }),
  );

  unlisteners.push(
    await listen<BackingPitchQuality>("backing_pitch:ready", async (e) => {
      backingPitchAnalyzing.set(null);
      try {
        const track = await invoke<PitchTrack | null>("get_backing_pitch_track");
        if (track) {
          backingPitchTrack.set(track);
          backingPitchQuality.set(e.payload);
          freeMode.set(false);
          freeModeReason.set(null);
        }
      } catch (err) {
        console.warn("[backing_pitch] failed to load pitch track:", err);
        backingPitchTrack.set(null);
        freeMode.set(true);
        freeModeReason.set({
          kind: "i18n",
          key: "pitchTimeline.banner.freeMode.loadFailed",
        });
      }
    }),
  );

  unlisteners.push(
    await listen<BackingPitchNotDetectedPayload>("backing_pitch:not_detected", (e) => {
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
      freeModeReason.set({ kind: "text", text: e.payload.reason });
    }),
  );

  unlisteners.push(
    await listen("audio:finished", () => {
      if (get(transportState) === "paused") return;
      transportState.set("idle");
      pausedResumeMode.set(null);
      pausedAtElapsed.set(null);
    }),
  );

  unlisteners.push(
    await listen<{ message: string }>("audio:error", (e) => {
      console.error("[audio:error]", e.payload.message);
      showToast(e.payload.message, "error", 5000);
      transportState.set("idle");
      pausedResumeMode.set(null);
      pausedAtElapsed.set(null);
    }),
  );
}

export async function teardownEventListeners(): Promise<void> {
  for (const fn of unlisteners) {
    fn();
  }
  unlisteners = [];
}
