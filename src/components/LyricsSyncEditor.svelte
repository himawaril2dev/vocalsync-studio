<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { lyricsLines, lyricsFileName } from "../stores/lyrics";
  import { elapsed } from "../stores/transport";
  import { loadedMedia } from "../stores/media";
  import type { LyricLine } from "../stores/lyrics";
  import { t, tSync } from "../i18n";

  interface SyncLine {
    text: string;
    translation?: string;
    start_ms: number;
    end_ms: number;
    synced: boolean;
  }

  type DragMode = "start" | "end" | "move";
  type ExportFormat = "lrc" | "srt" | "ass";

  interface TimelineDragState {
    idx: number;
    mode: DragMode;
    startX: number;
    trackWidth: number;
    baseStart: number;
    baseEnd: number;
  }

  let lines = $state<SyncLine[]>([]);
  let undoStack = $state<SyncLine[][]>([]);
  let lineEls = $state<HTMLDivElement[]>([]);
  let saveMsg = $state("");
  let alignMsg = $state("");
  let exportFormat = $state<ExportFormat>("lrc");
  let isExporting = $state(false);
  let detectedAudioDurationMs = $state(0);
  let dragState = $state<TimelineDragState | null>(null);
  let globalDelayMs = $state(0);
  let globalDelayHistoryCaptured = $state(false);

  let lastLyricsKey = $state("");

  let timelineMaxMs = $derived.by(() => {
    const mediaMs = ($loadedMedia?.duration ?? 0) * 1000;
    const lastLineMs = lines.reduce(
      (max, line) => Math.max(max, line.start_ms, line.end_ms),
      0,
    );
    return Math.max(1000, Math.ceil(mediaMs), detectedAudioDurationMs, lastLineMs + 1000);
  });

  let playbackIdx = $derived.by(() => {
    const nowMs = $elapsed * 1000;
    for (let i = 0; i < lines.length; i += 1) {
      const line = lines[i];
      if (line.start_ms <= nowMs && nowMs < line.end_ms) return i;
    }
    return -1;
  });

  let timedLineCount = $derived(lines.filter((line) => line.end_ms > line.start_ms).length);
  let canShiftGlobally = $derived(timedLineCount > 0);

  $effect(() => {
    const storeLines = $lyricsLines;
    if (storeLines.length === 0) {
      lines = [];
      lastLyricsKey = "";
      undoStack = [];
      globalDelayMs = 0;
      globalDelayHistoryCaptured = false;
      return;
    }

    const key = lyricsKey(storeLines);
    if (key !== lastLyricsKey) {
      lastLyricsKey = key;
      lines = storeLines.map((line) => ({
        text: line.text,
        translation: line.translation,
        start_ms: line.start_ms,
        end_ms: line.end_ms,
        synced: line.start_ms > 0 || line.end_ms > 0,
      }));
      undoStack = [];
      alignMsg = "";
      globalDelayMs = 0;
      globalDelayHistoryCaptured = false;
    }
  });

  $effect(() => {
    if (playbackIdx >= 0 && lineEls[playbackIdx]) {
      lineEls[playbackIdx].scrollIntoView({ behavior: "smooth", block: "center" });
    }
  });

  function cloneLines(source: SyncLine[]): SyncLine[] {
    return source.map((line) => ({ ...line }));
  }

  function pushHistory() {
    undoStack = [...undoStack.slice(-49), cloneLines(lines)];
  }

  function lineToLyric(line: SyncLine): LyricLine {
    return {
      start_ms: line.start_ms,
      end_ms: line.end_ms,
      text: line.text,
      translation: line.translation,
    };
  }

  function lyricsKey(source: LyricLine[]): string {
    return [
      $lyricsFileName,
      source.length.toString(),
      source
        .map((line) =>
          [
            Math.round(line.start_ms),
            Math.round(line.end_ms),
            line.text,
            line.translation ?? "",
          ].join(":"),
        )
        .join("|"),
    ].join("|");
  }

  function syncLinesToStore() {
    const next = lines.map(lineToLyric);
    lastLyricsKey = lyricsKey(next);
    lyricsLines.set(next);
  }

  function undo() {
    if (undoStack.length === 0) return;
    const previous = undoStack[undoStack.length - 1];
    undoStack = undoStack.slice(0, -1);
    lines = previous;
    globalDelayMs = 0;
    globalDelayHistoryCaptured = false;
    syncLinesToStore();
  }

  function formatMs(ms: number): string {
    if (ms <= 0) return "--:--";
    const min = Math.floor(ms / 60000);
    const sec = Math.floor((ms % 60000) / 1000);
    const centis = Math.floor((ms % 1000) / 10);
    return `${min}:${sec.toString().padStart(2, "0")}.${centis.toString().padStart(2, "0")}`;
  }

  function formatInputTime(ms: number): string {
    const safe = Math.max(0, Math.round(ms));
    const min = Math.floor(safe / 60000);
    const sec = Math.floor((safe % 60000) / 1000);
    const centis = Math.floor((safe % 1000) / 10);
    return `${min}:${sec.toString().padStart(2, "0")}.${centis.toString().padStart(2, "0")}`;
  }

  function formatSignedMs(ms: number): string {
    const safe = Math.round(ms);
    return `${safe >= 0 ? "+" : ""}${safe} ms`;
  }

  function parseInputTime(value: string): number | null {
    const raw = value.trim().replace(",", ".");
    if (!raw) return null;
    const parts = raw.split(":").map((part) => part.trim());
    let seconds = 0;
    if (parts.length === 1) {
      seconds = Number(parts[0]);
    } else if (parts.length === 2) {
      seconds = Number(parts[0]) * 60 + Number(parts[1]);
    } else if (parts.length === 3) {
      seconds = Number(parts[0]) * 3600 + Number(parts[1]) * 60 + Number(parts[2]);
    } else {
      return null;
    }
    if (!Number.isFinite(seconds) || seconds < 0) return null;
    return Math.round(seconds * 1000);
  }

  function updateLineTime(idx: number, field: "start_ms" | "end_ms", value: string) {
    const parsed = parseInputTime(value);
    if (parsed === null || !lines[idx]) return;
    pushHistory();
    setLineRange(
      idx,
      field === "start_ms" ? parsed : lines[idx].start_ms,
      field === "end_ms" ? parsed : lines[idx].end_ms,
    );
    syncLinesToStore();
  }

  function setLineRange(idx: number, startMs: number, endMs: number) {
    const maxMs = timelineMaxMs;
    const start = Math.max(0, Math.min(maxMs - 1, Math.round(startMs)));
    const end = Math.max(start + 1, Math.min(maxMs, Math.round(endMs)));
    lines[idx] = {
      ...lines[idx],
      start_ms: start,
      end_ms: end,
      synced: true,
    };
  }

  function beginGlobalDelayChange() {
    if (globalDelayHistoryCaptured) return;
    pushHistory();
    globalDelayHistoryCaptured = true;
  }

  function finishGlobalDelayChange() {
    globalDelayHistoryCaptured = false;
  }

  function applyGlobalDelayValue(requestedMs: number) {
    if (!canShiftGlobally) return;
    const targetMs = Math.max(-10000, Math.min(10000, Math.round(requestedMs / 10) * 10));
    let deltaMs = targetMs - globalDelayMs;
    if (deltaMs === 0) return;

    const timedLines = lines.filter((line) => line.end_ms > line.start_ms);
    const earliestStart = timedLines.reduce(
      (min, line) => Math.min(min, line.start_ms),
      Number.POSITIVE_INFINITY,
    );
    if (!Number.isFinite(earliestStart)) return;

    deltaMs = Math.max(deltaMs, -earliestStart);
    if (deltaMs === 0) return;

    lines = lines.map((line) => {
      if (line.end_ms <= line.start_ms) return line;
      const duration = line.end_ms - line.start_ms;
      const start = Math.max(0, Math.round(line.start_ms + deltaMs));
      return {
        ...line,
        start_ms: start,
        end_ms: start + duration,
        synced: true,
      };
    });
    globalDelayMs += deltaMs;
    alignMsg = "";
    syncLinesToStore();
  }

  function handleGlobalDelayInput(event: Event) {
    if (!canShiftGlobally) return;
    beginGlobalDelayChange();
    applyGlobalDelayValue(Number((event.currentTarget as HTMLInputElement).value));
  }

  function resetGlobalDelay() {
    if (!canShiftGlobally || globalDelayMs === 0) return;
    pushHistory();
    applyGlobalDelayValue(0);
    globalDelayHistoryCaptured = false;
  }

  async function exportSubtitle() {
    if (lines.length === 0 || isExporting) return;
    const currentLines = lines.map(lineToLyric);
    lastLyricsKey = lyricsKey(currentLines);
    lyricsLines.set(currentLines);
    const baseName = $lyricsFileName ? $lyricsFileName.replace(/\.[^.]+$/, "") : "lyrics";

    isExporting = true;
    alignMsg = "";
    saveMsg = "";
    try {
      const filePath = await invoke<string | null>("save_lyrics_as_subtitle", {
        lines: currentLines,
        format: exportFormat,
        defaultFileName: `${baseName}_synced.${exportFormat}`,
      });
      if (!filePath) return;
      saveMsg = tSync("lyricsSync.status.saved", { path: filePath });
    } catch (err) {
      saveMsg = tSync("lyricsSync.status.saveFailed", { error: String(err) });
    } finally {
      isExporting = false;
    }
  }

  function percent(ms: number): number {
    return Math.max(0, Math.min(100, (ms / timelineMaxMs) * 100));
  }

  function barLeft(line: SyncLine): number {
    return percent(line.start_ms);
  }

  function barWidth(line: SyncLine): number {
    return Math.max(0.5, percent(Math.max(line.end_ms, line.start_ms + 1)) - percent(line.start_ms));
  }

  function startTimelineDrag(e: PointerEvent, idx: number, mode: DragMode) {
    const target = e.currentTarget as HTMLElement;
    const track = target.closest(".timeline-track") as HTMLElement | null;
    const rect = track?.getBoundingClientRect();
    if (!rect || rect.width <= 0 || !lines[idx]) return;
    e.preventDefault();
    e.stopPropagation();
    pushHistory();
    dragState = {
      idx,
      mode,
      startX: e.clientX,
      trackWidth: rect.width,
      baseStart: lines[idx].start_ms,
      baseEnd: lines[idx].end_ms,
    };
  }

  function handleTimelinePointerMove(e: PointerEvent) {
    if (!dragState) return;
    const deltaMs = ((e.clientX - dragState.startX) / dragState.trackWidth) * timelineMaxMs;
    const duration = Math.max(1, dragState.baseEnd - dragState.baseStart);
    if (dragState.mode === "start") {
      setLineRange(
        dragState.idx,
        Math.min(dragState.baseStart + deltaMs, dragState.baseEnd - 1),
        dragState.baseEnd,
      );
    } else if (dragState.mode === "end") {
      setLineRange(
        dragState.idx,
        dragState.baseStart,
        Math.max(dragState.baseEnd + deltaMs, dragState.baseStart + 1),
      );
    } else {
      setLineRange(
        dragState.idx,
        dragState.baseStart + deltaMs,
        dragState.baseStart + deltaMs + duration,
      );
    }
  }

  function stopTimelineDrag() {
    if (!dragState) return;
    dragState = null;
    syncLinesToStore();
  }

