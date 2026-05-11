<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { lyricsLines, lyricsFileName } from "../stores/lyrics";
  import { guideVocalPath } from "../stores/melody";
  import type { LyricLine } from "../stores/lyrics";
  import { t, tSync } from "../i18n";

  interface LyricsTranscriptAlignmentResult {
    lines: LyricLine[];
    transcript_segments: number;
    matched_lines: number;
    average_score: number;
    confidence: "high" | "medium" | "low" | string;
    detected_intro_ms?: number | null;
  }

  interface WhisperToolsStatus {
    runner_available: boolean;
    runner_path?: string | null;
    model_available: boolean;
    model_path?: string | null;
    model_size_bytes?: number | null;
  }

  interface LocalWhisperRunnerCandidate {
    runner_path: string;
    runner_sha256: string;
  }

  interface LocalWhisperModelCandidate {
    model_path: string;
    model_sha256: string;
    model_size_bytes: number;
  }

  interface WhisperModelOption {
    id: string;
    label: string;
    file_name: string;
    size_bytes: number;
    installed: boolean;
  }

  interface WhisperInstallProgress {
    percent: number;
    status: string;
    message: string;
  }

  interface WhisperTranscriptionResult {
    transcript_path: string;
    subtitle_path?: string | null;
    segment_count: number;
    model_path: string;
  }

  interface WhisperTranscriptionProgress {
    percent: number;
    status: string;
    message: string;
    elapsed_seconds: number;
    eta_seconds?: number | null;
  }

  type ExportFormat = "lrc" | "srt" | "ass";
  type WhisperLanguage = "auto" | "zh" | "en" | "ja" | "ko";

  let exportFormat = $state<ExportFormat>("lrc");
  let whisperLanguage = $state<WhisperLanguage>("auto");
  let whisperModelId = $state("large-v3-turbo-q5_0");
  let whisperModelOptions = $state<WhisperModelOption[]>([]);
  let whisperStatus = $state<WhisperToolsStatus | null>(null);
  let whisperRunnerInstallProgress = $state<WhisperInstallProgress | null>(null);
  let whisperModelInstallProgress = $state<WhisperInstallProgress | null>(null);
  let whisperTranscriptionProgress = $state<WhisperTranscriptionProgress | null>(null);
  let whisperTranscriptionProgressUpdatedAt = $state(0);
  let whisperTranscriptionTick = $state(0);
  let whisperTranscriptionTimer: number | null = null;
  let isWhisperChecking = $state(false);
  let isWhisperConfiguring = $state(false);
  let isWhisperRunnerInstalling = $state(false);
  let isWhisperModelInstalling = $state(false);
  let isWhisperTranscribing = $state(false);
  let whisperMsg = $state("");
  let actionMsg = $state("");

  let unlistenWhisperRunnerInstall: UnlistenFn | null = null;
  let unlistenWhisperModelInstall: UnlistenFn | null = null;
  let unlistenWhisperTranscription: UnlistenFn | null = null;

  let hasLyrics = $derived($lyricsLines.length > 0);
  let activeAudioPath = $derived($guideVocalPath);
  let isWhisperInstalling = $derived(isWhisperRunnerInstalling || isWhisperModelInstalling);
  let isBusy = $derived(isWhisperTranscribing);
  let selectedWhisperModel = $derived(whisperModelOptions.find((option) => option.id === whisperModelId));
  let selectedWhisperModelActive = $derived(Boolean(
    selectedWhisperModel
      && whisperStatus?.model_available
      && fileBaseName(whisperStatus.model_path ?? "") === selectedWhisperModel.file_name,
  ));
  let selectedWhisperModelInstalled = $derived(Boolean(selectedWhisperModel?.installed));
  let whisperReady = $derived(Boolean(whisperStatus?.runner_available && selectedWhisperModelActive));
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
  let whisperStatusText = $derived.by(() => {
    const translate = $t;
    if (isWhisperChecking) return translate("lyricsSync.whisper.status.checking");
    if (!whisperStatus?.runner_available) return translate("lyricsSync.whisper.status.needRunner");
    if (selectedWhisperModel && !selectedWhisperModelInstalled) {
      return translate("lyricsSync.whisper.status.needSelectedModel");
    }
    if (selectedWhisperModel && selectedWhisperModelInstalled && !selectedWhisperModelActive) {
      return translate("lyricsSync.whisper.status.activatingModel");
    }
    if (!whisperStatus?.model_available) return translate("lyricsSync.whisper.status.needModel");
    const modelName = fileBaseName(whisperStatus.model_path ?? "");
    const size = formatBytes(whisperStatus.model_size_bytes ?? 0);
    return translate("lyricsSync.whisper.status.ready", { model: modelName, size });
  });
  let whisperTranscriptionEtaText = $derived.by(() => {
    const translate = $t;
    const progress = whisperTranscriptionProgress;
    void whisperTranscriptionTick;
    if (!isWhisperTranscribing || !progress) return "";
    const percent = Math.max(0, Math.min(100, Math.round(progress.percent)));
    if (percent >= 100) {
      return translate("lyricsSync.whisper.progress.complete");
    }
    const elapsedSinceUpdate = whisperTranscriptionProgressUpdatedAt > 0
      ? Math.floor((Date.now() - whisperTranscriptionProgressUpdatedAt) / 1000)
      : 0;
    const eta = progress.eta_seconds == null
      ? null
      : Math.max(0, Math.round(progress.eta_seconds - elapsedSinceUpdate));
    if (eta == null || percent <= 0) {
      return translate("lyricsSync.whisper.progress.estimating", { percent });
    }
    return translate("lyricsSync.whisper.progress.eta", {
      percent,
      eta: formatDuration(eta),
    });
  });

  function fileBaseName(path: string): string {
    return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  }

  function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return "-";
    if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  function formatDuration(seconds: number): string {
    const safeSeconds = Math.max(0, Math.round(seconds));
    const hours = Math.floor(safeSeconds / 3600);
    const minutes = Math.floor((safeSeconds % 3600) / 60);
    const secs = safeSeconds % 60;
    if (hours > 0) {
      return `${hours}:${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
    }
    return `${minutes}:${String(secs).padStart(2, "0")}`;
  }

  function startWhisperTranscriptionTimer() {
    stopWhisperTranscriptionTimer();
    whisperTranscriptionTick = Date.now();
    whisperTranscriptionTimer = window.setInterval(() => {
      whisperTranscriptionTick = Date.now();
    }, 1000);
  }

  function stopWhisperTranscriptionTimer() {
    if (whisperTranscriptionTimer !== null) {
      window.clearInterval(whisperTranscriptionTimer);
      whisperTranscriptionTimer = null;
    }
  }

  function applyTranscriptAlignment(result: LyricsTranscriptAlignmentResult) {
    lyricsLines.set(result.lines);
  }

  function activeWhisperModelOption(status: WhisperToolsStatus | null = whisperStatus) {
    const activeFileName = fileBaseName(status?.model_path ?? "");
    if (!activeFileName) return undefined;
    return whisperModelOptions.find((option) => option.file_name === activeFileName);
  }

  function syncWhisperModelSelectionToActive(status: WhisperToolsStatus | null = whisperStatus) {
    const activeOption = activeWhisperModelOption(status);
    if (activeOption) {
      whisperModelId = activeOption.id;
    }
  }

  async function refreshWhisperStatus(syncSelection = false) {
    isWhisperChecking = true;
    try {
      const status = await invoke<WhisperToolsStatus>("check_whisper_tools");
      whisperStatus = status;
      if (syncSelection) {
        syncWhisperModelSelectionToActive(status);
      }
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.selectFailed", { error: String(err) });
    } finally {
      isWhisperChecking = false;
    }
  }

  async function loadWhisperModelOptions(syncSelection = false) {
    try {
      const options = await invoke<WhisperModelOption[]>("list_whisper_model_options");
      whisperModelOptions = options;
      if (options.length > 0 && !options.some((option) => option.id === whisperModelId)) {
        whisperModelId = options[0].id;
      }
      if (syncSelection) {
        syncWhisperModelSelectionToActive();
      }
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.selectFailed", { error: String(err) });
    }
  }

  async function initializeWhisperTools() {
    await loadWhisperModelOptions();
    await refreshWhisperStatus(true);
    const option = whisperModelOptions.find((item) => item.id === whisperModelId);
    if (option?.installed && fileBaseName(whisperStatus?.model_path ?? "") !== option.file_name) {
      await activateInstalledWhisperModel(option.id);
    }
  }

  async function activateInstalledWhisperModel(modelId: string) {
    if (isWhisperConfiguring || isWhisperInstalling || isBusy) return;
    isWhisperConfiguring = true;
    whisperMsg = tSync("lyricsSync.whisper.status.activatingModel");
    try {
      const candidate = await invoke<LocalWhisperModelCandidate>("activate_installed_whisper_model", {
        modelId,
      });
      whisperMsg = tSync("lyricsSync.whisper.status.modelActivated", {
        model: fileBaseName(candidate.model_path),
        size: formatBytes(candidate.model_size_bytes),
      });
      await loadWhisperModelOptions();
      await refreshWhisperStatus();
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.selectFailed", { error: String(err) });
    } finally {
      isWhisperConfiguring = false;
    }
  }

  async function handleWhisperModelChange(event: Event) {
    const modelId = (event.currentTarget as HTMLSelectElement).value;
    whisperModelId = modelId;
    const option = whisperModelOptions.find((item) => item.id === modelId);
    if (option?.installed && fileBaseName(whisperStatus?.model_path ?? "") !== option.file_name) {
      await activateInstalledWhisperModel(modelId);
    } else {
      whisperMsg = "";
    }
  }

  async function installWhisperRunner() {
    if (isWhisperInstalling || isBusy) return;
    isWhisperRunnerInstalling = true;
    whisperRunnerInstallProgress = null;
    whisperMsg = tSync("lyricsSync.whisper.status.runnerDownloading");
    try {
      const candidate = await invoke<LocalWhisperRunnerCandidate>("install_whisper_runner");
      whisperMsg = tSync("lyricsSync.whisper.status.runnerInstalled", {
        runner: fileBaseName(candidate.runner_path),
      });
      await refreshWhisperStatus(true);
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.selectFailed", { error: String(err) });
    } finally {
      isWhisperRunnerInstalling = false;
    }
  }

  async function installWhisperModel() {
    if (isWhisperInstalling || isBusy) return;
    isWhisperModelInstalling = true;
    whisperModelInstallProgress = null;
    whisperMsg = tSync("lyricsSync.whisper.status.modelDownloading");
    try {
      const candidate = await invoke<LocalWhisperModelCandidate>("install_whisper_model", {
        modelId: whisperModelId,
      });
      whisperMsg = tSync("lyricsSync.whisper.status.modelInstalled", {
        model: fileBaseName(candidate.model_path),
        size: formatBytes(candidate.model_size_bytes),
      });
      await loadWhisperModelOptions();
      await refreshWhisperStatus();
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.selectFailed", { error: String(err) });
    } finally {
      isWhisperModelInstalling = false;
    }
  }

  async function selectWhisperModel() {
    if (isWhisperConfiguring || isWhisperInstalling || isBusy) return;
    const modelPath = await open({
      multiple: false,
      title: tSync("lyricsSync.whisper.dialog.modelTitle"),
      filters: [{ name: tSync("lyricsSync.whisper.dialog.modelFilter"), extensions: ["bin", "gguf"] }],
    });
    if (!modelPath || Array.isArray(modelPath)) return;

    isWhisperConfiguring = true;
    whisperMsg = "";
    try {
      const candidate = await invoke<LocalWhisperModelCandidate>("inspect_local_whisper_model_path", {
        path: modelPath,
      });
      const trusted = await invoke<LocalWhisperModelCandidate>("trust_local_whisper_model", { candidate });
      whisperMsg = tSync("lyricsSync.whisper.status.modelTrusted", {
        model: fileBaseName(trusted.model_path),
        size: formatBytes(trusted.model_size_bytes),
      });
      await loadWhisperModelOptions();
      await refreshWhisperStatus(true);
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.selectFailed", { error: String(err) });
    } finally {
      isWhisperConfiguring = false;
    }
  }

  async function openWhisperModelFolder() {
    if (isBusy) return;
    try {
      const paths = await invoke<string[]>("open_whisper_model_folder");
      whisperMsg = tSync("lyricsSync.whisper.status.modelFolderOpened", {
        path: paths.join(" / "),
      });
    } catch (err) {
      whisperMsg = tSync("lyricsSync.whisper.status.modelFolderOpenFailed", { error: String(err) });
    }
  }

  async function transcribeVocalsWithWhisper() {
    if (!hasLyrics || isBusy) return;
    if (!activeAudioPath) {
      actionMsg = tSync("lyricsSync.status.needVocals");
      return;
    }
    if (!whisperReady) {
      whisperMsg = tSync("lyricsSync.whisper.status.needSetup");
      return;
    }

    isWhisperTranscribing = true;
    whisperTranscriptionProgress = {
      percent: 0,
      status: "preparing",
      message: "",
      elapsed_seconds: 0,
      eta_seconds: null,
    };
    whisperTranscriptionProgressUpdatedAt = Date.now();
    startWhisperTranscriptionTimer();
    whisperMsg = "";
    actionMsg = tSync("lyricsSync.status.whisperTranscribing");
    try {
      const transcript = await invoke<WhisperTranscriptionResult>("transcribe_vocals_with_whisper", {
        audioPath: activeAudioPath,
        language: whisperLanguage,
      });
      const result = await invoke<LyricsTranscriptAlignmentResult>("align_lyrics_to_timed_transcript", {
        transcriptPath: transcript.transcript_path,
        lines: $lyricsLines,
        audioPath: activeAudioPath,
      });
      applyTranscriptAlignment(result);
      actionMsg = tSync("lyricsSync.status.whisperTranscribedAligned", {
        segments: transcript.segment_count,
        matched: result.matched_lines,
        total: result.lines.length,
        confidence: tSync(`lyricsSync.confidence.${result.confidence}`),
        intro: formatDuration((result.detected_intro_ms ?? 0) / 1000),
      });
    } catch (err) {
      actionMsg = tSync("lyricsSync.status.whisperTranscribeFailed", { error: String(err) });
    } finally {
      isWhisperTranscribing = false;
      stopWhisperTranscriptionTimer();
    }
  }

  async function exportSubtitle() {
    if (!hasLyrics) return;
    const baseName = $lyricsFileName ? $lyricsFileName.replace(/\.[^.]+$/, "") : "lyrics";
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
    }
  }

  onMount(() => {
    void (async () => {
      unlistenWhisperRunnerInstall = await listen<WhisperInstallProgress>(
        "whisper:runner_install_progress",
        (event) => {
          whisperRunnerInstallProgress = event.payload;
          whisperMsg = event.payload.message;
        },
      );
      unlistenWhisperModelInstall = await listen<WhisperInstallProgress>(
        "whisper:model_install_progress",
        (event) => {
          whisperModelInstallProgress = event.payload;
          whisperMsg = event.payload.message;
        },
      );
      unlistenWhisperTranscription = await listen<WhisperTranscriptionProgress>(
        "whisper:transcription_progress",
        (event) => {
          whisperTranscriptionProgress = event.payload;
          whisperTranscriptionProgressUpdatedAt = Date.now();
        },
      );
    })();
    void initializeWhisperTools();
  });

  onDestroy(() => {
    stopWhisperTranscriptionTimer();
    unlistenWhisperRunnerInstall?.();
    unlistenWhisperModelInstall?.();
    unlistenWhisperTranscription?.();
  });
</script>

<div class="lyrics-prep-tools">
  <div class="tool-row">
    <div class="tool-status">
      <span>{$t("lyricsSync.whisper.title")}</span>
      <strong>{whisperStatusText}</strong>
    </div>
    <div class="tool-status timing-status">
      <span>{$t("lyricsSync.timingStatus.title")}</span>
      <strong>{lyricsTimingStatusText}</strong>
    </div>
    <div class="tool-actions">
      {#if !whisperStatus?.runner_available}
        <button class="tool-btn" onclick={installWhisperRunner} disabled={isWhisperConfiguring || isWhisperInstalling || isBusy}>
          {isWhisperRunnerInstalling ? $t("lyricsSync.whisper.action.downloadingRunner") : $t("lyricsSync.whisper.action.downloadRunner")}
        </button>
      {/if}
      <button class="tool-btn" onclick={selectWhisperModel} disabled={isWhisperConfiguring || isWhisperInstalling || isBusy}>
        {$t("lyricsSync.whisper.action.model")}
      </button>
      <select
        class="tool-select model-select"
        bind:value={whisperModelId}
        onchange={handleWhisperModelChange}
        aria-label={$t("lyricsSync.whisper.modelPreset")}
        disabled={isWhisperConfiguring || isWhisperInstalling || isBusy}
      >
        {#each whisperModelOptions as option}
          <option value={option.id}>{option.label}</option>
        {/each}
      </select>
      <button class="tool-btn" onclick={installWhisperModel} disabled={isWhisperConfiguring || isWhisperInstalling || isBusy || whisperModelOptions.length === 0 || selectedWhisperModelInstalled}>
        {#if isWhisperModelInstalling}
          {$t("lyricsSync.whisper.action.downloadingModel")}
        {:else if selectedWhisperModelInstalled}
          {$t("lyricsSync.whisper.action.downloadedModel")}
        {:else}
          {$t("lyricsSync.whisper.action.downloadModel")}
        {/if}
      </button>
      <button class="tool-btn" onclick={openWhisperModelFolder} disabled={isBusy}>
        {$t("lyricsSync.whisper.action.openModelFolder")}
      </button>
      <button class="tool-btn" onclick={() => refreshWhisperStatus(true)} disabled={isWhisperConfiguring || isWhisperChecking || isWhisperInstalling || isBusy}>
        {$t("lyricsSync.whisper.action.refresh")}
      </button>
    </div>
  </div>
  <p class="model-help">{$t("lyricsSync.whisper.modelHelp")}</p>

  <div class="tool-row compact">
    <div class="tool-actions">
      <select class="tool-select" bind:value={whisperLanguage} aria-label={$t("lyricsSync.whisper.language")}>
        <option value="auto">{$t("lyricsSync.whisper.language.auto")}</option>
        <option value="zh">{$t("lyricsSync.whisper.language.zh")}</option>
        <option value="en">{$t("lyricsSync.whisper.language.en")}</option>
        <option value="ja">{$t("lyricsSync.whisper.language.ja")}</option>
        <option value="ko">{$t("lyricsSync.whisper.language.ko")}</option>
      </select>
      <button class="tool-btn primary" onclick={transcribeVocalsWithWhisper} disabled={!hasLyrics || isBusy || isWhisperConfiguring || isWhisperInstalling || !whisperReady}>
        {isWhisperTranscribing ? $t("lyricsSync.action.whisperTranscribing") : $t("lyricsSync.action.whisperTranscribe")}
      </button>
    </div>
    <div class="tool-actions export-actions">
      <select class="tool-select" bind:value={exportFormat} aria-label={$t("lyricsSync.export.format")}>
        <option value="lrc">LRC</option>
        <option value="srt">SRT</option>
        <option value="ass">ASS</option>
      </select>
      <button class="tool-btn" onclick={exportSubtitle} disabled={!hasLyrics || isBusy}>
        {$t("lyricsSync.action.export")}
      </button>
    </div>
  </div>

  {#if whisperRunnerInstallProgress || whisperModelInstallProgress}
    <div class="progress-list">
      {#if whisperRunnerInstallProgress}
        <div>
          <span>{$t("lyricsSync.whisper.progress.runner")}</span>
          <progress max="100" value={Math.round(whisperRunnerInstallProgress.percent)}></progress>
          <span>{Math.round(whisperRunnerInstallProgress.percent)}%</span>
        </div>
      {/if}
      {#if whisperModelInstallProgress}
        <div>
          <span>{$t("lyricsSync.whisper.progress.model")}</span>
          <progress max="100" value={Math.round(whisperModelInstallProgress.percent)}></progress>
          <span>{Math.round(whisperModelInstallProgress.percent)}%</span>
        </div>
      {/if}
    </div>
  {/if}

  {#if isWhisperTranscribing && whisperTranscriptionProgress}
    <div class="progress-list">
      <div>
        <span>{$t("lyricsSync.whisper.progress.transcription")}</span>
        <progress max="100" value={Math.round(whisperTranscriptionProgress.percent)}></progress>
        <span>{Math.round(whisperTranscriptionProgress.percent)}%</span>
      </div>
      <p class="progress-eta">{whisperTranscriptionEtaText}</p>
    </div>
  {/if}

  {#if whisperMsg || actionMsg}
    <p class="tool-message">{whisperMsg || actionMsg}</p>
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

  .tool-row.compact {
    align-items: flex-start;
  }

  .tool-status {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 180px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .tool-status strong {
    color: var(--color-text);
    font-size: 13px;
    overflow-wrap: anywhere;
  }

  .timing-status {
    min-width: 150px;
  }

  .model-help {
    margin: -2px 0 0;
    color: var(--color-text-muted);
    font-size: 12px;
    line-height: 1.45;
  }

  .tool-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
    min-width: 0;
  }

  .export-actions {
    margin-left: auto;
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

  .tool-btn.primary {
    border-color: var(--color-brand);
    background: var(--color-brand);
    color: #fff;
  }

  .tool-select {
    max-width: 260px;
    padding: 0 8px;
  }

  .model-select {
    width: min(260px, 100%);
  }

  .progress-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .progress-list > div {
    display: grid;
    grid-template-columns: 80px minmax(120px, 1fr) 42px;
    gap: 8px;
    align-items: center;
  }

  .progress-list progress {
    width: 100%;
    height: 8px;
    accent-color: var(--color-brand);
  }

  .progress-eta {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .tool-message {
    margin: 0;
    color: var(--color-brand);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  @media (max-width: 760px) {
    .tool-row {
      align-items: flex-start;
      flex-direction: column;
    }

    .tool-actions,
    .export-actions {
      justify-content: flex-start;
      margin-left: 0;
      width: 100%;
    }

    .tool-select {
      max-width: 100%;
    }
  }
</style>
