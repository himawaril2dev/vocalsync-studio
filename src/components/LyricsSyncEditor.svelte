<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { lyricsLines, lyricsFileName } from "../stores/lyrics";
  import { elapsed } from "../stores/transport";
  import type { LyricLine } from "../stores/lyrics";
  import { t, tSync } from "../i18n";

  /** 編輯中的歌詞行（帶有可修改的時間戳） */
  interface SyncLine {
    text: string;
    translation?: string;
    start_ms: number;
    end_ms: number;
    synced: boolean;
  }

  let lines = $state<SyncLine[]>([]);
  let currentIdx = $state(0);
  let undoStack = $state<{ idx: number; prev: SyncLine }[]>([]);
  let isSyncing = $state(false);
  let containerEl = $state<HTMLDivElement | null>(null);
  let lineEls = $state<HTMLDivElement[]>([]);
  let saveMsg = $state("");

  // 從 store 初始化（含第二次載入歌詞時重新 hydrate）
  let lastLyricsKey = $state("");
  $effect(() => {
    const storeLines = $lyricsLines;
    if (storeLines.length === 0) return;
    // 用歌詞文字內容生成簡易 key 判斷是否為新歌詞
    const key = storeLines.map((l) => l.text).join("|");
    if (key !== lastLyricsKey) {
      lastLyricsKey = key;
      lines = storeLines.map((l) => ({
        text: l.text,
        translation: l.translation,
        start_ms: l.start_ms,
        end_ms: l.end_ms,
        synced: l.start_ms > 0 || l.end_ms > 0,
      }));
      undoStack = [];
      isSyncing = false;
    }
  });

  // 自動捲動到當前行
  $effect(() => {
    const idx = currentIdx;
    if (idx >= 0 && lineEls[idx]) {
      lineEls[idx].scrollIntoView({ behavior: "smooth", block: "center" });
    }
  });

  function startSync() {
    // 若已有同步過的時間戳，先確認再清除
    const hasSynced = lines.some((l) => l.synced);
    if (hasSynced && !confirm(tSync("lyricsSync.confirm.resetSync"))) {
      return;
    }

    isSyncing = true;
    currentIdx = 0;
    undoStack = [];
    // 清空所有時間戳
    lines = lines.map((l) => ({
      ...l,
      start_ms: 0,
      end_ms: 0,
      synced: false,
    }));
  }

  function stopSync() {
    isSyncing = false;
  }

  function tapSync() {
    if (!isSyncing || currentIdx >= lines.length) return;

    const nowMs = Math.round($elapsed * 1000);

    // 保存 undo 記錄
    undoStack = [...undoStack, { idx: currentIdx, prev: { ...lines[currentIdx] } }];

    // 設定上一行的 end_ms（如果有的話）
    if (currentIdx > 0 && lines[currentIdx - 1].synced) {
      lines[currentIdx - 1] = {
        ...lines[currentIdx - 1],
        end_ms: nowMs,
      };
    }

    // 設定當前行的 start_ms
    lines[currentIdx] = {
      ...lines[currentIdx],
      start_ms: nowMs,
      end_ms: 0,
      synced: true,
    };

    // 移到下一行
    if (currentIdx < lines.length - 1) {
      currentIdx += 1;
    } else {
      // 最後一行：設定 end_ms 為 +5 秒
      lines[currentIdx] = {
        ...lines[currentIdx],
        end_ms: nowMs + 5000,
      };
      isSyncing = false;
    }
  }

  function undo() {
    if (undoStack.length === 0) return;

    const last = undoStack[undoStack.length - 1];
    undoStack = undoStack.slice(0, -1);

    lines[last.idx] = last.prev;
    currentIdx = last.idx;
  }

  function formatMs(ms: number): string {
    if (ms <= 0) return "--:--";
    const min = Math.floor(ms / 60000);
    const sec = Math.floor((ms % 60000) / 1000);
    const centis = Math.floor((ms % 1000) / 10);
    return `${min}:${sec.toString().padStart(2, "0")}.${centis.toString().padStart(2, "0")}`;
  }

  async function applyToStore() {
    const result: LyricLine[] = lines.map((l) => ({
      start_ms: l.start_ms,
      end_ms: l.end_ms,
      text: l.text,
      translation: l.translation,
    }));
    lyricsLines.set(result);
    saveMsg = tSync("lyricsSync.status.applied");
    setTimeout(() => (saveMsg = ""), 2000);
  }

  async function exportLrc() {
    const result: LyricLine[] = lines.map((l) => ({
      start_ms: l.start_ms,
      end_ms: l.end_ms,
      text: l.text,
      translation: l.translation,
    }));

    // 用原始檔名替換副檔名為 .lrc
    const baseName = $lyricsFileName
      ? $lyricsFileName.replace(/\.[^.]+$/, "")
      : "lyrics";

    try {
      const filePath = await save({
        title: tSync("lyricsSync.export.dialog.title"),
        filters: [{ name: "LRC", extensions: ["lrc"] }],
        defaultPath: `${baseName}_synced.lrc`,
      });
      if (!filePath) return;

      await invoke("save_lyrics_as_lrc", { lines: result, outputPath: filePath });
      saveMsg = tSync("lyricsSync.status.saved", { path: filePath });
    } catch (e) {
      saveMsg = tSync("lyricsSync.status.saveFailed", { error: String(e) });
    }
    setTimeout(() => (saveMsg = ""), 3000);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!isSyncing) return;

    if (e.code === "Space") {
      e.preventDefault();
      tapSync();
    } else if ((e.ctrlKey || e.metaKey) && e.code === "KeyZ") {
      e.preventDefault();
      undo();
    }
  }

  onMount(() => {
    window.addEventListener("keydown", handleKeydown);
  });

  onDestroy(() => {
    window.removeEventListener("keydown", handleKeydown);
  });
