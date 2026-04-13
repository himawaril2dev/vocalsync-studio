<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import {
    transportState,
    elapsed,
    duration,
    backingRms,
    micRms,
    hasRecording,
    loopA,
    loopB,
    loopActive,
    setLoopA,
    setLoopB,
    clearLoop,
    speed,
    pitchSemitones,
    hasSpeedOrPitch,
    setSpeed,
    setPitchSemitones,
    resetSpeedAndPitch,
  } from "../stores/transport";
  import { loadedMedia } from "../stores/media";
  import { clearLiveVocalSamples } from "../stores/pitch";
  import { inputDeviceIndex, outputDeviceIndex, latencyMs, backingVolume, micGain, autoBalanceMixin } from "../stores/settings";
  import { detectedKey, keyDetectionStatus } from "../stores/pitch";
  import { showToast } from "../stores/toast";
  import Icon from "../components/Icon.svelte";
  import LyricsPanel from "../components/LyricsPanel.svelte";
  import LyricsSyncEditor from "../components/LyricsSyncEditor.svelte";
  import PitchTimeline from "../components/PitchTimeline.svelte";

  type PanelView = "lyrics" | "sync";

  // 已遷移至全域 toast 系統
  let videoEl = $state<HTMLVideoElement | null>(null);

  /** 右上面板顯示：歌詞 or 同步編輯器 */
  let panelView = $state<PanelView>("lyrics");

  // ── 水平分割（影片 | 歌詞）──
  let splitContainer = $state<HTMLDivElement | null>(null);
  let videoFlex = $state(0.5);
  let isDraggingH = $state(false);

  function startDragH(e: PointerEvent) {
    isDraggingH = true;
    e.preventDefault();
  }

  // ── 垂直分割（上方 | 音高）──
  let mainContainer = $state<HTMLDivElement | null>(null);
  let topFlex = $state(0.7);
  let isDraggingV = $state(false);

  function startDragV(e: PointerEvent) {
    isDraggingV = true;
    e.preventDefault();
  }

  function onPointerMove(e: PointerEvent) {
    if (isDraggingH && splitContainer) {
      const rect = splitContainer.getBoundingClientRect();
      const x = e.clientX - rect.left;
      videoFlex = Math.max(0.15, Math.min(0.75, x / rect.width));
    }
    if (isDraggingV && mainContainer) {
      const rect = mainContainer.getBoundingClientRect();
      const y = e.clientY - rect.top;
      topFlex = Math.max(0.2, Math.min(0.9, y / rect.height));
    }
  }

  function onPointerUp() {
    isDraggingH = false;
    isDraggingV = false;
  }

  // ── 影片同步邏輯：跟隨後端音訊時鐘 ──
  // 後端 ~20Hz 推送 elapsed → 比對 video.currentTime
  // 偏差 > 200ms 強制 seek，否則微調 playbackRate 讓畫面緩慢追趕
  $effect(() => {
    const targetTime = $elapsed;
    if (!videoEl || !$loadedMedia?.is_video) return;
    if ($transportState === "idle") {
      videoEl.pause();
      return;
    }

    // 確保影片在播
    if (videoEl.paused) {
      videoEl.play().catch(() => {});
    }

    const diff = targetTime - videoEl.currentTime;
    if (Math.abs(diff) > 0.2) {
      // 偏差太大，強制對齊
      videoEl.currentTime = targetTime;
      videoEl.playbackRate = 1.0;
    } else if (Math.abs(diff) > 0.05) {
      // 微幅偏差，調整 playbackRate 緩慢追趕
      videoEl.playbackRate = diff > 0 ? 1.05 : 0.95;
    } else {
      videoEl.playbackRate = 1.0;
    }
  });

  // 音量改變時即時更新後端
  $effect(() => {
    invoke("set_volume", { backing: $backingVolume, mic: $micGain }).catch(() => {});
  });

  function onSeekChange(e: Event) {
    const val = parseFloat((e.target as HTMLInputElement).value);
    invoke("seek", { seconds: val });
  }

  function fmtTime(sec: number): string {
    const s = Math.max(0, Math.floor(sec));
    const m = Math.floor(s / 60);
    return `${m.toString().padStart(2, "0")}:${(s % 60).toString().padStart(2, "0")}`;
  }

  function resetVideo() {
    if (videoEl) {
      videoEl.currentTime = 0;
      videoEl.pause();
    }
  }

  async function startPreview() {
    resetVideo();
    try {
      await invoke("start_preview", {
        startFrame: null,
        outputDevice: $outputDeviceIndex,
        inputDevice: $inputDeviceIndex,
      });
    } catch (e) {
      showToast(`試聽失敗：${e}`, "error");
    }
  }

  async function startPlayback() {
    resetVideo();
    try {
      await invoke("start_playback", { startFrame: null, outputDevice: $outputDeviceIndex, latencyMs: $latencyMs });
    } catch (e) {
      showToast(`回放失敗：${e}`, "error");
    }
  }

  async function pausePlayback() {
    try {
      await invoke("pause_playback");
      videoEl?.pause();
    } catch (e) {
      showToast(`暫停失敗：${e}`, "error");
    }
  }

  async function startRecording() {
    resetVideo();
    clearLiveVocalSamples();
    try {
      await invoke("start_recording", { inputDevice: $inputDeviceIndex, outputDevice: $outputDeviceIndex });
      hasRecording.set(true);
    } catch (e) {
      showToast(`錄音失敗：${e}`, "error");
    }
  }

  async function stopAll() {
    try {
      await invoke("stop_recording");
      if (videoEl) videoEl.pause();
    } catch (e) {
      showToast(`停止失敗：${e}`, "error");
    }
  }

  async function exportAudio() {
    try {
      const filePath = await save({
        title: "選擇導出位置",
        filters: [{ name: "WAV", extensions: ["wav"] }],
        defaultPath: "vocalsync_recording",
      });
      if (!filePath) return;

      const slashIdx = Math.max(
        filePath.lastIndexOf("\\"),
        filePath.lastIndexOf("/"),
      );
      const dir = filePath.substring(0, slashIdx);
      let prefix = filePath.substring(slashIdx + 1);
      if (prefix.endsWith(".wav")) prefix = prefix.slice(0, -4);

      const result = await invoke<{ vocal_path: string; mix_path: string }>(
        "export_audio",
        {
          dir,
          prefix,
          autoBalance: $autoBalanceMixin,
          latencyMs: $latencyMs,
        },
      );
      showToast(`導出成功\n人聲：${result.vocal_path}\n混音：${result.mix_path}`, "success", 5000);
    } catch (e) {
      showToast(`導出失敗：${e}`, "error");
    }
  }

  const transportStateLabel: Record<import("../stores/transport").TransportState, string> = {
    idle: "待命",
    previewing: "試聽中",
    recording: "錄音中",
    playing_back: "回放中",
  };

  function rmsToWidth(rms: number, boost: number = 3.0): number {
    return Math.min(100, rms * boost * 100);
  }

  function handleGlobalKeydown(e: KeyboardEvent) {
    // 忽略在 input/textarea 中的按鍵
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

    switch (e.code) {
      case "Space":
        e.preventDefault();
        if ($transportState === "previewing" || $transportState === "playing_back") {
          pausePlayback();
        } else if ($transportState === "idle" && $loadedMedia) {
          startPreview();
        }
        break;
      case "KeyR":
        if (!e.ctrlKey && !e.metaKey && $transportState === "idle" && $loadedMedia) {
          e.preventDefault();
          startRecording();
        }
        break;
      case "KeyA":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          setLoopA();
        }
        break;
      case "KeyB":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          setLoopB();
        }
        break;
      case "Equal": // + key
        e.preventDefault();
        setPitchSemitones($pitchSemitones + 1);
        break;
      case "Minus": // - key
        e.preventDefault();
        setPitchSemitones($pitchSemitones - 1);
        break;
      case "Escape":
        if ($transportState !== "idle") {
          e.preventDefault();
          stopAll();
        }
        break;
    }
  }
