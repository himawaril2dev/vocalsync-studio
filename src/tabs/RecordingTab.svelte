<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { ask, save } from "@tauri-apps/plugin-dialog";
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
    hasNonDefaultSpeed,
    hasNonDefaultPitch,
    setSpeed,
    setPitchSemitones,
    resetSpeed,
    resetPitch,
    pausedResumeMode,
    pausedAtElapsed,
    isTransportRunning,
    isTransportPaused,
    type ResumeMode,
    PITCH_SEMITONES_MIN,
    PITCH_SEMITONES_MAX,
  } from "../stores/transport";
  import { get } from "svelte/store";
  import { loadedMedia } from "../stores/media";
  import { clearLiveVocalSamples } from "../stores/pitch";
  import {
    inputDeviceIndex,
    outputDeviceIndex,
    latencyMs,
    backingVolume,
    micGain,
    guideVolume,
    guideVocalEnabled,
    autoBalanceMixin,
    resetBackingVolume,
    resetMicGain,
    resetGuideVolume,
    DEFAULT_BACKING_VOLUME,
    DEFAULT_MIC_GAIN,
    DEFAULT_GUIDE_VOLUME,
  } from "../stores/settings";
  import { guideVocalPath } from "../stores/melody";
  import { showToast } from "../stores/toast";
  import Icon from "../components/Icon.svelte";
  import LyricsPanel from "../components/LyricsPanel.svelte";
  import LyricsSyncEditor from "../components/LyricsSyncEditor.svelte";
  import PitchTimeline from "../components/PitchTimeline.svelte";
  import { t, tSync } from "../i18n";

  type PanelView = "lyrics" | "sync";

  // 已遷移至全域 toast 系統
  let videoEl = $state<HTMLVideoElement | null>(null);

  // ── 進度條拖動狀態 ──
  // 播放中後端 ~20Hz 推送 elapsed，若 slider 直接綁 $elapsed 會在拖動過程中被覆蓋回去，
  // 造成「拖不動」的感覺。用 seekDraftValue 做拖動中的本地緩衝，鬆手時才套用。
  let isSeekDragging = $state(false);
  let seekDraftValue = $state(0);
  const sliderValue = $derived(isSeekDragging ? seekDraftValue : $elapsed);

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

    // idle 或 paused：影片停下但保持當前畫格（位置對齊 elapsed），
    // 不觸發 play()，避免 paused 下 effect 重跑把影片喚醒。
    if ($transportState === "idle" || $transportState === "paused") {
      if (!videoEl.paused) videoEl.pause();
      // paused 時使用者可能拖動進度條，讓影片同步跳到新位置
      if ($transportState === "paused") {
        const diff = targetTime - videoEl.currentTime;
        if (Math.abs(diff) > 0.1) videoEl.currentTime = targetTime;
      }
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
    invoke("set_volume", {
      backing: $backingVolume,
      mic: $micGain,
      guide: $guideVolume,
    }).catch(() => {});
    invoke("set_guide_vocal_enabled", {
      enabled: Boolean($guideVocalPath && $guideVocalEnabled),
    }).catch(() => {});
  });

  function onSeekInput(e: Event) {
    const val = parseFloat((e.target as HTMLInputElement).value);
    seekDraftValue = val;
    if (!isSeekDragging) isSeekDragging = true;
  }

  function onSeekCommit(e: Event) {
    const val = parseFloat((e.target as HTMLInputElement).value);
    // paused 狀態下後端不會 emit progress，前端要自行同步 elapsed
    // 以便「暫停後拖進度條到 N 秒 → 按繼續」能正確從 N 秒啟動。
    // 進行中狀態下 set 也無害，後端下一個 tick 會覆蓋。
    elapsed.set(val);
    invoke("seek", { seconds: val }).catch((err) =>
      console.error("[seek] failed:", err),
    );
    isSeekDragging = false;
  }

  function onSeekPointerDown() {
    isSeekDragging = true;
    seekDraftValue = get(elapsed);
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

  /** 計算從目前 $elapsed 換算的 start_frame；elapsed == 0 時回傳 null（fresh start）。*/
  function currentStartFrame(): number | null {
    const media = $loadedMedia;
    if (!media || $elapsed <= 0) return null;
    return Math.round($elapsed * media.sample_rate);
  }

  async function startPreview() {
    // paused 模式下按 Play = 從暫停位置續播；idle 則從 0 起播。
    const resuming = $isTransportPaused && $pausedResumeMode === "previewing";
    const startFrame = resuming ? currentStartFrame() : null;
    if (!resuming) {
      resetVideo();
    }
    try {
      await invoke("start_preview", {
        startFrame,
        outputDevice: $outputDeviceIndex,
        inputDevice: $inputDeviceIndex,
      });
      pausedResumeMode.set(null);
      pausedAtElapsed.set(null);
    } catch (e) {
      showToast(tSync("recording.toast.previewFailed", { error: String(e) }), "error");
    }
  }

  async function startPlayback() {
    const resuming = $isTransportPaused && $pausedResumeMode === "playing_back";
    const startFrame = resuming ? currentStartFrame() : null;
    if (!resuming) {
      resetVideo();
    }
    try {
      await invoke("start_playback", {
        startFrame,
        outputDevice: $outputDeviceIndex,
        latencyMs: $latencyMs,
      });
      pausedResumeMode.set(null);
      pausedAtElapsed.set(null);
    } catch (e) {
      showToast(tSync("recording.toast.playbackFailed", { error: String(e) }), "error");
    }
  }

  async function startRecording() {
    // 續錄判斷優先序（從高到低）：
    //   1. paused + 上次是 recording → 從 $elapsed 續錄（已錄內容保留，允許拖進度條向後跳）
    //   2. idle + 已有錄音 + $elapsed > 0 → 也視為續錄（錄→停止→再錄的情境）
    //   3. 其他 → fresh start，清空影片與即時音高
    const media = $loadedMedia;
    const wasRecording =
      $isTransportPaused && $pausedResumeMode === "recording";
    const canResume =
      wasRecording || ($hasRecording && $elapsed > 0 && media !== null);
    const startFrame = canResume ? currentStartFrame() : null;

    // 🔴 Codex 安全審查 P1：若使用者拖進度條**向前**跳後按錄音，
    // 後端會 truncate vocal_buffer 到新位置（捨棄後段已錄的聲音）。
    // 先跳 dialog 確認，避免誤觸毀掉錄音。
    if (canResume && wasRecording && $pausedAtElapsed !== null) {
      const baseline = $pausedAtElapsed;
      const now = $elapsed;
      if (now + 0.05 < baseline) {
        const confirmed = await ask(
          tSync("recording.dialog.forwardRecord.message", {
            now: now.toFixed(1),
            baseline: baseline.toFixed(1),
          }),
          { title: tSync("recording.dialog.forwardRecord.title"), kind: "warning" },
        );
        if (!confirmed) return;
      }
    }

    if (!canResume) {
      resetVideo();
      clearLiveVocalSamples();
    }

    try {
      await invoke("start_recording", {
        startFrame,
        inputDevice: $inputDeviceIndex,
        outputDevice: $outputDeviceIndex,
      });
      hasRecording.set(true);
      pausedResumeMode.set(null);
      pausedAtElapsed.set(null);
    } catch (e) {
      showToast(tSync("recording.toast.recordingFailed", { error: String(e) }), "error");
    }
  }

  /** 暫停目前進行中的模式：保留位置，記住模式與位置供之後繼續。*/
  async function pauseCurrent(): Promise<void> {
    const current = get(transportState);
    if (current === "idle" || current === "paused") return;

    const resumeMode = current as ResumeMode;
    try {
      await invoke("pause_playback");
      videoEl?.pause();
      pausedResumeMode.set(resumeMode);
      pausedAtElapsed.set(get(elapsed));
      transportState.set("paused");
    } catch (e) {
      showToast(tSync("recording.toast.pauseFailed", { error: String(e) }), "error");
    }
  }

  /** 從 paused 狀態繼續上一次模式；透過對應的 start* 函式統一走續播邏輯。*/
  async function resumeFromPause(): Promise<void> {
    const mode = get(pausedResumeMode);
    if (mode === "previewing") await startPreview();
    else if (mode === "recording") await startRecording();
    else if (mode === "playing_back") await startPlayback();
  }

  /** 停止並回到最開頭（使用者要求：停止按鈕一律 seek 0）。*/
  async function stopAll() {
    try {
      // pause_playback 對任何模式都適用（內部走 engine.pause → stop worker）
      await invoke("pause_playback").catch(() => {});
      await invoke("seek", { seconds: 0 });
      elapsed.set(0);
      pausedResumeMode.set(null);
      pausedAtElapsed.set(null);
      transportState.set("idle");
      if (videoEl) {
        videoEl.currentTime = 0;
        videoEl.pause();
      }
    } catch (e) {
      showToast(tSync("recording.toast.stopFailed", { error: String(e) }), "error");
    }
  }

  /** 清除目前錄音：vocal buffer + pitch track 全部歸零，seek 回 0，hasRecording → false。
   *  供使用者在續錄不想要時「重新開始」。*/
  async function clearRecording(): Promise<void> {
    const confirmed = await ask(
      tSync("recording.dialog.clearRecord.message"),
      { title: tSync("recording.dialog.clearRecord.title"), kind: "warning" },
    );
    if (!confirmed) return;

    try {
      await invoke("clear_recording");
      hasRecording.set(false);
      clearLiveVocalSamples();
      resetVideo();
      showToast(tSync("recording.toast.clearedCanRerecord"), "success", 2500);
    } catch (e) {
      showToast(tSync("recording.toast.clearFailed", { error: String(e) }), "error");
    }
  }

  async function exportAudio() {
    try {
      const filePath = await save({
        title: tSync("recording.export.dialog.title"),
        filters: [{ name: tSync("recording.export.dialog.filter"), extensions: ["wav"] }],
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
      showToast(tSync("recording.toast.exportSuccess", { vocal: result.vocal_path, mix: result.mix_path }), "success", 5000);
    } catch (e) {
      showToast(tSync("recording.toast.exportFailed", { error: String(e) }), "error");
    }
  }

  let stateLabelText = $derived.by(() => {
    const translate = $t;
    const s = $transportState;
    const stateMap: Record<import("../stores/transport").TransportState, string> = {
      idle: translate("recording.transport.state.idle"),
      previewing: translate("recording.transport.state.previewing"),
      recording: translate("recording.transport.state.recording"),
      playing_back: translate("recording.transport.state.playingBack"),
      paused: translate("recording.transport.state.paused"),
    };
    if (s === "paused" && $pausedResumeMode) {
      const suffixMap: Record<ResumeMode, string> = {
        previewing: translate("recording.transport.resume.previewing"),
        recording: translate("recording.transport.resume.recording"),
        playing_back: translate("recording.transport.resume.playingBack"),
      };
      return `${stateMap.paused}${suffixMap[$pausedResumeMode]}`;
    }
    return stateMap[s];
  });

  function rmsToWidth(rms: number, boost: number = 3.0): number {
    return Math.min(100, rms * boost * 100);
  }

  function handleGlobalKeydown(e: KeyboardEvent) {
    // 忽略在 input/textarea 中的按鍵
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

    switch (e.code) {
      case "Space":
        // Space = 通用 play/pause toggle
        //   running → pauseCurrent
        //   paused → resumeFromPause
        //   idle → startPreview（試聽）
        e.preventDefault();
        if ($isTransportRunning) {
          pauseCurrent();
        } else if ($isTransportPaused) {
          resumeFromPause();
        } else if ($transportState === "idle" && $loadedMedia) {
          startPreview();
        }
        break;
      case "KeyR":
        // R = 錄音。running 時無作用（避免誤觸切模式）；idle/paused 時按即錄
        if (!e.ctrlKey && !e.metaKey && !$isTransportRunning && $loadedMedia) {
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

  // ── Speed 檔位 ──
  // 0.5 / 0.75 / 0.9 / 1.0 / 1.1 / 1.25 六段
  // 1.0 走 bypass、純移調走 timestretch producer、其他走 WSOLA（方向正確且穩定）
  const SPEED_STEPS = [0.5, 0.75, 0.9, 1.0, 1.1, 1.25] as const;
  const SPEED_EPSILON = 0.01;

  /** 在 SPEED_STEPS 中找最接近目前值的索引（若無完全吻合則取最近的）。*/
  function currentSpeedIndex(v: number): number {
    let bestIdx = 0;
    let bestDiff = Math.abs(SPEED_STEPS[0] - v);
    for (let i = 1; i < SPEED_STEPS.length; i++) {
      const d = Math.abs(SPEED_STEPS[i] - v);
      if (d < bestDiff) {
        bestDiff = d;
        bestIdx = i;
      }
    }
    return bestIdx;
  }

  function decrementSpeedStep() {
    const i = currentSpeedIndex($speed);
    // 若目前值就在某檔位 ±epsilon 內，則往下切一檔；否則先 snap 到最近檔位
    const onStep = Math.abs(SPEED_STEPS[i] - $speed) < SPEED_EPSILON;
    const targetIdx = onStep ? Math.max(0, i - 1) : i;
    setSpeed(SPEED_STEPS[targetIdx]);
  }

  function incrementSpeedStep() {
    const i = currentSpeedIndex($speed);
    const onStep = Math.abs(SPEED_STEPS[i] - $speed) < SPEED_EPSILON;
    const targetIdx = onStep ? Math.min(SPEED_STEPS.length - 1, i + 1) : i;
    setSpeed(SPEED_STEPS[targetIdx]);
  }

  // 按鈕 disabled 判定（響應式，跟著 $speed 自動更新）
  let atMinSpeed = $derived($speed <= SPEED_STEPS[0] + SPEED_EPSILON);
  let atMaxSpeed = $derived($speed >= SPEED_STEPS[SPEED_STEPS.length - 1] - SPEED_EPSILON);

  /** 各軌音量是否已偏離預設值（用於顯示對應的「重設」按鈕）*/
  let hasNonDefaultBacking = $derived(
    Math.abs($backingVolume - DEFAULT_BACKING_VOLUME) > 0.001,
  );
  let hasNonDefaultMic = $derived(
    Math.abs($micGain - DEFAULT_MIC_GAIN) > 0.001,
  );
  let hasNonDefaultGuide = $derived(
    Math.abs($guideVolume - DEFAULT_GUIDE_VOLUME) > 0.001,
  );
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
            <p>{$t("recording.video.audioOnly")}</p>
            <p class="hint">{$t("recording.video.audioInfo", { minutes: ($loadedMedia.duration / 60).toFixed(1), sampleRate: $loadedMedia.sample_rate })}</p>
          </div>
        {:else}
          <div class="video-placeholder">
            <p>{$t("recording.video.noBacking")}</p>
          </div>
        {/if}
      </div>

      <div
        class="resizer resizer-h"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize video and lyrics panels"
        onpointerdown={startDragH}
      ></div>

      <div class="lyrics-area" style="flex: {1 - videoFlex};">
        <div class="panel-toggle">
          <button
            class="toggle-btn"
            class:active={panelView === "lyrics"}
            onclick={() => (panelView = "lyrics")}
            aria-pressed={panelView === "lyrics"}
          >
            {$t("recording.panel.lyrics")}
          </button>
          <button
            class="toggle-btn"
            class:active={panelView === "sync"}
            onclick={() => (panelView = "sync")}
            aria-pressed={panelView === "sync"}
          >
            {$t("recording.panel.sync")}
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
    <div
      class="resizer resizer-v"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize upper and lower panels"
      onpointerdown={startDragV}
    ></div>

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
        <!-- 試聽：idle 可按；paused 時按下會從暫停位置繼續原模式 -->
        <button
          class="t-btn play"
          onclick={$isTransportPaused ? resumeFromPause : startPreview}
          disabled={!$loadedMedia || $isTransportRunning}
          title={$isTransportPaused ? $t("recording.action.resume.title") : $t("recording.action.play.title")}
          aria-label={$isTransportPaused ? $t("recording.action.resume.aria") : $t("recording.action.play.aria")}
        ><Icon name="play" size={14} /></button>

        <!-- 暫停：running 時可按暫停；paused 時保持 II 圖示且 disabled（由試聽/錄音鍵繼續）-->
        <button
          class="t-btn pause"
          onclick={pauseCurrent}
          disabled={!$isTransportRunning}
          title={$t("recording.action.pause.title")}
          aria-label={$t("recording.action.pause.aria")}
        ><Icon name="pause" size={14} /></button>

        <!-- 停止：回到最開頭。running 或 paused 時可按 -->
        <button
          class="t-btn stop"
          onclick={stopAll}
          disabled={$transportState === "idle"}
          title={$t("recording.action.stop.title")}
          aria-label={$t("recording.action.stop.aria")}
        ><Icon name="stop" size={14} /></button>

        <!-- 錄音：idle → 新錄/續錄，paused(recording) → 續錄，paused(其他) → 從當前位置開錄 -->
        <button
          class="t-btn rec"
          onclick={startRecording}
          disabled={!$loadedMedia || $isTransportRunning}
          title={$t("recording.action.record.title")}
          aria-label={$t("recording.action.record.aria")}
        ><Icon name="record" size={14} /></button>
      </div>

      <div class="state-chip">
        {#if $transportState === "recording"}
          <span class="rec-dot" aria-label={$t("recording.action.record.dot.aria")}></span>
        {/if}
        <strong>{stateLabelText}</strong>
        {#if $loadedMedia}
          <span class="track-name" title={$loadedMedia.file_path}>{$loadedMedia.file_name}</span>
        {/if}
        {#if $loopActive}
          <span class="loop-badge">{$t("recording.loop.badge")}</span>
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
      <input
        type="range"
        class="progress-slider"
        min="0"
        max={$duration || 1}
        step="0.1"
        value={sliderValue}
        onpointerdown={onSeekPointerDown}
        oninput={onSeekInput}
        onchange={onSeekCommit}
      />
    </div>

    <div class="time-display time-end">{fmtTime($duration)}</div>
  </div>

  <!-- 控制列：音量 + 調整 + 匯出 -->
  <div class="control-row">
    <div class="vu-compact">
      <div class="vu-item">
        <span class="vu-label">{$t("recording.volume.backing")}</span>
        <div class="vu-bar"><div class="vu-fill" style="width: {rmsToWidth($backingRms, 3.0)}%"></div></div>
        <input type="range" class="vol-slider" min="0" max="1" step="0.01" bind:value={$backingVolume} title={$t("recording.volume.backing.title", { pct: Math.round($backingVolume * 100) })}>
        <span class="vu-value">{Math.round($backingVolume * 100)}%</span>
        <button
          class="vu-reset"
          onclick={resetBackingVolume}
          disabled={!hasNonDefaultBacking}
          aria-label={$t("recording.volume.backing.reset.aria", { pct: Math.round(DEFAULT_BACKING_VOLUME * 100) })}
          title={$t("recording.volume.backing.reset.title", { pct: Math.round(DEFAULT_BACKING_VOLUME * 100) })}
        >↺</button>
      </div>
      <div class="vu-item">
        <span class="vu-label">{$t("recording.volume.mic")}</span>
        <div class="vu-bar"><div class="vu-fill" style="width: {rmsToWidth($micRms, 5.0)}%"></div></div>
        <input type="range" class="vol-slider" min="0" max="3" step="0.01" bind:value={$micGain} title={$t("recording.volume.mic.title", { pct: Math.round($micGain * 100) })}>
        <span class="vu-value">{Math.round($micGain * 100)}%</span>
        <button
          class="vu-reset"
          onclick={resetMicGain}
          disabled={!hasNonDefaultMic}
          aria-label={$t("recording.volume.mic.reset.aria", { pct: Math.round(DEFAULT_MIC_GAIN * 100) })}
          title={$t("recording.volume.mic.reset.title", { pct: Math.round(DEFAULT_MIC_GAIN * 100) })}
        >↺</button>
      </div>
      <div class:disabled-guide={!$guideVocalPath} class="vu-item guide-item">
        <label
          class="vu-enable"
          title={$guideVocalPath
            ? $t("recording.volume.guide.enable.title")
            : $t("recording.volume.guide.disabledTitle")}
        >
          <input type="checkbox" bind:checked={$guideVocalEnabled} disabled={!$guideVocalPath} />
        </label>
        <span class="vu-label">{$t("recording.volume.guide")}</span>
        <div class="vu-bar"><div class="vu-fill guide-fill" style="width: {$guideVocalPath && $guideVocalEnabled ? Math.round($guideVolume * 100) : 0}%"></div></div>
        <input
          type="range"
          class="vol-slider"
          min="0"
          max="1"
          step="0.01"
          bind:value={$guideVolume}
          disabled={!$guideVocalPath || !$guideVocalEnabled}
          title={$guideVocalPath
            ? $t("recording.volume.guide.title", { pct: Math.round($guideVolume * 100) })
            : $t("recording.volume.guide.disabledTitle")}
        >
        <span class="vu-value">{Math.round($guideVolume * 100)}%</span>
        <button
          class="vu-reset"
          onclick={resetGuideVolume}
          disabled={!$guideVocalPath || !hasNonDefaultGuide}
          aria-label={$t("recording.volume.guide.reset.aria", { pct: Math.round(DEFAULT_GUIDE_VOLUME * 100) })}
          title={$t("recording.volume.guide.reset.title", { pct: Math.round(DEFAULT_GUIDE_VOLUME * 100) })}
        >↺</button>
      </div>
    </div>

    <div class="divider"></div>

    <div class="sp-control">
      <span class="sp-label">{$t("recording.speed.label")}</span>
      <button
        class="sp-btn"
        onclick={decrementSpeedStep}
        disabled={atMinSpeed}
        title={$t("recording.speed.decrease.title")}
      >-</button>
      <span class="sp-value">{$speed.toFixed(2)}x</span>
      <button
        class="sp-btn"
        onclick={incrementSpeedStep}
        disabled={atMaxSpeed}
        title={$t("recording.speed.increase.title")}
      >+</button>
      <button
        class="vu-reset"
        onclick={resetSpeed}
        disabled={!$hasNonDefaultSpeed}
        aria-label={$t("recording.speed.reset.aria")}
        title={$t("recording.speed.reset.title")}
      >↺</button>
    </div>

    <div class="sp-control">
      <span class="sp-label">{$t("recording.pitch.label")}</span>
      <button class="sp-btn" onclick={() => setPitchSemitones($pitchSemitones - 1)} disabled={$pitchSemitones <= PITCH_SEMITONES_MIN} title={$t("recording.pitch.decrease.title")}>-</button>
      <span class="sp-value pitch-val">{$pitchSemitones > 0 ? "+" : ""}{$pitchSemitones}</span>
      <button class="sp-btn" onclick={() => setPitchSemitones($pitchSemitones + 1)} disabled={$pitchSemitones >= PITCH_SEMITONES_MAX} title={$t("recording.pitch.increase.title")}>+</button>
      <button
        class="vu-reset"
        onclick={resetPitch}
        disabled={!$hasNonDefaultPitch}
        aria-label={$t("recording.pitch.reset.aria")}
        title={$t("recording.pitch.reset.title")}
      >↺</button>
    </div>

    <div class="divider"></div>

    <div class="loop-group">
      <button
        class="t-btn loop-ab"
        class:loop-set={$loopA !== null}
        onclick={setLoopA}
        title={$loopA !== null ? $t("recording.loop.a.set", { time: fmtTime($loopA) }) : $t("recording.loop.a.default")}
      >A</button>
      <button
        class="t-btn loop-ab"
        class:loop-set={$loopB !== null}
        onclick={async () => {
          const ok = await setLoopB();
          if (!ok) showToast(tSync("recording.toast.loopBAfterA"), "warning");
        }}
        disabled={$loopA === null}
        title={$loopB !== null ? $t("recording.loop.b.set", { time: fmtTime($loopB) }) : $t("recording.loop.b.default")}
      >B</button>
      {#if $loopActive}
        <button class="t-btn loop-clear" onclick={clearLoop} title={$t("recording.loop.clear.title")}>✕</button>
      {/if}
    </div>

    <div class="divider"></div>

    <button
      class="t-btn export"
      onclick={exportAudio}
      disabled={$isTransportRunning || !$hasRecording}
    >
      <Icon name="download" size={14} /> {$t("recording.export.button")}
    </button>

    <button
      class="t-btn playback"
      onclick={startPlayback}
      disabled={!$hasRecording || !$loadedMedia || $isTransportRunning}
      title={$t("recording.playback.title")}
    >
      {$t("recording.playback.button")}
    </button>

    <button
      class="t-btn clear-rec"
      onclick={clearRecording}
      disabled={!$hasRecording || $isTransportRunning}
      title={$t("recording.clearRecord.title")}
    >
      {$t("recording.clearRecord.button")}
    </button>

    <div class="auto-balance-group">
      <button
        type="button"
        class="auto-balance-hint"
        data-tooltip={$t("recording.autoBalance.tooltip")}
        aria-label={$t("recording.autoBalance.aria")}
      >!</button>
      <label class="auto-balance-label">
        <input type="checkbox" bind:checked={$autoBalanceMixin} />
        {$t("recording.autoBalance.label")}
      </label>
    </div>
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

  .track-name {
    font-size: 12px;
    color: var(--color-text);
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: help;
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
    min-width: 280px;
  }

  .vu-item {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .vu-label {
    width: 52px;
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
    white-space: nowrap;
  }

  .guide-item .vu-label {
    width: 52px;
  }

  .disabled-guide {
    opacity: 0.42;
  }

  .vu-enable {
    width: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .vu-enable input {
    width: 13px;
    height: 13px;
    accent-color: var(--color-brand);
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

  .guide-fill {
    background: linear-gradient(90deg, #b89a45 0%, #d7bd6d 100%);
  }

  .vol-slider {
    width: 60px;
    accent-color: var(--color-brand);
  }

  .vu-value {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text);
    min-width: 36px;
    text-align: right;
    flex-shrink: 0;
  }

  .vu-reset {
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: var(--color-bg-hover);
    color: var(--color-text-muted);
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all var(--transition-fast);
  }

  .vu-reset:hover:not(:disabled) {
    background: var(--color-bg-active);
    color: var(--color-text);
  }

  .vu-reset:disabled {
    opacity: 0.25;
    cursor: not-allowed;
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
    width: 36px;
    height: 36px;
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
    width: 36px;
    height: 36px;
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

  .t-btn.clear-rec {
    font-size: 12px;
    background: transparent;
    border: 1px solid var(--color-border);
    color: var(--color-text-muted);
  }

  .t-btn.clear-rec:hover:not(:disabled) {
    background: var(--color-danger-soft, #fde8e8);
    border-color: var(--color-danger, #b71c1c);
    color: var(--color-danger, #b71c1c);
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

  .auto-balance-group {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    margin-left: auto;
  }

  .auto-balance-hint {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    font-size: 11px;
    font-weight: 700;
    line-height: 1;
    color: var(--color-text-muted);
    border: 1px solid var(--color-border);
    border-radius: 50%;
    cursor: help;
    user-select: none;
  }

  .auto-balance-hint:hover,
  .auto-balance-hint:focus-visible {
    color: var(--color-text);
    border-color: var(--color-text-muted);
    outline: none;
  }

  /* 自訂 tooltip：Tauri WebView2 對原生 title 支援不穩定，改用 CSS ::after */
  .auto-balance-hint:hover::after,
  .auto-balance-hint:focus-visible::after {
    content: attr(data-tooltip);
    position: absolute;
    bottom: calc(100% + 8px);
    right: 0;
    background: #2a241e;
    color: #fff;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 400;
    line-height: 1.5;
    white-space: normal;
    width: max-content;
    max-width: 280px;
    text-align: left;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
    z-index: 9999;
    pointer-events: none;
  }

  /* 下方小三角形指向按鈕 */
  .auto-balance-hint:hover::before,
  .auto-balance-hint:focus-visible::before {
    content: "";
    position: absolute;
    bottom: calc(100% + 3px);
    right: 6px;
    border: 5px solid transparent;
    border-top-color: #2a241e;
    z-index: 9999;
    pointer-events: none;
  }

  .auto-balance-label {
    font-size: 11px;
    color: var(--color-text-secondary);
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    cursor: pointer;
    white-space: nowrap;
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