</script>

<div class="sync-editor" bind:this={containerEl}>
  {#if lines.length === 0}
    <div class="sync-empty">
      <p>{$t("lyricsSync.empty.title")}</p>
      <p class="hint">{$t("lyricsSync.empty.hint")}</p>
    </div>
  {:else}
    <!-- 工具列 -->
    <div class="sync-toolbar">
      {#if !isSyncing}
        <button class="sync-btn primary" onclick={startSync}>
          {$t("lyricsSync.action.start")}
        </button>
      {:else}
        <button class="sync-btn" onclick={stopSync}>
          {$t("lyricsSync.action.stop")}
        </button>
        <button class="sync-btn" onclick={undo} disabled={undoStack.length === 0}>
          {$t("lyricsSync.action.undo")}
        </button>
        <span class="sync-hint">{$t("lyricsSync.hint.spaceToMark")}</span>
      {/if}

      <div class="sync-toolbar-right">
        <button class="sync-btn" onclick={applyToStore} disabled={isSyncing}>
          {$t("lyricsSync.action.apply")}
        </button>
        <button class="sync-btn" onclick={exportLrc} disabled={isSyncing}>
          {$t("lyricsSync.action.export")}
        </button>
      </div>
    </div>

    {#if saveMsg}
      <div class="save-msg">{saveMsg}</div>
    {/if}

    <!-- 歌詞列表 -->
    <div class="sync-lines">
      {#each lines as line, i}
        <div
          class="sync-line"
          class:current={isSyncing && i === currentIdx}
          class:synced={line.synced}
          class:upcoming={isSyncing && i > currentIdx}
          bind:this={lineEls[i]}
        >
          <span class="sync-time">{formatMs(line.start_ms)}</span>
          <span class="sync-text">
            {line.text}
            {#if line.translation}
              <span class="sync-translation">{line.translation}</span>
            {/if}
          </span>
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
  }

  .sync-toolbar-right {
    margin-left: auto;
    display: flex;
    gap: var(--space-sm);
  }

  .sync-btn {
    padding: 5px var(--space-md);
    border: 1px solid var(--color-border-light);
    border-radius: var(--radius-sm);
    background: var(--color-bg-sidebar);
    color: var(--color-text);
    font-size: 13px;
    cursor: pointer;
    transition: all var(--transition-normal);
  }

  .sync-btn:hover:not(:disabled) {
    background: var(--color-bg-hover);
    border-color: var(--color-text-muted);
  }

  .sync-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .sync-btn.primary {
    background: var(--color-brand);
    color: #fff;
    border-color: var(--color-brand);
  }

  .sync-btn.primary:hover {
    background: var(--color-brand-hover);
  }

  .sync-hint {
    font-size: 12px;
    color: var(--color-text-muted);
    font-style: italic;
  }

  .save-msg {
    padding: var(--space-xs) var(--space-lg);
    font-size: 12px;
    color: var(--color-brand);
    background: #fdf8e8;
    text-align: center;
  }

  .sync-lines {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-md) var(--space-lg);
    scrollbar-width: thin;
    scrollbar-color: var(--color-border-light) transparent;
  }

  .sync-line {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
    padding: var(--space-sm);
    border-radius: var(--radius-sm);
    transition: all var(--transition-normal);
    color: var(--color-text-muted);
  }

  .sync-line.synced {
    color: var(--color-text);
  }

  .sync-line.current {
    color: var(--color-brand);
    font-weight: 700;
    font-size: 17px;
    background: #fdf8e8;
  }

  .sync-line.upcoming {
    color: var(--color-text-faint);
  }

  .sync-time {
    font-family: var(--font-mono);
    font-size: 12px;
    min-width: 60px;
    text-align: right;
    color: inherit;
    opacity: 0.7;
  }

  .sync-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .sync-translation {
    font-size: 12px;
    color: var(--color-text-muted);
    font-weight: 400;
  }

  .sync-line.current .sync-translation {
    color: #9a8600;
  }
</style>