</script>

<svelte:window
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  onkeydown={handleGlobalKeydown}
/>

<div class="recording-page">
  <div class="main-panels" bind:this={mainContainer}>
    <!-- 上半：左影片 + 右歌詞 -->
    <div class="top-split" style="flex: {topFlex};" bind:this={splitContainer}>
      <div class="video-area" style="flex: {videoFlex};">
        {#if $loadedMedia?.is_video && $loadedMedia.video_url}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video
            bind:this={videoEl}
            src={$loadedMedia.video_url}
            muted
            playsinline
            preload="auto"
            class="video-el"
          ></video>
        {:else if $loadedMedia}
          <div class="video-placeholder">
            <p>純音訊模式</p>
            <p class="hint">{($loadedMedia.duration / 60).toFixed(1)} 分鐘 · {$loadedMedia.sample_rate} Hz</p>
          </div>
        {:else}
          <div class="video-placeholder">
            <p>請先載入伴奏</p>
          </div>
        {/if}
      </div>

      <div class="resizer resizer-h" onpointerdown={startDragH}></div>

      <div class="lyrics-area" style="flex: {1 - videoFlex};">
        <div class="panel-toggle">
          <button
            class="toggle-btn"
            class:active={panelView === "lyrics"}
            onclick={() => (panelView = "lyrics")}
            aria-pressed={panelView === "lyrics"}
          >
            歌詞
          </button>
          <button
            class="toggle-btn"
            class:active={panelView === "sync"}
            onclick={() => (panelView = "sync")}
            aria-pressed={panelView === "sync"}
          >
            同步
          </button>
        </div>
        <div class="panel-content">
          {#if panelView === "lyrics"}
            <LyricsPanel />
          {:else}
            <LyricsSyncEditor />
          {/if}
        </div>
      </div>
    </div>

    <!-- 上下拖曳分隔線 -->
    <div class="resizer resizer-v" onpointerdown={startDragV}></div>

    <!-- 下半：全寬音高曲線 -->
    <div class="pitch-area" style="flex: {1 - topFlex};">
      <div class="panel-content">
        <PitchTimeline />
      </div>
    </div>
  </div>

  <!-- 播放列：核心操作 -->
  <div class="transport-row">
    <div class="transport-left">
      <div class="transport-buttons">
        {#if $transportState === "playing_back" || $transportState === "previewing"}
          <button class="t-btn pause" onclick={pausePlayback} title="暫停 (Space)"><Icon name="pause" size={14} /></button>
        {:else}
          <button class="t-btn play" onclick={startPreview} disabled={!$loadedMedia} title="試聽 (Space)"><Icon name="play" size={14} /></button>
        {/if}

        {#if $transportState === "idle"}
          <button class="t-btn rec" onclick={startRecording} disabled={!$loadedMedia} title="錄音 (R)"><Icon name="record" size={12} /></button>
        {:else}
          <button class="t-btn stop" onclick={stopAll} title="停止 (Esc)"><Icon name="stop" size={12} /></button>
        {/if}
      </div>

      <div class="state-chip">
        {#if $transportState === "recording"}
          <span class="rec-dot" aria-label="錄音中"></span>
        {/if}
        <strong>{transportStateLabel[$transportState]}</strong>
        {#if $loopActive}
          <span class="loop-badge">循環</span>
        {/if}
        {#if $detectedKey}
          <span class="key-badge" title="相關係數 {$detectedKey.correlation.toFixed(3)}">{$detectedKey.key}</span>
        {:else if $keyDetectionStatus === "detecting"}
          <span class="key-detecting">偵測調性…</span>
        {/if}
      </div>
    </div>

    <div class="time-display">{fmtTime($elapsed)}</div>

    <div class="progress-bar-container">
      {#if $loopA !== null && $loopB !== null && $duration > 0}
        <div
          class="loop-region"
          style="left: {($loopA / $duration) * 100}%; width: {(($loopB - $loopA) / $duration) * 100}%;"
        ></div>
      {/if}
      <input type="range" class="progress-slider" min="0" max={$duration || 1} step="0.1" value={$elapsed} onchange={onSeekChange} />
    </div>

    <div class="time-display time-end">{fmtTime($duration)}</div>
  </div>

  <!-- 控制列：音量 + 調整 + 匯出 -->
  <div class="control-row">
    <div class="vu-compact">
      <div class="vu-item">
        <span class="vu-label">伴奏</span>
        <div class="vu-bar"><div class="vu-fill" style="width: {rmsToWidth($backingRms, 3.0)}%"></div></div>
        <input type="range" class="vol-slider" min="0" max="1" step="0.01" bind:value={$backingVolume} title="伴奏音量 {Math.round($backingVolume * 100)}%">
      </div>
      <div class="vu-item">
        <span class="vu-label">人聲</span>
        <div class="vu-bar"><div class="vu-fill" style="width: {rmsToWidth($micRms, 5.0)}%"></div></div>
        <input type="range" class="vol-slider" min="0" max="3" step="0.01" bind:value={$micGain} title="麥克風增益 {Math.round($micGain * 100)}%">
      </div>
    </div>

    <div class="divider"></div>

    <div class="sp-control">
      <span class="sp-label">速度</span>
      <input
        type="range"
        class="sp-slider"
        min="0.25"
        max="2"
        step="0.05"
        value={$speed}
        oninput={(e) => setSpeed(parseFloat((e.target as HTMLInputElement).value))}
      />
      <span class="sp-value">{$speed.toFixed(2)}x</span>
    </div>

    <div class="sp-control">
      <span class="sp-label">移調</span>
      <button class="sp-btn" onclick={() => setPitchSemitones($pitchSemitones - 1)} disabled={$pitchSemitones <= -24} title="降半音 (-)">-</button>
      <span class="sp-value pitch-val">{$pitchSemitones > 0 ? "+" : ""}{$pitchSemitones}</span>
      <button class="sp-btn" onclick={() => setPitchSemitones($pitchSemitones + 1)} disabled={$pitchSemitones >= 24} title="升半音 (+)">+</button>
    </div>
    {#if $hasSpeedOrPitch}
      <button class="t-btn sp-reset" onclick={resetSpeedAndPitch} title="重設速度與移調">重設</button>
    {/if}

    <div class="divider"></div>

    <div class="loop-group">
      <button
        class="t-btn loop-ab"
        class:loop-set={$loopA !== null}
        onclick={setLoopA}
        title={$loopA !== null ? `A: ${fmtTime($loopA)}` : "設定 A 點 (A)"}
      >A</button>
      <button
        class="t-btn loop-ab"
        class:loop-set={$loopB !== null}
        onclick={async () => {
          const ok = await setLoopB();
          if (!ok) showToast("B 點必須在 A 點之後", "warning");
        }}
        disabled={$loopA === null}
        title={$loopB !== null ? `B: ${fmtTime($loopB)}` : "設定 B 點 (B)"}
      >B</button>
      {#if $loopActive}
        <button class="t-btn loop-clear" onclick={clearLoop} title="清除循環">✕</button>
      {/if}
    </div>

    <div class="divider"></div>

    <button
      class="t-btn export"
      onclick={exportAudio}
      disabled={$transportState !== "idle" || !$hasRecording}
    >
      <Icon name="download" size={14} /> 導出
    </button>

    <button
      class="t-btn playback"
      onclick={startPlayback}
      disabled={!$hasRecording || !$loadedMedia || $transportState !== "idle"}
      title="回放錄音"
    >
      回放
    </button>

    <label class="auto-balance-label" title="匯出時自動平衡音量">
      <input type="checkbox" bind:checked={$autoBalanceMixin} />
      標準化
    </label>
  </div>

</div>

<style>
  .recording-page {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: var(--space-lg) var(--space-xl) var(--space-sm);
    gap: var(--space-sm);
  }

  /* ── 主面板容器（垂直分割） ── */
  .main-panels {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    user-select: none;
  }

  /* ── 上半：影片 + 歌詞 (水平分割) ── */
  .top-split {
    display: flex;
    flex-direction: row;
    min-height: 0;
  }

  .video-area {
    min-width: 0;
    background: #1a1714;
    border-radius: var(--radius-xl);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }

  /* ── 共用 resizer ── */
  .resizer {
    background-color: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    transition: background-color var(--transition-normal);
    flex-shrink: 0;
  }

  .resizer:hover, .resizer:active {
    background-color: var(--color-bg-active);
  }

  .resizer-h {
    width: 12px;
    margin: 0 var(--space-xs);
    cursor: ew-resize;
  }

  .resizer-h::after {
    content: "";
    width: 4px;
    height: 40px;
    background-color: var(--color-border-light);
    border-radius: 2px;
  }

  .resizer-v {
    height: 12px;
    margin: var(--space-xs) 0;
    cursor: ns-resize;
  }

  .resizer-v::after {
    content: "";
    width: 40px;
    height: 4px;
    background-color: var(--color-border-light);
    border-radius: 2px;
  }

  .lyrics-area {
    min-width: 0;
    overflow: hidden;
    border-radius: var(--radius-xl);
    background: var(--color-bg-surface);
    display: flex;
    flex-direction: column;
  }

  /* ── 下半：全寬音高 ── */
  .pitch-area {
    min-height: 80px;
    overflow: hidden;
    border-radius: var(--radius-xl);
    background: var(--color-bg-surface);
    display: flex;
    flex-direction: column;
  }

  .panel-toggle {
    display: flex;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md) 0 var(--space-md);
    flex-shrink: 0;
  }

  .toggle-btn {
    padding: var(--space-sm) 18px;
    border: none;
    background: var(--color-bg-hover);
    color: var(--color-text-secondary);
    font-size: 13px;
    font-weight: 500;
    border-radius: var(--radius-md) var(--radius-md) 0 0;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .toggle-btn:hover {
    background: var(--color-bg-active);
    color: var(--color-text);
  }

  .toggle-btn.active {
    background: var(--color-brand);
    color: #fff;
    font-weight: 600;
  }

  .panel-content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: var(--space-sm) var(--space-md) var(--space-md) var(--space-md);
    overflow: hidden;
  }

  .video-placeholder {
    text-align: center;
    color: var(--color-text-secondary);
  }

  .video-placeholder .hint {
    font-size: 12px;
    margin-top: var(--space-xs);
    color: var(--color-text-muted);
  }

  .video-el {
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: #000;
  }

  /* ── 播放列 ── */
  .transport-row {
    background: var(--color-bg-surface);
    border-radius: var(--radius-lg);
    padding: var(--space-sm) var(--space-lg);
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }

  .transport-left {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    flex-shrink: 0;
  }

  .transport-buttons {
    display: flex;
    gap: var(--space-xs);
  }

  .state-chip {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    font-size: 12px;
    color: var(--color-text-secondary);
    white-space: nowrap;
  }

  .time-display {
    font-family: var(--font-mono);
    font-size: 14px;
    font-weight: 600;
    color: var(--color-text);
    flex-shrink: 0;
    min-width: 42px;
    text-align: center;
  }

  .time-end {
    color: var(--color-text-muted);
  }

  .progress-bar-container {
    flex: 1;
    min-width: 120px;
    display: flex;
    align-items: center;
    position: relative;
  }

  .loop-region {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    height: 18px;
    background: rgba(37, 99, 235, 0.15);
    border-left: 2px solid var(--color-info);
    border-right: 2px solid var(--color-info);
    border-radius: 3px;
    pointer-events: none;
    z-index: 1;
  }

  .progress-slider {
    width: 100%;
    accent-color: var(--color-brand);
    cursor: pointer;
  }

  /* ── 控制列 ── */
  .control-row {
    background: var(--color-bg-surface);
    border-radius: var(--radius-lg);
    padding: var(--space-sm) var(--space-lg);
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }

  .divider {
    width: 1px;
    height: 20px;
    background: var(--color-border);
    flex-shrink: 0;
  }

  .vu-compact {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 180px;
  }

  .vu-item {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .vu-label {
    width: 30px;
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .vu-bar {
    width: 60px;
    height: 6px;
    background: var(--color-bg-hover);
    border-radius: 3px;
    overflow: hidden;
    flex-shrink: 0;
  }

  .vu-fill {
    height: 100%;
    background: linear-gradient(90deg, var(--color-success) 0%, var(--color-success) 60%, #f9a825 80%, #d50000 100%);
    transition: width 0.05s linear;
  }

  .vol-slider {
    width: 60px;
    accent-color: var(--color-brand);
  }

  /* ── 共用按鈕 ── */
  .t-btn {
    padding: var(--space-sm) var(--space-md);
    border: none;
    border-radius: var(--radius-md);
    font-size: 13px;
    cursor: pointer;
    background: var(--color-bg-hover);
    color: var(--color-text);
    transition: all var(--transition-fast);
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    white-space: nowrap;
  }

  .t-btn:hover:not(:disabled) {
    background: var(--color-bg-active);
  }

  .t-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .t-btn.play {
    background: var(--color-brand);
    color: #fff;
    width: 36px;
    height: 36px;
    padding: 0;
    justify-content: center;
    border-radius: 50%;
  }

  .t-btn.play:hover:not(:disabled) {
    background: var(--color-brand-hover);
  }

  .t-btn.pause {
    background: var(--color-brand);
    color: #fff;
    width: 36px;
    height: 36px;
    padding: 0;
    justify-content: center;
    border-radius: 50%;
  }

  .t-btn.pause:hover {
    background: var(--color-brand-dark);
  }

  .t-btn.rec {
    background: var(--color-danger);
    color: #fff;
    width: 32px;
    height: 32px;
    padding: 0;
    justify-content: center;
    border-radius: 50%;
  }

  .t-btn.rec:hover:not(:disabled) {
    background: var(--color-danger-hover);
  }

  .t-btn.stop {
    background: var(--color-brand-dark);
    color: #fff;
    width: 32px;
    height: 32px;
    padding: 0;
    justify-content: center;
    border-radius: 50%;
  }

  .t-btn.stop:hover {
    background: var(--color-brand);
  }

  .t-btn.export {
    background: var(--color-accent);
    color: #3d2b00;
  }

  .t-btn.export:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .t-btn.playback {
    font-size: 12px;
  }

  /* ── A-B 循環 ── */
  .loop-group {
    display: flex;
    gap: 2px;
    align-items: center;
  }

  .loop-ab {
    min-width: 28px;
    font-size: 12px;
    font-weight: 700;
    padding: var(--space-xs) var(--space-sm);
    letter-spacing: 0.5px;
  }

  .loop-ab.loop-set {
    background: var(--color-info-bg);
    color: var(--color-info);
    border: 1px solid #93b4f5;
  }

  .loop-clear {
    font-size: 11px;
    min-width: 24px;
    padding: var(--space-xs) var(--space-sm);
    background: var(--color-warning-bg);
    color: var(--color-warning-text);
  }

  .loop-clear:hover {
    background: #fde68a;
  }

  .loop-badge {
    display: inline-block;
    background: var(--color-info-bg);
    color: #1d4ed8;
    font-size: 10px;
    font-weight: 600;
    padding: 1px var(--space-xs);
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }

  .key-badge {
    display: inline-block;
    background: #e8f4e8;
    color: #2a6e2a;
    font-size: 10px;
    font-weight: 600;
    padding: 1px var(--space-xs);
    border-radius: var(--radius-sm);
    white-space: nowrap;
    cursor: help;
  }

  .key-detecting {
    font-size: 10px;
    color: var(--color-text-muted);
    font-style: italic;
  }

  .auto-balance-label {
    font-size: 11px;
    color: var(--color-text-secondary);
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    cursor: pointer;
    white-space: nowrap;
    margin-left: auto;
  }

  /* ── 速度/移調 ── */
  .sp-control {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    font-size: 12px;
    color: var(--color-text);
  }

  .sp-label {
    font-weight: 500;
    font-size: 11px;
    color: var(--color-text-muted);
  }

  .sp-slider {
    width: 80px;
    accent-color: var(--color-brand);
  }

  .sp-value {
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text);
    min-width: 36px;
    text-align: center;
  }

  .pitch-val {
    min-width: 28px;
  }

  .sp-btn {
    background: var(--color-bg-hover);
    border: 1px solid var(--color-border);
    border-radius: var(--space-xs);
    width: 22px;
    height: 22px;
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background var(--transition-fast);
  }

  .sp-btn:hover:not(:disabled) {
    background: var(--color-bg-active);
  }

  .sp-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .sp-reset {
    font-size: 11px;
    padding: 2px var(--space-sm);
  }

  /* ── 錄音指示 ── */
  .rec-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-danger);
    animation: rec-blink 1s ease-in-out infinite;
    vertical-align: middle;
  }

  @keyframes rec-blink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.2; }
  }
</style>
