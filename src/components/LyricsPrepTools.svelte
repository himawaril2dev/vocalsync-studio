<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { lyricsLines, lyricsFileName } from "../stores/lyrics";
  import { t, tSync } from "../i18n";

  type ExportFormat = "lrc" | "srt" | "ass";

  let exportFormat = $state<ExportFormat>("lrc");
  let actionMsg = $state("");
  let isExporting = $state(false);

  let hasLyrics = $derived($lyricsLines.length > 0);
  let timedLyricCount = $derived($lyricsLines.filter((line) => line.end_ms > line.start_ms).length);
  let lyricsTimingStatusText = $derived.by(() => {
    const translate = $t;
    if (!hasLyrics) return translate("lyricsSync.timingStatus.empty");
    if (timedLyricCount === 0) return translate("lyricsSync.timingStatus.notAligned");
    return translate("lyricsSync.timingStatus.aligned", {
      count: timedLyricCount,
      total: $lyricsLines.length,
    });
  });

  async function exportSubtitle() {
    if (!hasLyrics || isExporting) return;
    const baseName = $lyricsFileName ? $lyricsFileName.replace(/\.[^.]+$/, "") : "lyrics";
    isExporting = true;
    actionMsg = "";
    try {
      const filePath = await invoke<string | null>("save_lyrics_as_subtitle", {
        lines: $lyricsLines,
        format: exportFormat,
        defaultFileName: `${baseName}_synced.${exportFormat}`,
      });
      if (!filePath) return;
      actionMsg = tSync("lyricsSync.status.saved", { path: filePath });
    } catch (err) {
      actionMsg = tSync("lyricsSync.status.saveFailed", { error: String(err) });
    } finally {
      isExporting = false;
    }
  }
</script>

<div class="lyrics-prep-tools">
  <div class="tool-row">
    <div class="tool-status">
      <span>{$t("lyricsSync.timingStatus.title")}</span>
      <strong>{lyricsTimingStatusText}</strong>
    </div>
    <div class="tool-actions export-actions">
      <select
        class="tool-select"
        bind:value={exportFormat}
        aria-label={$t("lyricsSync.export.format")}
        disabled={!hasLyrics || isExporting}
      >
        <option value="lrc">LRC</option>
        <option value="srt">SRT</option>
        <option value="ass">ASS</option>
      </select>
      <button class="tool-btn" onclick={exportSubtitle} disabled={!hasLyrics || isExporting}>
        {isExporting ? $t("lyricsSync.action.exporting") : $t("lyricsSync.action.export")}
      </button>
    </div>
  </div>
  <p class="tool-help">{$t("lyricsSync.export.help")}</p>
  {#if actionMsg}
    <p class="tool-message">{actionMsg}</p>
  {/if}
</div>

<style>
  .lyrics-prep-tools {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--color-border);
  }

  .tool-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }

  .tool-status {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 150px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .tool-status strong {
    color: var(--color-text);
    font-size: 13px;
    overflow-wrap: anywhere;
  }

  .tool-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
    min-width: 0;
  }

  .tool-btn,
  .tool-select {
    height: 32px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-bg-card);
    color: var(--color-text);
    font-size: 13px;
  }

  .tool-btn {
    padding: 0 12px;
    cursor: pointer;
    white-space: nowrap;
  }

  .tool-btn:hover:not(:disabled) {
    background: var(--color-bg-hover);
  }

  .tool-btn:disabled,
  .tool-select:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .tool-select {
    max-width: 160px;
    padding: 0 8px;
  }

  .tool-help,
  .tool-message {
    margin: 0;
    font-size: 12px;
    line-height: 1.45;
  }

  .tool-help {
    color: var(--color-text-muted);
  }

  .tool-message {
    color: var(--color-brand);
    overflow-wrap: anywhere;
  }

  @media (max-width: 760px) {
    .tool-row {
      align-items: flex-start;
      flex-direction: column;
    }

    .tool-actions {
      justify-content: flex-start;
      width: 100%;
    }

    .tool-select {
      max-width: 100%;
    }
  }
</style>