</script>

<svelte:window
  onpointermove={handleTimelinePointerMove}
  onpointerup={stopTimelineDrag}
  onpointercancel={stopTimelineDrag}
/>

<div class="sync-editor">
  {#if lines.length === 0}
    <div class="sync-empty">
      <p>{$t("lyricsSync.empty.title")}</p>
      <p class="hint">{$t("lyricsSync.empty.hint")}</p>
    </div>
  {:else}
    <div class="sync-toolbar">
      <button class="sync-btn" onclick={undo} disabled={undoStack.length === 0}>
        {$t("lyricsSync.action.undo")}
      </button>

      <div class="sync-toolbar-right">
        <select
          class="sync-select"
          bind:value={exportFormat}
          aria-label={$t("lyricsSync.export.format")}
          disabled={isExporting}
        >
          <option value="lrc">LRC</option>
          <option value="srt">SRT</option>
          <option value="ass">ASS</option>
        </select>
        <button class="sync-btn" onclick={exportSubtitle} disabled={isExporting}>
          {$t("lyricsSync.action.export")}
        </button>
      </div>
    </div>

    <div class="sync-help">
      <p>{$t("lyricsSync.help.manual")}</p>
    </div>

    <div class="global-delay-panel" class:disabled={!canShiftGlobally}>
      <div class="global-delay-header">
        <div>
          <strong>{$t("lyricsSync.globalDelay.title")}</strong>
          <span>{$t(canShiftGlobally ? "lyricsSync.globalDelay.hint" : "lyricsSync.globalDelay.disabled")}</span>
        </div>
        <span class="global-delay-value">{formatSignedMs(globalDelayMs)}</span>
      </div>
      <div class="global-delay-controls">
        <input
          id="global_delay"
          type="range"
          min="-10000"
          max="10000"
          step="10"
          value={globalDelayMs}
          disabled={!canShiftGlobally}
          aria-label={$t("lyricsSync.globalDelay.title")}
          oninput={handleGlobalDelayInput}
          onpointerup={finishGlobalDelayChange}
          onkeyup={finishGlobalDelayChange}
          onchange={finishGlobalDelayChange}
        />
        <button
          class="sync-btn"
          onclick={resetGlobalDelay}
          disabled={!canShiftGlobally || globalDelayMs === 0}
          title={$t("lyricsSync.globalDelay.reset.title")}
        >
          {$t("lyricsSync.globalDelay.reset.text")}
        </button>
      </div>
    </div>

    {#if alignMsg}
      <div class="save-msg">{alignMsg}</div>
    {:else if saveMsg}
      <div class="save-msg">{saveMsg}</div>
    {/if}

    <div class="sync-lines">
      {#each lines as line, i}
        <div
          class="sync-line"
          class:playing={i === playbackIdx}
          class:synced={line.synced}
          bind:this={lineEls[i]}
        >
          <div class="line-meta">
            <span class="sync-index">{(i + 1).toString().padStart(2, "0")}</span>
            <label>
              <span>{$t("lyricsSync.time.start")}</span>
              <input
                class="time-input"
                value={formatInputTime(line.start_ms)}
                onchange={(e) => updateLineTime(i, "start_ms", (e.currentTarget as HTMLInputElement).value)}
              />
            </label>
            <label>
              <span>{$t("lyricsSync.time.end")}</span>
              <input
                class="time-input"
                value={formatInputTime(line.end_ms)}
                onchange={(e) => updateLineTime(i, "end_ms", (e.currentTarget as HTMLInputElement).value)}
              />
            </label>
            <span class="compact-time">{formatMs(line.start_ms)}</span>
          </div>

          <div class="sync-text">
            <span>{line.text}</span>
            {#if line.translation}
              <span class="sync-translation">{line.translation}</span>
            {/if}
          </div>

          <div class="timeline-track" aria-label={$t("lyricsSync.timeline.aria")}>
            <div
              class="timeline-bar"
              style="left: {barLeft(line)}%; width: {barWidth(line)}%;"
              role="slider"
              tabindex="0"
              aria-valuemin={0}
              aria-valuemax={Math.round(timelineMaxMs / 1000)}
              aria-valuenow={Math.round(line.start_ms / 1000)}
              aria-label={$t("lyricsSync.timeline.moveHandle")}
              onpointerdown={(e) => startTimelineDrag(e, i, "move")}
            >
              <button
                class="timeline-handle start"
                aria-label={$t("lyricsSync.timeline.startHandle")}
                onpointerdown={(e) => startTimelineDrag(e, i, "start")}
              ></button>
              <button
                class="timeline-handle end"
                aria-label={$t("lyricsSync.timeline.endHandle")}
                onpointerdown={(e) => startTimelineDrag(e, i, "end")}
              ></button>
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .sync-editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-bg-surface);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  .sync-empty {
    text-align: center;
    color: var(--color-text-muted);
    padding: 30px 0;
  }

  .sync-empty .hint {
    font-size: 12px;
    margin-top: var(--space-xs);
    color: var(--color-text-faint);
  }

  .sync-toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-lg);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    min-width: 0;
  }

  .sync-toolbar-right {
    margin-left: auto;
    display: flex;
    gap: var(--space-sm);
    align-items: center;
  }

  .sync-btn,
  .sync-select {
    min-height: 30px;
    border: 1px solid var(--color-border-light);
    border-radius: var(--radius-sm);
    background: var(--color-bg-sidebar);
    color: var(--color-text);
    font-size: 13px;
  }

  .sync-btn,
  .sync-select {
    padding: 5px var(--space-md);
    white-space: nowrap;
  }

  .sync-btn {
    cursor: pointer;
    transition: all var(--transition-normal);
  }

  .sync-btn:hover:not(:disabled) {
    background: var(--color-bg-hover);
    border-color: var(--color-text-muted);
  }

  .sync-btn:disabled,
  .sync-select:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .save-msg {
    padding: var(--space-xs) var(--space-lg);
    font-size: 12px;
    color: var(--color-brand);
    background: #fdf8e8;
    text-align: center;
  }

  .sync-help {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px var(--space-lg);
    border-bottom: 1px solid var(--color-border);
    background: #fffaf0;
    color: var(--color-text-muted);
    font-size: 12px;
    line-height: 1.5;
  }

  .sync-help p {
    margin: 0;
  }

  .global-delay-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px var(--space-lg);
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-sidebar);
  }

  .global-delay-panel.disabled {
    opacity: 0.65;
  }

  .global-delay-header {
    display: flex;
    justify-content: space-between;
    gap: var(--space-md);
    align-items: flex-start;
    min-width: 0;
  }

  .global-delay-header > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .global-delay-header strong {
    font-size: 13px;
    color: var(--color-text);
  }

  .global-delay-header span {
    font-size: 12px;
    color: var(--color-text-muted);
    line-height: 1.45;
  }

  .global-delay-value {
    flex-shrink: 0;
    min-width: 82px;
    text-align: right;
    font-family: var(--font-mono);
    color: var(--color-brand);
    font-size: 13px;
  }

  .global-delay-controls {
    display: grid;
    grid-template-columns: minmax(120px, 1fr) auto;
    gap: var(--space-sm);
    align-items: center;
  }

  .global-delay-controls input[type="range"] {
    width: 100%;
    accent-color: var(--color-brand);
  }

  .sync-lines {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-md) var(--space-lg);
    scrollbar-width: thin;
    scrollbar-color: var(--color-border-light) transparent;
  }

  .sync-line {
    display: grid;
    grid-template-columns: minmax(180px, 210px) minmax(0, 1fr);
    gap: var(--space-sm) var(--space-md);
    padding: var(--space-sm);
    border-radius: var(--radius-sm);
    transition: background var(--transition-normal), color var(--transition-normal);
    color: var(--color-text-muted);
  }

  .sync-line.synced {
    color: var(--color-text);
  }

  .sync-line.playing {
    color: var(--color-brand);
    background: #fdf8e8;
  }

  .line-meta {
    display: grid;
    grid-template-columns: 28px 1fr 1fr;
    gap: 6px;
    align-items: end;
  }

  .sync-index {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-faint);
    padding-bottom: 6px;
  }

  .line-meta label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .line-meta label span {
    font-size: 10px;
    color: var(--color-text-muted);
  }

  .time-input {
    width: 100%;
    height: 24px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-bg-sidebar);
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 2px 5px;
  }

  .compact-time {
    display: none;
    font-family: var(--font-mono);
    font-size: 12px;
    color: inherit;
    opacity: 0.7;
  }

  .sync-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    line-height: 1.4;
  }

  .sync-text > span {
    overflow-wrap: anywhere;
  }

  .sync-translation {
    font-size: 12px;
    color: var(--color-text-muted);
    font-weight: 400;
  }

  .sync-line.playing .sync-translation {
    color: #9a8600;
  }

  .timeline-track {
    grid-column: 1 / -1;
    position: relative;
    height: 14px;
    border-radius: 7px;
    background: var(--color-bg-hover);
    overflow: visible;
  }

  .timeline-bar {
    position: absolute;
    top: 3px;
    height: 8px;
    min-width: 12px;
    border-radius: 4px;
    background: var(--color-accent);
    cursor: grab;
  }

  .timeline-bar:active {
    cursor: grabbing;
  }

  .timeline-handle {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 18px;
    border: 1px solid var(--color-brand);
    border-radius: 6px;
    background: #fff;
    transform: translateY(-50%);
    cursor: ew-resize;
    padding: 0;
  }

  .timeline-handle.start {
    left: -6px;
  }

  .timeline-handle.end {
    right: -6px;
  }

  @media (max-width: 760px) {
    .sync-toolbar {
      flex-wrap: wrap;
    }

    .sync-toolbar-right {
      width: 100%;
      margin-left: 0;
    }

    .sync-line {
      grid-template-columns: minmax(0, 1fr);
    }

    .line-meta {
      grid-template-columns: 28px 1fr 1fr;
    }

    .global-delay-header {
      flex-direction: column;
    }

    .global-delay-value {
      text-align: left;
    }

    .global-delay-controls {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
