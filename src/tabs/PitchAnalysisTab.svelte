<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import PitchTimeline from "../components/PitchTimeline.svelte";
  import {
    currentMelody,
    melodyStatus,
    alignmentResult,
  } from "../stores/melody";
  import {
    elapsed,
    duration,
    transportState,
  } from "../stores/transport";
  import { outputDeviceIndex, latencyMs } from "../stores/settings";
  import { t, tSync } from "../i18n";

  /** 是否正在播放（從 transport state 推導） */
  let isPlaying = $derived($transportState === "playing_back");

  /** 使用者當前拖曳的 slider 值（拖曳中暫存，放開才 invoke seek）*/
  let draggingValue = $state<number | null>(null);

  /** slider 顯示值：拖曳中用暫存值，否則跟 elapsed */
  let sliderValue = $derived(draggingValue ?? $elapsed);

  /**
   * Slider 的最大值：優先用 backing 的 duration，
   * 若沒 backing 但有 melody 就用 melody 的總長度（允許純 melody 視覺回放）
   */
  let sliderMax = $derived.by(() => {
    if ($duration > 0) return $duration;
    return $currentMelody?.total_duration_secs ?? 0;
  });

  /** Slider 是否可用：有 backing 或有 melody 任一即可 */
  let sliderEnabled = $derived(sliderMax > 0);

  /** 播放按鈕是否可用：需要真的有 backing（因為播放走後端音訊） */
  let playEnabled = $derived($duration > 0);

  function fmtTime(sec: number): string {
    if (!Number.isFinite(sec) || sec < 0) return "00:00";
    const totalSec = Math.floor(sec);
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  }

  async function togglePlayback(): Promise<void> {
    try {
      if (isPlaying) {
        await invoke("pause_playback");
      } else {
        await invoke("start_playback", {
          startFrame: null,
          outputDevice: $outputDeviceIndex,
          latencyMs: $latencyMs,
        });
      }
    } catch (err) {
      // 忽略錯誤（可能尚未載入 backing）
    }
  }

  function onSliderInput(e: Event): void {
    const val = parseFloat((e.target as HTMLInputElement).value);
    draggingValue = val;
    // 拖曳時也即時更新 elapsed，讓 PitchTimeline 視窗跟著滑動
    // （這樣不論有沒有 backing，拖曳就能即時預覽音高線）
    elapsed.set(val);
  }

  async function onSliderChange(e: Event): Promise<void> {
    const val = parseFloat((e.target as HTMLInputElement).value);
    draggingValue = null;
    elapsed.set(val);
    // 有 backing 時才呼叫後端 seek（讓實際音訊位置跟上）
    if ($duration > 0) {
      try {
        await invoke("seek", { seconds: val });
      } catch (err) {
        // 靜默失敗
      }
    }
  }

  /** melody 狀態訊息渲染（支援 locale 切換即時刷新） */
  let melodyStatusText = $derived.by(() => {
    const translate = $t;
    const m = $melodyStatus;
    if (!m) return translate("setup.melody.status.empty");
    let mergedVars = m.vars;
    if (m.nestedVars) {
      const translated: Record<string, string | number> = { ...(m.vars ?? {}) };
      for (const [field, desc] of Object.entries(m.nestedVars)) {
        translated[field] = translate(desc.key, desc.vars);
      }
      mergedVars = translated;
    }
    const base = translate(m.key, mergedVars);
    if (m.appendKey) {
      return translate(m.appendKey, { ...(m.appendVars ?? {}), status: base });
    }
    return base;
  });
</script>

<div class="pitch-page">
  <div class="header-row">
    <h2>{$t("pitch.header.title")}</h2>
    <div class="header-meta">
      {#if $currentMelody}
        <span class="meta-text">{melodyStatusText}</span>
        {#if $alignmentResult}
          <span class="meta-sep">·</span>
          <span class="meta-text">
            {$t("pitch.header.alignment", {
              sign: $alignmentResult.offset_secs >= 0 ? "+" : "",
              offset: $alignmentResult.offset_secs.toFixed(3),
            })}
          </span>
        {/if}
      {:else}
        <span class="meta-text muted">
          {$t("pitch.header.noMelody")}
        </span>
      {/if}
    </div>
  </div>

  <div class="timeline-wrapper">
    <PitchTimeline />
  </div>

  <div class="transport-bar">
    <button
      class="play-btn"
      onclick={togglePlayback}
      disabled={!playEnabled}
      title={playEnabled
        ? isPlaying
          ? $t("pitch.transport.pause")
          : $t("pitch.transport.play")
        : $t("pitch.transport.needBacking")}
      aria-label={isPlaying ? $t("pitch.transport.pause") : $t("pitch.transport.play")}
    >
      {#if isPlaying}
        ❚❚
      {:else}
        ▶
      {/if}
    </button>

    <span class="time-label">{fmtTime(sliderValue)}</span>

    <input
      class="seek-slider"
      type="range"
      min="0"
      max={sliderMax}
      step="0.05"
      value={sliderValue}
      disabled={!sliderEnabled}
      oninput={onSliderInput}
      onchange={onSliderChange}
    />

    <span class="time-label time-total">{fmtTime(sliderMax)}</span>
  </div>
</div>

<style>
  .pitch-page {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: var(--space-xl);
    gap: var(--space-lg);
    overflow: hidden;
  }

  .header-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    flex-shrink: 0;
  }

  .header-row h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: #3d3630;
  }

  .header-meta {
    font-size: 12px;
    color: #7a7268;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
    max-width: 70%;
  }

  .meta-text {
    white-space: nowrap;
  }

  .meta-text.muted {
    color: #b0a898;
  }

  .meta-sep {
    color: #d8d2c4;
  }

  .timeline-wrapper {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: #fff;
    border-radius: 12px;
    padding: 16px;
    overflow: hidden;
  }

  .transport-bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    background: #fff;
    border-radius: 12px;
    flex-shrink: 0;
  }

  .play-btn {
    width: 40px;
    height: 40px;
    border: none;
    border-radius: 50%;
    background: #755700;
    color: #fff;
    font-size: 14px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s;
    flex-shrink: 0;
  }

  .play-btn:hover:not(:disabled) {
    background: #5c4400;
  }

  .play-btn:disabled {
    background: #e8e2d8;
    color: #a0958a;
    cursor: not-allowed;
  }

  .time-label {
    font-size: 12px;
    color: #755700;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    min-width: 42px;
    text-align: center;
    flex-shrink: 0;
  }

  .time-total {
    color: #a0958a;
  }

  .seek-slider {
    flex: 1;
    accent-color: #d35400;
    cursor: pointer;
  }

  .seek-slider:disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }
</style>
