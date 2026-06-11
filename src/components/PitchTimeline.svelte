<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { elapsed } from "../stores/transport";
  import { latencyMs } from "../stores/settings";
  import {
    currentPitch,
    backingPitchTrack,
    backingPitchAnalyzing,
    freeMode,
    freeModeReason,
    liveVocalSamples,
    type BackingPitchAnalyzing,
    type PitchTrackSample,
  } from "../stores/pitch";
  import { t } from "../i18n";

  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let containerEl = $state<HTMLDivElement | null>(null);
  let rafId: number | null = null;

  // 反應式訂閱：自由模式 / 分析中狀態切換時重新調整 UI
  let isFreeMode = $state(false);
  let analyzing = $state<BackingPitchAnalyzing | null>(null);
  let manualMidiOffset = $state(0);
  // 分析中經過秒數（每秒更新一次，給橫幅顯示「已 X 秒」）
  let analyzingSeconds = $state(0);
  let analyzingTimer: number | null = null;

  const unsubFree = freeMode.subscribe((v) => (isFreeMode = v));

  /**
   * 把結構化 freeModeReason 翻成顯示字串。
   * 綁定 $t 讓 locale 切換時自動重翻，避免卡舊語言。
   */
  let freeReason = $derived.by(() => {
    const translate = $t;
    const r = $freeModeReason;
    if (!r) return "";
    if (r.kind === "i18n") return translate(r.key, r.vars);
    return r.text;
  });
  const unsubAnalyzing = backingPitchAnalyzing.subscribe((v) => {
    analyzing = v;
    if (analyzingTimer !== null) {
      clearInterval(analyzingTimer);
      analyzingTimer = null;
    }
    if (v) {
      analyzingSeconds = 0;
      analyzingTimer = window.setInterval(() => {
        analyzingSeconds += 1;
      }, 1000);
    } else {
      analyzingSeconds = 0;
    }
  });

  // ── 顯示參數 ──
  // 視窗寬度：以當前時間為中心，前後各看 5 秒
  const WINDOW_SECONDS = 10;
  const HALF_WINDOW = WINDOW_SECONDS / 2;

  // Y 軸自動聚焦：保留 3 個八度視窗，跟著可見的人聲/旋律置中。
  const VIEW_MIDI_SPAN = 36;
  const VIEW_MIDI_MIN_LIMIT = 24; // C1
  const VIEW_MIDI_MAX_LIMIT = 96; // C7
  const VIEW_SCROLL_STEP = 2;

  // 顏色
  const COLOR_BG = "#fafaf6";
  const COLOR_GRID = "#ebe6dc";
  const COLOR_LABEL = "#b0a898";
  const COLOR_BACKING = "#7cafc2"; // 柔和藍：目標旋律
  const COLOR_VOCAL = "#fdc003"; // 棕金：你的人聲
  const COLOR_CURSOR = "#c0392b"; // 紅：當前位置
  const COLOR_OCTAVE_LINE = "#d8d2c4";

  function freqToMidi(freq: number): number {
    if (freq <= 0) return 0;
    return 69 + 12 * Math.log2(freq / 440);
  }

  function clamp(value: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, value));
  }

  function midiToY(midi: number, h: number, viewMin: number, viewMax: number): number {
    const range = viewMax - viewMin;
    const norm = (midi - viewMin) / range;
    return h - norm * h;
  }

  function timeToX(t: number, currentT: number, w: number): number {
    // 當前時間在中央，每秒對應 w / WINDOW_SECONDS 像素
    const offset = t - currentT;
    const centerX = w / 2;
    return centerX + (offset / WINDOW_SECONDS) * w;
  }

  function firstVisibleIndex(
    samples: PitchTrackSample[],
    startTime: number,
    timeOffsetSecs = 0,
  ): number {
    const rawStartTime = startTime - timeOffsetSecs;
    let lo = 0;
    let hi = samples.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (samples[mid].timestamp < rawStartTime) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  function collectVisibleMidi(
    samples: PitchTrackSample[],
    tStart: number,
    tEnd: number,
    values: number[],
    timeOffsetSecs = 0,
  ) {
    const startIndex = firstVisibleIndex(samples, tStart - 0.5, timeOffsetSecs);
    for (let i = startIndex; i < samples.length; i++) {
      const sample = samples[i];
      const displayTime = sample.timestamp + timeOffsetSecs;
      if (displayTime > tEnd + 0.5) break;
      if (sample.freq <= 0) continue;
      const midi = freqToMidi(sample.freq);
      if (Number.isFinite(midi) && midi >= VIEW_MIDI_MIN_LIMIT && midi <= VIEW_MIDI_MAX_LIMIT) {
        values.push(midi);
      }
    }
  }

  function median(values: number[]): number {
    if (values.length === 0) return 60;
    const sorted = [...values].sort((a, b) => a - b);
    const mid = Math.floor(sorted.length / 2);
    return sorted.length % 2 === 0
      ? (sorted[mid - 1] + sorted[mid]) / 2
      : sorted[mid];
  }

  function computeViewRange(tStart: number, tEnd: number): { min: number; max: number } {
    const values: number[] = [];
    const backingTrack = get(backingPitchTrack);
    if (backingTrack && !isFreeMode) {
      collectVisibleMidi(backingTrack.samples, tStart, tEnd, values);
    }
    const vocalTimeOffsetSecs = -get(latencyMs) / 1000;
    collectVisibleMidi(get(liveVocalSamples), tStart, tEnd, values, vocalTimeOffsetSecs);

    const cur = get(currentPitch);
    if (cur?.freq) {
      values.push(freqToMidi(cur.freq));
    }

    const center = clamp(
      median(values),
      VIEW_MIDI_MIN_LIMIT + VIEW_MIDI_SPAN / 2,
      VIEW_MIDI_MAX_LIMIT - VIEW_MIDI_SPAN / 2,
    );
    const viewMin = clamp(
      center - VIEW_MIDI_SPAN / 2 + manualMidiOffset,
      VIEW_MIDI_MIN_LIMIT,
      VIEW_MIDI_MAX_LIMIT - VIEW_MIDI_SPAN,
    );
    return { min: viewMin, max: viewMin + VIEW_MIDI_SPAN };
  }

  function shiftView(deltaMidi: number) {
    manualMidiOffset = clamp(manualMidiOffset + deltaMidi, -36, 36);
  }

  function handleWheel(event: WheelEvent) {
    if (Math.abs(event.deltaY) < Math.abs(event.deltaX)) return;
    event.preventDefault();
    shiftView(event.deltaY > 0 ? -VIEW_SCROLL_STEP : VIEW_SCROLL_STEP);
  }

  // ── 繪製 ──
  function draw() {
    if (!canvasEl) {
      rafId = requestAnimationFrame(draw);
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      rafId = requestAnimationFrame(draw);
      return;
    }

    // 處理 high-DPI
    const dpr = window.devicePixelRatio || 1;
    const cssW = canvasEl.clientWidth;
    const cssH = canvasEl.clientHeight;
    if (canvasEl.width !== cssW * dpr || canvasEl.height !== cssH * dpr) {
      canvasEl.width = cssW * dpr;
      canvasEl.height = cssH * dpr;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const w = cssW;
    const h = cssH;

    // 背景
    ctx.fillStyle = COLOR_BG;
    ctx.fillRect(0, 0, w, h);

    const currentT = get(elapsed);
    const tStart = currentT - HALF_WINDOW;
    const tEnd = currentT + HALF_WINDOW;
    const viewRange = computeViewRange(tStart, tEnd);

    // 八度線（C 音標籤）：在當前聚焦 view 範圍內畫出每個 C
    ctx.strokeStyle = COLOR_OCTAVE_LINE;
    ctx.lineWidth = 1;
    ctx.font = "10px Consolas, monospace";
    ctx.fillStyle = COLOR_LABEL;
    ctx.textBaseline = "middle";
    const octaveLo = Math.floor(viewRange.min / 12) - 1;
    const octaveHi = Math.ceil(viewRange.max / 12);
    for (let octave = octaveLo; octave <= octaveHi; octave++) {
      const midi = (octave + 1) * 12; // MIDI for C{octave}
      if (midi < viewRange.min || midi > viewRange.max) continue;
      const y = midiToY(midi, h, viewRange.min, viewRange.max);
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
      ctx.fillText(`C${octave}`, 4, y - 6);
    }

    // 半音格線（淡色）
    ctx.strokeStyle = COLOR_GRID;
    const semiLo = Math.ceil(viewRange.min);
    const semiHi = Math.floor(viewRange.max);
    for (let midi = semiLo; midi <= semiHi; midi++) {
      const y = midiToY(midi, h, viewRange.min, viewRange.max);
      ctx.beginPath();
      ctx.moveTo(28, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }

    // ── 伴奏旋律（灰藍線）──
    // 自由模式時隱藏，避免顯示不可靠的目標旋律
    const backingTrack = get(backingPitchTrack);
    if (backingTrack && !isFreeMode) {
      ctx.strokeStyle = COLOR_BACKING;
      ctx.lineWidth = 3;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      drawSegmentedLine(ctx, backingTrack.samples, currentT, w, h, tStart, tEnd, viewRange.min, viewRange.max);
    }

    // ── 即時人聲（棕金線）──
    const vocalSamples = get(liveVocalSamples);
    if (vocalSamples.length > 0) {
      ctx.strokeStyle = COLOR_VOCAL;
      ctx.lineWidth = 3;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.shadowColor = "rgba(253, 192, 3, 0.4)";
      ctx.shadowBlur = 6;
      drawSegmentedLine(
        ctx,
        vocalSamples,
        currentT,
        w,
        h,
        tStart,
        tEnd,
        viewRange.min,
        viewRange.max,
        -get(latencyMs) / 1000,
      );
      ctx.shadowBlur = 0;
    }

    // ── 中央游標（紅線）──
    ctx.strokeStyle = COLOR_CURSOR;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(w / 2, 0);
    ctx.lineTo(w / 2, h);
    ctx.stroke();

    // ── 當前音名顯示（右上角）──
    const cur = get(currentPitch);
    if (cur) {
      ctx.fillStyle = COLOR_VOCAL;
      ctx.font = "bold 28px Consolas, monospace";
      ctx.textBaseline = "top";
      ctx.textAlign = "right";
      ctx.fillText(`${cur.note}${cur.octave}`, w - 12, 8);

      ctx.font = "11px Consolas, monospace";
      ctx.fillStyle = COLOR_LABEL;
      const sign = cur.cent > 0 ? "+" : "";
      ctx.fillText(`${sign}${cur.cent.toFixed(0)}¢`, w - 12, 40);
    }

    rafId = requestAnimationFrame(draw);
  }

  /**
   * 畫一條音高線：把連續樣本連成平滑曲線（quadratic bezier smooth），
   * 模擬 tkinter 的 smooth=True 效果。時間間隔過大或頻率跳出 view 時斷開。
   *
   * 演算法：對連續區段內的點，以「中點」當 anchor，原點當 control point，
   * 用 quadraticCurveTo 連起來。第一個點用 moveTo，最後一個點用 lineTo 收尾。
   */
  function drawSegmentedLine(
    ctx: CanvasRenderingContext2D,
    samples: PitchTrackSample[],
    currentT: number,
    w: number,
    h: number,
    tStart: number,
    tEnd: number,
    viewMin: number,
    viewMax: number,
    timeOffsetSecs = 0,
  ) {
    // Step 1: 把可見且在範圍內的點分組（time gap > 0.5s 或 midi 出範圍時斷組）
    const segments: Array<Array<{ x: number; y: number }>> = [];
    let current: Array<{ x: number; y: number }> = [];
    let prevT: number | null = null;
    let prevMidi: number | null = null;

    const flush = () => {
      if (current.length > 0) {
        segments.push(current);
        current = [];
      }
    };

    // 二分搜尋找到 tStart - 0.5 附近的起始 index，避免掃描所有早期 sample
    // 對 5 分鐘歌曲（~30000 samples），從 O(n) 降到 O(log n + visible)
    const searchTarget = tStart - 0.5 - timeOffsetSecs;
    let lo = 0;
    let hi = samples.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (samples[mid].timestamp < searchTarget) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }

    for (let si = lo; si < samples.length; si++) {
      const s = samples[si];
      const displayTime = s.timestamp + timeOffsetSecs;
      if (displayTime > tEnd + 0.5) break;

      const midi = freqToMidi(s.freq);
      if (midi < viewMin || midi > viewMax) {
        flush();
        prevT = null;
        prevMidi = null;
        continue;
      }

      // 斷線條件：時間 gap 過大，或音高跳躍過大
      const gap = prevT !== null ? displayTime - prevT : 0;
      const centJump =
        prevMidi !== null ? Math.abs(midi - prevMidi) * 100 : 0;

      // 時間 gap > 80ms（CREPE 10ms hop 的 8 倍）或音高跳 > 200 cent（2 半音）
      if (gap > 0.08 || centJump > 200) {
        flush();
      }

      const x = timeToX(displayTime, currentT, w);
      const y = midiToY(midi, h, viewMin, viewMax);
      current.push({ x, y });
      prevT = displayTime;
      prevMidi = midi;
    }
    flush();

    // Step 2: 對每個 segment 用 quadratic bezier smooth 繪製
    for (const seg of segments) {
      if (seg.length === 0) continue;
      ctx.beginPath();
      if (seg.length === 1) {
        // 單點：畫小圓
        ctx.arc(seg[0].x, seg[0].y, 1.5, 0, Math.PI * 2);
        ctx.fill();
        continue;
      }
      if (seg.length === 2) {
        // 兩點：直線
        ctx.moveTo(seg[0].x, seg[0].y);
        ctx.lineTo(seg[1].x, seg[1].y);
        ctx.stroke();
        continue;
      }
      // 三點以上：quadratic curve through midpoints
      ctx.moveTo(seg[0].x, seg[0].y);
      for (let i = 1; i < seg.length - 1; i++) {
        const midX = (seg[i].x + seg[i + 1].x) / 2;
        const midY = (seg[i].y + seg[i + 1].y) / 2;
        ctx.quadraticCurveTo(seg[i].x, seg[i].y, midX, midY);
      }
      // 最後一點直接 lineTo 收尾
      const last = seg[seg.length - 1];
      ctx.lineTo(last.x, last.y);
      ctx.stroke();
    }
  }

  onMount(() => {
    rafId = requestAnimationFrame(draw);
  });

  onDestroy(() => {
    if (rafId !== null) cancelAnimationFrame(rafId);
    if (analyzingTimer !== null) clearInterval(analyzingTimer);
    unsubFree();
    unsubAnalyzing();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pitch-timeline"
  bind:this={containerEl}
  onwheel={handleWheel}
>
  <canvas bind:this={canvasEl}></canvas>

  <!-- 圖例（自由模式時隱藏目標旋律項目）-->
  <div class="legend">
    {#if !isFreeMode}
      <span class="legend-item">
        <span class="legend-line backing"></span>
        {$t("pitchTimeline.legend.melody")}
      </span>
    {/if}
    <span class="legend-item">
      <span class="legend-line vocal"></span>
      {$t("pitchTimeline.legend.yourPitch")}
    </span>
  </div>

  <div class="view-controls" aria-label="Pitch range controls">
    <button
      type="button"
      title={$t("pitchTimeline.view.up")}
      aria-label={$t("pitchTimeline.view.up")}
      onclick={() => shiftView(VIEW_SCROLL_STEP)}
    >
      ↑
    </button>
    <button
      type="button"
      title={$t("pitchTimeline.view.auto")}
      aria-label={$t("pitchTimeline.view.auto")}
      onclick={() => (manualMidiOffset = 0)}
    >
      ◎
    </button>
    <button
      type="button"
      title={$t("pitchTimeline.view.down")}
      aria-label={$t("pitchTimeline.view.down")}
      onclick={() => shiftView(-VIEW_SCROLL_STEP)}
    >
      ↓
    </button>
  </div>

  <!-- 分析中橫幅（優先顯示）-->
  {#if analyzing}
    <div class="analyzing-banner" role="status">
      <span class="spinner"></span>
      <span class="banner-tag analyzing-tag">{$t("pitchTimeline.banner.analyzing.tag")}</span>
      <span class="banner-text">
        {$t("pitchTimeline.banner.analyzing.text", { seconds: analyzingSeconds })}
        {#if analyzing.duration > 0}
          {$t("pitchTimeline.banner.analyzing.duration", { duration: analyzing.duration.toFixed(1) })}
        {/if}
      </span>
    </div>
  {:else if isFreeMode}
    <!-- 自由模式提示橫幅 -->
    <div class="free-mode-banner" role="status">
      <span class="banner-tag">{$t("pitchTimeline.banner.freeMode.tag")}</span>
      <span class="banner-text">{freeReason || $t("pitchTimeline.banner.freeMode.defaultReason")}</span>
    </div>
  {/if}
</div>

<style>
  .pitch-timeline {
    position: relative;
    background: #fafaf6;
    border-radius: 10px;
    height: 100%;
    min-height: 280px;
    flex: 1;
    overflow: hidden;
  }

  canvas {
    width: 100%;
    height: 100%;
    display: block;
  }

  .legend {
    position: absolute;
    top: 8px;
    left: 12px;
    display: flex;
    gap: 14px;
    font-size: 11px;
    color: #7a7268;
    pointer-events: none;
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .legend-line {
    display: inline-block;
    width: 16px;
    height: 3px;
    border-radius: 2px;
  }

  .legend-line.backing {
    background: #7cafc2;
  }

  .legend-line.vocal {
    background: #fdc003;
    box-shadow: 0 0 4px rgba(253, 192, 3, 0.5);
  }

  .view-controls {
    position: absolute;
    top: 8px;
    right: 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .view-controls button {
    width: 28px;
    height: 28px;
    border: 1px solid rgba(139, 105, 0, 0.18);
    border-radius: 999px;
    background: rgba(255, 252, 244, 0.86);
    color: #8b6900;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    box-shadow: 0 6px 16px rgba(62, 54, 45, 0.08);
  }

  .view-controls button:hover {
    background: #fff8df;
    border-color: rgba(139, 105, 0, 0.35);
  }

  .free-mode-banner {
    position: absolute;
    bottom: 8px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 12px;
    background: rgba(124, 175, 194, 0.18);
    border: 1px solid rgba(124, 175, 194, 0.45);
    font-size: 11px;
    color: #5f7b88;
    pointer-events: none;
    backdrop-filter: blur(2px);
  }

  .banner-tag {
    font-weight: 600;
    color: #4a6a78;
    background: rgba(124, 175, 194, 0.35);
    padding: 1px 6px;
    border-radius: 8px;
  }

  .banner-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 320px;
  }

  .analyzing-banner {
    position: absolute;
    bottom: 8px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    border-radius: 14px;
    background: rgba(253, 192, 3, 0.18);
    border: 1px solid rgba(253, 192, 3, 0.5);
    font-size: 11px;
    color: #7a5a00;
    pointer-events: none;
    backdrop-filter: blur(2px);
  }

  .analyzing-tag {
    font-weight: 600;
    color: #5a4200;
    background: rgba(253, 192, 3, 0.45);
    padding: 1px 6px;
    border-radius: 8px;
  }

  .spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 2px solid rgba(253, 192, 3, 0.4);
    border-top-color: #b88600;
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
