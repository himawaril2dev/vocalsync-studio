<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import {
    lyricsLines,
    currentLyricIndex,
    setLyricBoundary,
    type LyricBoundary,
    type LyricLine,
  } from "../stores/lyrics";
  import { setLoopRange, clearLoop, loopA, loopB, elapsed } from "../stores/transport";
  import { showToast } from "../stores/toast";
  import { t, tSync } from "../i18n";

  let containerEl = $state<HTMLDivElement | null>(null);
  let lineEls = $state<HTMLDivElement[]>([]);
  let globalDelayMs = $state(0);
  let timedLineCount = $derived(
    $lyricsLines.filter((line) => line.end_ms > line.start_ms).length,
  );
  let canShiftGlobally = $derived(timedLineCount > 0);

  // 當前行變化時，自動捲動到中央
  $effect(() => {
    const idx = $currentLyricIndex;
    if (idx < 0 || !containerEl || !lineEls[idx]) return;
    lineEls[idx].scrollIntoView({
      behavior: "smooth",
      block: "center",
    });
  });

  $effect(() => {
    if ($lyricsLines.length === 0) {
      globalDelayMs = 0;
    }
  });

  /** 點擊歌詞行 → 跳到該行的起始時間 */
  function seekToLine(index: number) {
    const line = $lyricsLines[index];
    if (!line || !hasSeekTarget(line)) return;
    invoke("seek", { seconds: line.start_ms / 1000 });
  }

  function hasSeekTarget(line: LyricLine): boolean {
    return line.start_ms > 0 || line.end_ms > line.start_ms;
  }

  /** 判斷某行是否正在被循環 */
  function isLineLooping(index: number): boolean {
    const line = $lyricsLines[index];
    if (!line || line.end_ms <= line.start_ms) return false;
    const a = $loopA;
    const b = $loopB;
    if (a === null || b === null) return false;
    return Math.abs(a - line.start_ms / 1000) < 0.05
        && Math.abs(b - line.end_ms / 1000) < 0.05;
  }

  /** 點擊循環按鈕 → 設定或取消該行的 A-B 循環 */
  async function toggleLineLoop(index: number, event: MouseEvent) {
    event.stopPropagation();
    const line = $lyricsLines[index];
    if (!line || line.end_ms <= line.start_ms) return;

    if (isLineLooping(index)) {
      await clearLoop();
      showToast(tSync("lyricsPanel.toast.loopCancelled"), "info");
    } else {
      await setLoopRange(line.start_ms / 1000, line.end_ms / 1000);
      invoke("seek", { seconds: line.start_ms / 1000 });
      showToast(tSync("lyricsPanel.toast.loopStarted"), "success");
    }
  }

  function formatSyncTime(ms: number): string {
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

  function applyGlobalDelayValue(requestedMs: number) {
    if (!canShiftGlobally) return;

    const targetMs = Math.max(-10000, Math.min(10000, Math.round(requestedMs / 10) * 10));
    let deltaMs = targetMs - globalDelayMs;
    if (deltaMs === 0) return;

    let appliedDeltaMs = 0;
    lyricsLines.update((lines) => {
      const timedLines = lines.filter((line) => line.end_ms > line.start_ms);
      if (timedLines.length === 0) return lines;

      const earliestStart = timedLines.reduce(
        (min, line) => Math.min(min, line.start_ms),
        Number.POSITIVE_INFINITY,
      );
      if (!Number.isFinite(earliestStart)) return lines;

      deltaMs = Math.max(deltaMs, -earliestStart);
      if (deltaMs === 0) return lines;

      appliedDeltaMs = deltaMs;
      return lines.map((line) => {
        if (line.end_ms <= line.start_ms) return line;
        const duration = line.end_ms - line.start_ms;
        const startMs = Math.max(0, Math.round(line.start_ms + deltaMs));
        return {
          ...line,
          start_ms: startMs,
          end_ms: startMs + duration,
        };
      });
    });

    if (appliedDeltaMs !== 0) {
      globalDelayMs += appliedDeltaMs;
    }
  }

  function handleGlobalDelayInput(event: Event) {
    applyGlobalDelayValue(Number((event.currentTarget as HTMLInputElement).value));
  }

  function resetGlobalDelay() {
    if (!canShiftGlobally || globalDelayMs === 0) return;
    applyGlobalDelayValue(0);
  }

  function handleLyricLineKeydown(index: number, event: KeyboardEvent) {
    if (event.target instanceof HTMLElement && event.target.closest("button")) return;

    if (event.code === "Enter") {
      event.preventDefault();
      seekToLine(index);
      return;
    }

    if (event.code === "Space") {
      event.preventDefault();
    }
  }

  function suppressSpaceButtonActivation(event: KeyboardEvent) {
    if (event.code === "Space") {
      event.preventDefault();
    }
  }

  function syncLineBoundary(index: number, boundary: LyricBoundary, event: MouseEvent) {
    event.stopPropagation();
    const result = setLyricBoundary(index, boundary, $elapsed * 1000);
    if (result.ok) {
      showToast(
        tSync(
          boundary === "start"
            ? "lyricsPanel.sync.toast.startSet"
            : "lyricsPanel.sync.toast.endSet",
          { time: formatSyncTime(result.time_ms) },
        ),
        "success",
        1600,
      );
      return;
    }

    showToast(
      tSync(
        result.reason === "end_before_start"
          ? "lyricsPanel.sync.toast.endBeforeStart"
          : "lyricsPanel.sync.toast.lineMissing",
      ),
      "warning",
    );
  }
</script>

<div class="lyrics-panel">
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
        type="range"
        min="-10000"
        max="10000"
        step="10"
        value={globalDelayMs}
        disabled={!canShiftGlobally}
        aria-label={$t("lyricsSync.globalDelay.title")}
        oninput={handleGlobalDelayInput}
      />
      <button
        type="button"
        class="global-delay-reset"
        onclick={resetGlobalDelay}
        disabled={!canShiftGlobally || globalDelayMs === 0}
        title={$t("lyricsSync.globalDelay.reset.title")}
        onkeydown={suppressSpaceButtonActivation}
        onkeyup={suppressSpaceButtonActivation}
      >
        {$t("lyricsSync.globalDelay.reset.text")}
      </button>
    </div>
  </div>

  <div class="lyrics-scroll" bind:this={containerEl}>
    {#if $lyricsLines.length === 0}
      <div class="lyrics-empty">
        <p>{$t("lyricsPanel.empty.title")}</p>
        <p class="hint">{$t("lyricsPanel.empty.hint")}</p>
      </div>
    {:else}
      <div class="lyrics-spacer"></div>
      {#each $lyricsLines as line, i}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="lyric-line"
          class:active={i === $currentLyricIndex}
          class:past={i < $currentLyricIndex}
          class:clickable={hasSeekTarget(line)}
          class:looping={isLineLooping(i)}
          bind:this={lineEls[i]}
          onclick={() => seekToLine(i)}
          onkeydown={(e) => handleLyricLineKeydown(i, e)}
          role="button"
          tabindex={hasSeekTarget(line) ? 0 : -1}
        >
          <div class="lyric-content">
            <div class="line-actions" aria-label={$t("lyricsPanel.sync.actions.aria")}>
              <button
                type="button"
                class="sync-boundary-btn"
                title={$t("lyricsPanel.sync.start.title")}
                aria-label={$t("lyricsPanel.sync.start.title")}
                onclick={(e) => syncLineBoundary(i, "start", e)}
                onkeydown={suppressSpaceButtonActivation}
                onkeyup={suppressSpaceButtonActivation}
              >
                {$t("lyricsPanel.sync.start.label")}
              </button>
              <button
                type="button"
                class="sync-boundary-btn"
                title={$t("lyricsPanel.sync.end.title")}
                aria-label={$t("lyricsPanel.sync.end.title")}
                onclick={(e) => syncLineBoundary(i, "end", e)}
                onkeydown={suppressSpaceButtonActivation}
                onkeyup={suppressSpaceButtonActivation}
              >
                {$t("lyricsPanel.sync.end.label")}
              </button>
            </div>
            <div class="lyric-texts">
              <span class="lyric-text">{line.text}</span>
              {#if line.translation}
                <span class="lyric-translation">{line.translation}</span>
              {/if}
            </div>
            <div class="loop-slot">
              {#if line.end_ms > line.start_ms}
                <button
                  type="button"
                  class="loop-btn"
                  class:loop-active={isLineLooping(i)}
                  title={isLineLooping(i) ? $t("lyricsPanel.loop.cancel") : $t("lyricsPanel.loop.set")}
                  onclick={(e) => toggleLineLoop(i, e)}
                  onkeydown={suppressSpaceButtonActivation}
                  onkeyup={suppressSpaceButtonActivation}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="17 1 21 5 17 9" />
                    <path d="M3 11V9a4 4 0 0 1 4-4h14" />
                    <polyline points="7 23 3 19 7 15" />
                    <path d="M21 13v2a4 4 0 0 1-4 4H3" />
                  </svg>
                </button>
              {/if}
            </div>
          </div>
        </div>
      {/each}
      <div class="lyrics-spacer"></div>
    {/if}
  </div>
</div>

<style>
  .lyrics-panel {
    background: var(--color-bg-surface);
    border-radius: var(--radius-lg);
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .global-delay-panel {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px var(--space-lg) 10px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-sidebar);
  }

  .global-delay-panel.disabled {
    opacity: 0.62;
  }

  .global-delay-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-md);
    min-width: 0;
  }

  .global-delay-header > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .global-delay-header strong {
    color: var(--color-text);
    font-size: 13px;
    font-weight: 800;
  }

  .global-delay-header span {
    color: var(--color-text-muted);
    font-size: 12px;
    line-height: 1.45;
  }

  .global-delay-value {
    flex-shrink: 0;
    min-width: 76px;
    text-align: right;
    color: var(--color-brand);
    font-family: var(--font-mono);
    font-size: 13px;
    font-weight: 700;
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

  .global-delay-reset {
    min-height: 30px;
    padding: 5px var(--space-md);
    border: 1px solid var(--color-border-light);
    border-radius: var(--radius-sm);
    background: var(--color-bg-surface);
    color: var(--color-text);
    font-size: 13px;
    white-space: nowrap;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .global-delay-reset:hover:not(:disabled) {
    background: var(--color-bg-hover);
    border-color: var(--color-text-muted);
  }

  .global-delay-reset:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .lyrics-scroll {
    flex: 1;
    min-height: 0;
    padding: var(--space-lg) var(--space-xl);
    overflow-y: auto;
    scroll-behavior: smooth;
    scrollbar-width: thin;
    scrollbar-color: var(--color-border-light) transparent;
  }

  .lyrics-scroll::-webkit-scrollbar {
    width: 6px;
  }

  .lyrics-scroll::-webkit-scrollbar-thumb {
    background: var(--color-border-light);
    border-radius: 3px;
  }

  .lyrics-empty {
    text-align: center;
    color: var(--color-text-muted);
    padding: 30px 0;
  }

  .lyrics-empty .hint {
    font-size: 12px;
    margin-top: var(--space-xs);
    color: var(--color-text-faint);
  }

  .lyrics-spacer {
    height: 50px;
  }

  .lyric-line {
    text-align: center;
    padding: var(--space-sm);
    font-size: 16px;
    color: var(--color-text-muted);
    transition:
      color 0.3s ease,
      transform 0.3s ease,
      font-weight 0.3s ease;
    cursor: default;
    user-select: none;
  }

  .lyric-content {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr) 32px;
    align-items: center;
    gap: var(--space-sm);
  }

  .lyric-texts {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }

  .lyric-line.clickable {
    cursor: pointer;
  }

  .lyric-line.clickable:hover {
    background: var(--color-bg-hover);
    border-radius: var(--radius-sm);
  }

  .lyric-line.looping {
    background: var(--color-warning-bg);
    border-radius: var(--radius-sm);
  }

  .lyric-line.past {
    color: var(--color-text-faint);
    opacity: 0.6;
  }

  .lyric-line.active {
    color: var(--color-brand);
    font-size: 22px;
    font-weight: 700;
    transform: scale(1.05);
  }

  .loop-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    color: var(--color-text-faint);
    cursor: pointer;
    opacity: 0;
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }

  .line-actions {
    display: flex;
    align-items: center;
    justify-self: end;
    gap: 4px;
    opacity: 0.55;
    transition: opacity var(--transition-fast);
  }

  .lyric-line:hover .line-actions,
  .lyric-line:focus-within .line-actions {
    opacity: 1;
  }

  .sync-boundary-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 26px;
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--color-border-light);
    border-radius: var(--radius-sm);
    background: var(--color-bg-sidebar);
    color: var(--color-text-muted);
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .sync-boundary-btn:hover,
  .sync-boundary-btn:focus-visible {
    background: var(--color-bg-active);
    border-color: var(--color-brand);
    color: var(--color-brand);
    outline: none;
  }

  .loop-slot {
    width: 28px;
    display: flex;
    justify-content: flex-start;
  }

  .lyric-line:hover .loop-btn {
    opacity: 1;
  }

  .loop-btn:hover {
    background: var(--color-bg-active);
    color: var(--color-brand);
  }

  .loop-btn.loop-active {
    opacity: 1;
    color: var(--color-accent);
    background: var(--color-brand);
  }

  .lyric-translation {
    font-size: 13px;
    color: var(--color-text-muted);
    font-weight: 400;
  }

  .lyric-line.active .lyric-translation {
    font-size: 15px;
    color: #9a8600;
  }

  .lyric-line.past .lyric-translation {
    opacity: 0.5;
  }
</style>
