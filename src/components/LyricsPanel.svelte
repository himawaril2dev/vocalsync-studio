<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { lyricsLines, currentLyricIndex } from "../stores/lyrics";
  import { setLoopRange, clearLoop, loopA, loopB } from "../stores/transport";
  import { showToast } from "../stores/toast";
  import { t, tSync } from "../i18n";

  let containerEl = $state<HTMLDivElement | null>(null);
  let lineEls = $state<HTMLDivElement[]>([]);

  // 當前行變化時，自動捲動到中央
  $effect(() => {
    const idx = $currentLyricIndex;
    if (idx < 0 || !containerEl || !lineEls[idx]) return;
    lineEls[idx].scrollIntoView({
      behavior: "smooth",
      block: "center",
    });
  });

  /** 點擊歌詞行 → 跳到該行的起始時間 */
  function seekToLine(index: number) {
    const line = $lyricsLines[index];
    if (!line || line.start_ms <= 0) return;
    invoke("seek", { seconds: line.start_ms / 1000 });
  }

  /** 判斷某行是否正在被循環 */
  function isLineLooping(index: number): boolean {
    const line = $lyricsLines[index];
    if (!line || line.start_ms <= 0 || line.end_ms <= 0) return false;
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
    if (!line || line.start_ms <= 0 || line.end_ms <= 0) return;

    if (isLineLooping(index)) {
      await clearLoop();
      showToast(tSync("lyricsPanel.toast.loopCancelled"), "info");
    } else {
      await setLoopRange(line.start_ms / 1000, line.end_ms / 1000);
      invoke("seek", { seconds: line.start_ms / 1000 });
      showToast(tSync("lyricsPanel.toast.loopStarted"), "success");
    }
  }
</script>

<div class="lyrics-panel" bind:this={containerEl}>
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
        class:clickable={line.start_ms > 0}
        class:looping={isLineLooping(i)}
        bind:this={lineEls[i]}
        onclick={() => seekToLine(i)}
        onkeydown={(e) => { if (e.code === "Enter" || e.code === "Space") { e.preventDefault(); seekToLine(i); } }}
        role="button"
        tabindex={line.start_ms > 0 ? 0 : -1}
      >
        <div class="lyric-content">
          <div class="lyric-texts">
            <span class="lyric-text">{line.text}</span>
            {#if line.translation}
              <span class="lyric-translation">{line.translation}</span>
            {/if}
          </div>
          {#if line.start_ms > 0 && line.end_ms > 0}
            <button
              class="loop-btn"
              class:loop-active={isLineLooping(i)}
              title={isLineLooping(i) ? $t("lyricsPanel.loop.cancel") : $t("lyricsPanel.loop.set")}
              onclick={(e) => toggleLineLoop(i, e)}
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
    {/each}
    <div class="lyrics-spacer"></div>
  {/if}
</div>

<style>
  .lyrics-panel {
    background: var(--color-bg-surface);
    border-radius: var(--radius-lg);
    padding: var(--space-lg) var(--space-xl);
    overflow-y: auto;
    height: 100%;
    scroll-behavior: smooth;
    scrollbar-width: thin;
    scrollbar-color: var(--color-border-light) transparent;
  }

  .lyrics-panel::-webkit-scrollbar {
    width: 6px;
  }

  .lyrics-panel::-webkit-scrollbar-thumb {
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
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
  }

  .lyric-texts {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
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
