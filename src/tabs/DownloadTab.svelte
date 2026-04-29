<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount, onDestroy } from "svelte";
  import { showToast } from "../stores/toast";
  import {
    toolStatus,
    downloadProgress,
    downloadStatus,
    lastResult,
    detectedUrlType,
    isDownloading,
    isInstalling,
    installProgress,
    isInstallingFfmpeg,
    ffmpegInstallProgress,
    resetDownloadState,
    type ToolStatus,
    type DownloadProgress,
    type DownloadResult,
    type DownloadFormat,
    type VideoQuality,
    type SubtitleLang,
    type InstallProgress,
    type LocalFfmpegCandidate,
    type LocalYtdlpCandidate,
    type UrlType,
  } from "../stores/download";
  import { lyricsLines, lyricsFileName, type LyricLine } from "../stores/lyrics";
  import { t, tSync } from "../i18n";

  // ── 表單狀態 ──────────────────────────────────────────────────

  let url = $state("");
  let format = $state<DownloadFormat>("mp3");
  let quality = $state<VideoQuality>("best");
  let subtitleLang = $state<SubtitleLang>("none");
  let outputDir = $state("");

  // ── 初始化 ────────────────────────────────────────────────────

  let unlistenProgress: UnlistenFn | null = null;
  let unlistenInstall: UnlistenFn | null = null;
  let unlistenFfmpegInstall: UnlistenFn | null = null;
  let localFfmpegCandidate = $state<LocalFfmpegCandidate | null>(null);
  let localFfmpegMessage = $state("");
  let isDetectingLocalFfmpeg = $state(false);
  let isTrustingLocalFfmpeg = $state(false);
  let localYtdlpCandidate = $state<LocalYtdlpCandidate | null>(null);
  let localYtdlpMessage = $state("");
  let isDetectingLocalYtdlp = $state(false);
  let isSelectingLocalYtdlp = $state(false);
  let isTrustingLocalYtdlp = $state(false);

  onMount(async () => {
    try {
    // 檢查工具狀態
    const status = await invoke<ToolStatus>("check_download_tools");
    toolStatus.set(status);

    // 預設輸出目錄：使用者桌面
    if (!outputDir) {
      const home = await getDefaultOutputDir();
      if (home) outputDir = home;
    }

    // 監聽下載進度 event
    unlistenProgress = await listen<DownloadProgress>("ytdlp:progress", (event) => {
      downloadProgress.set(event.payload);
      if (event.payload.status === "downloading" || event.payload.status === "postprocessing") {
        downloadStatus.set(event.payload.status);
      }
    });

    // 監聽安裝進度 event
    unlistenInstall = await listen<InstallProgress>("ytdlp:install_progress", (event) => {
      installProgress.set(event.payload);
    });

    // 監聽 FFmpeg 安裝進度
    unlistenFfmpegInstall = await listen<InstallProgress>("ffmpeg:install_progress", (event) => {
      ffmpegInstallProgress.set(event.payload);
    });
    } catch (e) {
      showToast(tSync("download.result.initFailed", { error: String(e) }), "error");
    }
  });

  onDestroy(() => {
    unlistenProgress?.();
    unlistenInstall?.();
    unlistenFfmpegInstall?.();
  });

  async function getDefaultOutputDir(): Promise<string | null> {
    try {
      const home = await invoke<string | null>("get_default_download_dir");
      return home;
    } catch (err) {
      console.warn("[download] 取得預設下載目錄失敗", err);
      return null;
    }
  }

  // ── URL 偵測 ──────────────────────────────────────────────────

  async function onUrlChange(): Promise<void> {
    if (!url.trim()) {
      detectedUrlType.set(null);
      return;
    }
    try {
      const type = await invoke<string>("detect_download_url_type", { url });
      detectedUrlType.set(type as UrlType);
    } catch (err) {
      console.warn("[download] URL 類型偵測失敗", err);
      detectedUrlType.set(null);
    }
  }

  async function resolveDownloadUrlType(): Promise<UrlType | null> {
    if ($detectedUrlType) return $detectedUrlType;
    try {
      const type = await invoke<string>("detect_download_url_type", { url });
      const typed = type as UrlType;
      detectedUrlType.set(typed);
      return typed;
    } catch {
      return null;
    }
  }

  function batchDownloadLimit(): number {
    return $toolStatus?.batch_download_limit ?? 25;
  }

  function confirmBatchDownload(type: UrlType): boolean {
    if (type === "video") return true;
    return window.confirm(
      tSync("download.confirm.batch", {
        type: urlTypeLabel(type),
        limit: batchDownloadLimit(),
      }),
    );
  }

  // ── 選擇輸出目錄 ─────────────────────────────────────────────

  async function selectOutputDir(): Promise<void> {
    const selected = await open({
      directory: true,
      title: tSync("download.form.outputDir.dialog"),
    });
    if (selected) {
      outputDir = selected as string;
    }
  }

  // ── 開始下載 ──────────────────────────────────────────────────

  async function startDownload(): Promise<void> {
    if (!url.trim()) return;
    if (!outputDir.trim()) {
      lastResult.set({
        success: false,
        message: tSync("download.result.noOutputDir"),
        output_dir: "",
        subtitle_paths: [],
      });
      return;
    }

    const urlType = await resolveDownloadUrlType();
    if (urlType && !confirmBatchDownload(urlType)) return;

    resetDownloadState();
    downloadStatus.set("downloading");

    try {
      const result = await invoke<DownloadResult>("start_download", {
        request: {
          url: url.trim(),
          format,
          quality,
          subtitle_lang: subtitleLang,
          output_dir: outputDir,
        },
      });

      lastResult.set(result);
      downloadStatus.set(result.success ? "finished" : "error");
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      lastResult.set({
        success: false,
        message,
        output_dir: outputDir,
        subtitle_paths: [],
      });
      downloadStatus.set("error");
    }
  }

  // ── 取消下載 ──────────────────────────────────────────────────

  async function cancelDownload(): Promise<void> {
    try {
      await invoke("cancel_download");
      downloadStatus.set("cancelled");
    } catch (err) {
      console.warn("[download] 取消下載失敗", err);
    }
  }

  // ── 安裝 yt-dlp ────────────────────────────────────────────────

  async function installYtdlp(): Promise<void> {
    isInstalling.set(true);
    installProgress.set(null);

    try {
      await invoke<string>("install_ytdlp");
      // 安裝完成後重新檢查工具狀態
      const status = await invoke<ToolStatus>("check_download_tools");
      toolStatus.set(status);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      installProgress.set({
        percent: 0,
        status: "error",
        message: tSync("download.result.installFailed", { error: message }),
      });
    } finally {
      isInstalling.set(false);
    }
  }

  async function detectLocalYtdlp(): Promise<void> {
    isDetectingLocalYtdlp = true;
    localYtdlpMessage = "";

    try {
      const candidate = await invoke<LocalYtdlpCandidate | null>("detect_local_ytdlp");
      localYtdlpCandidate = candidate;
      if (!candidate) {
        localYtdlpMessage = tSync("download.tool.ytdlp.localNotFound");
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      localYtdlpMessage = tSync("download.tool.ytdlp.localDetectFailed", { error: message });
    } finally {
      isDetectingLocalYtdlp = false;
    }
  }

  async function selectLocalYtdlp(): Promise<void> {
    isSelectingLocalYtdlp = true;
    localYtdlpMessage = "";

    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: tSync("download.tool.ytdlp.selectDialog"),
        filters: [{ name: tSync("download.tool.ytdlp.selectFilter"), extensions: ["exe"] }],
      });
      if (!selected || Array.isArray(selected)) return;

      localYtdlpCandidate = await invoke<LocalYtdlpCandidate>("inspect_local_ytdlp_path", {
        path: selected,
      });
      localYtdlpMessage = tSync("download.tool.ytdlp.manualSelected");
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      localYtdlpMessage = tSync("download.tool.ytdlp.manualSelectFailed", { error: message });
    } finally {
      isSelectingLocalYtdlp = false;
    }
  }

  async function trustLocalYtdlp(): Promise<void> {
    isTrustingLocalYtdlp = true;
    localYtdlpMessage = "";

    try {
      if (!localYtdlpCandidate) {
        localYtdlpMessage = tSync("download.tool.ytdlp.localNotFound");
        return;
      }
      localYtdlpCandidate = await invoke<LocalYtdlpCandidate>("trust_local_ytdlp", {
        candidate: localYtdlpCandidate,
      });
      const status = await invoke<ToolStatus>("check_download_tools");
      toolStatus.set(status);
      localYtdlpMessage = tSync("download.tool.ytdlp.localTrusted");
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      localYtdlpMessage = tSync("download.tool.ytdlp.localTrustFailed", { error: message });
    } finally {
      isTrustingLocalYtdlp = false;
    }
  }

  // ── 安裝 FFmpeg ───────────────────────────────────────────────

  async function installFfmpeg(): Promise<void> {
    isInstallingFfmpeg.set(true);
    ffmpegInstallProgress.set(null);

    try {
      await invoke<string>("install_ffmpeg");
      // 安裝完成後重新檢查工具狀態
      const status = await invoke<ToolStatus>("check_download_tools");
      toolStatus.set(status);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      ffmpegInstallProgress.set({
        percent: 0,
        status: "error",
        message: tSync("download.result.installFailed", { error: message }),
      });
    } finally {
      isInstallingFfmpeg.set(false);
    }
  }

  async function detectLocalFfmpeg(): Promise<void> {
    isDetectingLocalFfmpeg = true;
    localFfmpegMessage = "";

    try {
      const candidate = await invoke<LocalFfmpegCandidate | null>("detect_local_ffmpeg");
      localFfmpegCandidate = candidate;
      if (!candidate) {
        localFfmpegMessage = tSync("download.tool.ffmpeg.localNotFound");
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      localFfmpegMessage = tSync("download.tool.ffmpeg.localDetectFailed", { error: message });
    } finally {
      isDetectingLocalFfmpeg = false;
    }
  }

  async function trustLocalFfmpeg(): Promise<void> {
    isTrustingLocalFfmpeg = true;
    localFfmpegMessage = "";

    try {
      if (!localFfmpegCandidate) {
        localFfmpegMessage = tSync("download.tool.ffmpeg.localNotFound");
        return;
      }
      localFfmpegCandidate = await invoke<LocalFfmpegCandidate>("trust_local_ffmpeg", {
        candidate: localFfmpegCandidate,
      });
      const status = await invoke<ToolStatus>("check_download_tools");
      toolStatus.set(status);
      localFfmpegMessage = tSync("download.tool.ffmpeg.localTrusted");
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      localFfmpegMessage = tSync("download.tool.ffmpeg.localTrustFailed", { error: message });
    } finally {
      isTrustingLocalFfmpeg = false;
    }
  }

  // ── URL 類型標籤 ──────────────────────────────────────────────

  function urlTypeLabel(type: string | null): string {
    switch (type) {
      case "video": return tSync("download.urlType.video");
      case "playlist": return tSync("download.urlType.playlist");
      case "channel": return tSync("download.urlType.channel");
      default: return "";
    }
  }

  // ── 字幕載入為歌詞 ─────────────────────────────────────────────

  let subtitleLoadMsg = $state("");
  // 🟡 Y6 修正：顯示載入中狀態 + 🟡 Y3 修正：防止重複點擊
  let subtitleLoading = $state(false);

  /** 將字幕檔載入為歌詞（自動判斷 SRT / VTT / LRC） */
  async function loadSubtitleAsLyrics(path: string): Promise<void> {
    if (subtitleLoading) return; // Y3: 防止重複點擊 race condition
    subtitleLoading = true;
    subtitleLoadMsg = tSync("download.subtitle.loadingAsLyrics");
    try {
      const lines = await invoke<LyricLine[]>("load_lyrics", { path });
      lyricsLines.set(lines);
      // 取檔名部分作為歌詞檔名
      const parts = path.replace(/\\/g, "/").split("/");
      lyricsFileName.set(parts[parts.length - 1] ?? path);
      subtitleLoadMsg = tSync("download.subtitle.loadedAsLyrics", { count: lines.length });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      subtitleLoadMsg = tSync("download.subtitle.loadFailed", { error: message });
    } finally {
      subtitleLoading = false;
    }
  }

  /** 從路徑取檔名 */
  function basename(path: string): string {
    const parts = path.replace(/\\/g, "/").split("/");
    return parts[parts.length - 1] ?? path;
  }
</script>

<div class="download-tab">

  <!-- 工具狀態 -->
  {#if $toolStatus}
    <div class="tool-status">
      <div class="status-item" class:ok={$toolStatus.ytdlp_available} class:missing={!$toolStatus.ytdlp_available}>
        <span class="status-dot"></span>
        <span title={$toolStatus.ytdlp_path ?? ""}>yt-dlp {$toolStatus.ytdlp_available ? $toolStatus.ytdlp_version ?? "" : $t("download.tool.ytdlp.notInstalled")}</span>
        {#if $toolStatus.ytdlp_available && !$toolStatus.ytdlp_hash_matches_managed}
          <span
            class="trust-badge"
            title={$t("download.tool.ytdlp.localTrustedHint")}
          >{$t("download.tool.ytdlp.localTrustedBadge")}</span>
        {/if}
      </div>
      <div class="status-item" class:ok={$toolStatus.ffmpeg_available} class:missing={!$toolStatus.ffmpeg_available}>
        <span class="status-dot"></span>
        <span title={$toolStatus.ffmpeg_path ?? ""}>FFmpeg {$toolStatus.ffmpeg_available ? $t("download.tool.ffmpeg.installed") : $t("download.tool.ffmpeg.notInstalled")}</span>
      </div>
      <div class="tool-actions">
        <button
          class="btn btn-secondary btn-tool-update"
          onclick={detectLocalYtdlp}
          disabled={isDetectingLocalYtdlp || isSelectingLocalYtdlp || isTrustingLocalYtdlp || $isDownloading}
        >
          {isDetectingLocalYtdlp ? $t("download.tool.ytdlp.detectingLocal") : $t("download.tool.ytdlp.detectLocal")}
        </button>
        <button
          class="btn btn-secondary btn-tool-update"
          onclick={selectLocalYtdlp}
          disabled={isDetectingLocalYtdlp || isSelectingLocalYtdlp || isTrustingLocalYtdlp || $isDownloading}
        >
          {isSelectingLocalYtdlp ? $t("download.tool.ytdlp.selectingLocal") : $t("download.tool.ytdlp.selectLocal")}
        </button>
        {#if $toolStatus.ytdlp_available}
          <button
            class="btn btn-secondary btn-tool-update"
            onclick={installYtdlp}
            disabled={$isInstalling || $isDownloading || !$toolStatus.ytdlp_update_available}
            title={$isDownloading
              ? $t("download.tool.ytdlp.disabledDuringDownload")
              : $isInstalling
                ? $t("download.tool.ytdlp.updating")
                : $toolStatus.ytdlp_update_available
              ? $t("download.tool.ytdlp.updateHint", { version: $toolStatus.managed_ytdlp_version })
              : $t("download.tool.ytdlp.upToDateHint", { version: $toolStatus.managed_ytdlp_version })}
          >
            {$isInstalling
              ? $t("download.tool.ytdlp.updating")
              : $toolStatus.ytdlp_update_available
                ? $t("download.tool.ytdlp.update")
                : $t("download.tool.ytdlp.upToDate")}
          </button>
        {/if}
      </div>
    </div>

    {#if $toolStatus.ytdlp_available && $installProgress}
      <div class="install-status update-status">
        {#if $installProgress.status === "downloading"}
          <div class="progress-bar-container" style="margin-top: 8px">
            <div
              class="progress-bar-fill"
              style="width: {Math.min($installProgress.percent, 100)}%"
            ></div>
          </div>
        {/if}
        <span class="install-message">{$installProgress.message}</span>
      </div>
    {/if}

    {#if localYtdlpCandidate || localYtdlpMessage}
      <div class="local-tool-card">
        {#if localYtdlpCandidate}
          <p class="local-tool-title">{$t("download.tool.ytdlp.localFound")}</p>
          <code>{localYtdlpCandidate.ytdlp_path}</code>
          <code class="sha">SHA-256: {localYtdlpCandidate.ytdlp_sha256}</code>
          <button
            class="btn btn-install"
            onclick={trustLocalYtdlp}
            disabled={isTrustingLocalYtdlp}
          >
            {isTrustingLocalYtdlp ? $t("download.tool.ytdlp.trustingLocal") : $t("download.tool.ytdlp.trustLocal")}
          </button>
        {/if}
        {#if localYtdlpMessage}
          <p class="install-message">{localYtdlpMessage}</p>
        {/if}
      </div>
    {/if}

    {#if !$toolStatus.ytdlp_available || !$toolStatus.ffmpeg_available}
      <div class="warning-box">
        {#if !$toolStatus.ytdlp_available}
          <div class="install-section">
            <p>{$t("download.tool.ytdlp.hint")}</p>
            <p class="hint-text">{$t("download.tool.ytdlp.localHint")}</p>
            <button
              class="btn btn-install"
              onclick={installYtdlp}
              disabled={$isInstalling || $isDownloading}
            >
              {$isInstalling ? $t("download.tool.ytdlp.installing") : $t("download.tool.ytdlp.install")}
            </button>
            {#if $installProgress}
              <div class="install-status">
                {#if $installProgress.status === "downloading"}
                  <div class="progress-bar-container" style="margin-top: 8px">
                    <div
                      class="progress-bar-fill"
                      style="width: {Math.min($installProgress.percent, 100)}%"
                    ></div>
                  </div>
                {/if}
                <span class="install-message">{$installProgress.message}</span>
              </div>
            {/if}
            <p class="hint-text">{$t("download.tool.ytdlp.manualHint")}</p>
          </div>
        {/if}

        {#if !$toolStatus.ffmpeg_available}
          <div class="install-section">
            <p>{$t("download.tool.ffmpeg.hint")}</p>
            <p class="hint-text">{$t("download.tool.ffmpeg.localHint")}</p>
            <div class="button-row">
              <button
                class="btn btn-secondary"
                onclick={detectLocalFfmpeg}
                disabled={isDetectingLocalFfmpeg || isTrustingLocalFfmpeg}
              >
                {isDetectingLocalFfmpeg ? $t("download.tool.ffmpeg.detectingLocal") : $t("download.tool.ffmpeg.detectLocal")}
              </button>
            </div>
            {#if localFfmpegCandidate}
              <div class="local-tool-card">
                <p class="local-tool-title">{$t("download.tool.ffmpeg.localFound")}</p>
                <code>{localFfmpegCandidate.ffmpeg_path}</code>
                <code>{localFfmpegCandidate.ffprobe_path}</code>
                <code class="sha">ffmpeg SHA-256: {localFfmpegCandidate.ffmpeg_sha256}</code>
                <code class="sha">ffprobe SHA-256: {localFfmpegCandidate.ffprobe_sha256}</code>
                <button
                  class="btn btn-install"
                  onclick={trustLocalFfmpeg}
                  disabled={isTrustingLocalFfmpeg}
                >
                  {isTrustingLocalFfmpeg ? $t("download.tool.ffmpeg.trustingLocal") : $t("download.tool.ffmpeg.trustLocal")}
                </button>
              </div>
            {/if}
            {#if localFfmpegMessage}
              <p class="install-message">{localFfmpegMessage}</p>
            {/if}
            <button
              class="btn btn-install"
              onclick={installFfmpeg}
              disabled={$isInstallingFfmpeg}
            >
              {$isInstallingFfmpeg ? $t("download.tool.ytdlp.installing") : $t("download.tool.ffmpeg.install")}
            </button>
            {#if $ffmpegInstallProgress}
              <div class="install-status">
                {#if $ffmpegInstallProgress.status === "downloading"}
                  <div class="progress-bar-container" style="margin-top: 8px">
                    <div
                      class="progress-bar-fill"
                      style="width: {Math.min($ffmpegInstallProgress.percent, 100)}%"
                    ></div>
                  </div>
                {/if}
                <span class="install-message">{$ffmpegInstallProgress.message}</span>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  {/if}

  <!-- URL 輸入 -->
  <div class="form-group">
    <label class="form-label" for="url-input">{$t("download.form.url.label")}</label>
    <div class="url-row">
      <input
        id="url-input"
        type="text"
        class="form-input"
        placeholder={$t("download.form.url.placeholder")}
        bind:value={url}
        oninput={onUrlChange}
        disabled={$isDownloading}
      />
      {#if $detectedUrlType}
        <span class="url-type-badge">{urlTypeLabel($detectedUrlType)}</span>
      {/if}
    </div>
  </div>

  <!-- 格式選項 -->
  <div class="form-row">
    <div class="form-group">
      <label class="form-label" for="format-select">{$t("download.form.format.label")}</label>
      <select id="format-select" class="form-select" bind:value={format} disabled={$isDownloading}>
        <option value="mp3">{$t("download.form.format.mp3")}</option>
        <option value="m4a">{$t("download.form.format.m4a")}</option>
        <option value="wav">{$t("download.form.format.wav")}</option>
        <option value="video">{$t("download.form.format.video")}</option>
        <option value="subtitle_only">{$t("download.form.format.subtitleOnly")}</option>
      </select>
    </div>

    {#if format === "video"}
      <div class="form-group">
        <label class="form-label" for="quality-select">{$t("download.form.quality.label")}</label>
        <select id="quality-select" class="form-select" bind:value={quality} disabled={$isDownloading}>
          <option value="best">{$t("download.form.quality.best")}</option>
          <option value="1080p">1080p</option>
          <option value="720p">720p</option>
          <option value="480p">480p</option>
          <option value="360p">360p</option>
        </select>
      </div>
    {/if}

    <div class="form-group">
      <label class="form-label" for="sub-select">
        {format === "subtitle_only" ? $t("download.form.subtitle.labelOnly") : $t("download.form.subtitle.label")}
      </label>
      <select id="sub-select" class="form-select" bind:value={subtitleLang} disabled={$isDownloading}>
        {#if format !== "subtitle_only"}
          <option value="none">{$t("download.form.subtitle.none")}</option>
        {/if}
        <option value="traditional_chinese">{$t("download.form.subtitle.traditional")}</option>
        <option value="simplified_chinese">{$t("download.form.subtitle.simplified")}</option>
        <option value="english">{$t("download.form.subtitle.english")}</option>
        <option value="japanese">{$t("download.form.subtitle.japanese")}</option>
        <option value="all">{$t("download.form.subtitle.all")}</option>
      </select>
    </div>
  </div>

  <!-- 輸出目錄 -->
  <div class="form-group">
    <label class="form-label" for="output-dir-input">{$t("download.form.outputDir.label")}</label>
    <div class="dir-row">
      <input
        id="output-dir-input"
        type="text"
        class="form-input dir-input"
        bind:value={outputDir}
        placeholder={$t("download.form.outputDir.placeholder")}
        disabled={$isDownloading}
        readonly
      />
      <button class="btn btn-secondary" onclick={selectOutputDir} disabled={$isDownloading}>
        {$t("download.form.outputDir.browse")}
      </button>
    </div>
  </div>

  <!-- 動作按鈕 -->
  <div class="actions">
    {#if $isDownloading}
      <button class="btn btn-danger" onclick={cancelDownload}>
        {$t("download.action.cancel")}
      </button>
    {:else}
      <button
        class="btn btn-primary"
        onclick={startDownload}
        disabled={!url.trim() || !outputDir.trim() || !$toolStatus?.ytdlp_available}
      >
        {format === "subtitle_only" ? $t("download.action.downloadSubtitle") : $t("download.action.download")}
      </button>
    {/if}
  </div>

  <!-- 進度條 -->
  {#if $downloadProgress && $isDownloading}
    <div class="progress-section">
      <div class="progress-bar-container">
        <div
          class="progress-bar-fill"
          style="width: {Math.min($downloadProgress.percent, 100)}%"
        ></div>
      </div>
      <div class="progress-info">
        <span class="progress-pct">{$downloadProgress.percent.toFixed(1)}%</span>
        {#if $downloadProgress.speed}
          <span class="progress-speed">{$downloadProgress.speed}</span>
        {/if}
        {#if $downloadProgress.eta}
          <span class="progress-eta">{$t("download.progress.eta", { eta: $downloadProgress.eta })}</span>
        {/if}
        {#if $downloadProgress.status === "postprocessing"}
          <span class="progress-postproc">{$t("download.progress.postprocessing")}</span>
        {/if}
      </div>
      {#if $downloadProgress.filename}
        <div class="progress-filename" title={$downloadProgress.filename}>
          {$downloadProgress.filename.split("/").pop() ?? $downloadProgress.filename}
        </div>
      {/if}
    </div>
  {/if}

  <!-- 結果訊息 -->
  {#if $lastResult && !$isDownloading}
    <div class="result-box" class:success={$lastResult.success} class:fail={!$lastResult.success}>
      {$lastResult.message}
    </div>

    <!-- 字幕檔案發現 + 載入為歌詞 -->
    {#if $lastResult.success && $lastResult.subtitle_paths.length > 0}
      <div class="subtitle-section">
        <p class="subtitle-title">{$t("download.subtitle.foundCount", { count: $lastResult.subtitle_paths.length })}</p>
        <div class="subtitle-list">
          {#each $lastResult.subtitle_paths as subPath}
            <div class="subtitle-item">
              <span class="subtitle-name" title={subPath}>{basename(subPath)}</span>
              <button
                class="btn btn-small btn-secondary"
                onclick={() => loadSubtitleAsLyrics(subPath)}
                disabled={subtitleLoading}
              >
                {subtitleLoading ? $t("download.subtitle.loadingAsLyrics") : $t("download.subtitle.loadAsLyrics")}
              </button>
            </div>
          {/each}
        </div>
        {#if subtitleLoadMsg}
          <p class="subtitle-msg">{subtitleLoadMsg}</p>
        {/if}
      </div>
    {/if}
  {/if}

  {#if $downloadStatus === "cancelled"}
    <div class="result-box fail">{$t("download.result.cancelled")}</div>
  {/if}
</div>

<style>
  .download-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  /* 工具狀態 */
  .tool-status {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 16px;
    font-size: 13px;
  }

  .tool-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }

  .status-item {
    display: flex;
    align-items: center;
    gap: 6px;
    color: #7a7268;
  }

  .trust-badge {
    border: 1px solid #e5cf96;
    border-radius: 999px;
    color: #755700;
    background: #fff8e1;
    font-size: 11px;
    font-weight: 600;
    padding: 2px 6px;
  }

  .btn-tool-update {
    padding: 6px 12px;
    font-size: 12px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .status-item.ok .status-dot {
    background: #4caf50;
  }

  .status-item.missing .status-dot {
    background: #e57373;
  }

  .warning-box {
    background: #fff3e0;
    border: 1px solid #ffe0b2;
    border-radius: 8px;
    padding: 12px 16px;
    font-size: 13px;
    color: #795548;
    line-height: 1.5;
  }

  .warning-box p {
    margin: 0 0 8px;
  }

  .warning-box p:last-child {
    margin-bottom: 0;
  }

  .warning-box code {
    background: #efebe9;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 12px;
  }

  .btn-install {
    background: #755700;
    color: white;
    padding: 8px 16px;
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-install:hover:not(:disabled) {
    background: #8a6800;
  }

  .btn-install:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .install-status {
    margin-top: 6px;
  }

  .update-status {
    background: #fffaf0;
    border: 1px solid #f1ddb0;
    border-radius: 8px;
    padding: 8px 12px;
  }

  .install-message {
    font-size: 12px;
    color: #795548;
    display: block;
    margin-top: 4px;
  }

  .hint-text {
    font-size: 12px;
    color: #a0968c;
    margin-top: 8px;
  }

  .install-section {
    padding-bottom: 12px;
    margin-bottom: 12px;
    border-bottom: 1px solid #ffe0b2;
  }

  .install-section:last-child {
    border-bottom: none;
    padding-bottom: 0;
    margin-bottom: 0;
  }

  .button-row {
    display: flex;
    gap: 8px;
    margin: 8px 0;
  }

  .local-tool-card {
    display: grid;
    gap: 6px;
    background: #fffaf0;
    border: 1px solid #f1ddb0;
    border-radius: 8px;
    padding: 10px 12px;
    margin: 8px 0;
  }

  .local-tool-title {
    font-weight: 600;
    color: #5a5248;
  }

  .local-tool-card code {
    display: block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .local-tool-card code.sha {
    font-size: 11px;
    color: var(--color-text-muted);
  }

  /* 表單 */
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-label {
    font-size: 13px;
    font-weight: 500;
    color: #5a5248;
  }

  .form-input,
  .form-select {
    padding: 8px 12px;
    border: 1px solid #e0dbd4;
    border-radius: 8px;
    font-size: 14px;
    background: white;
    color: #3d3630;
    transition: border-color 0.2s;
  }

  .form-input:focus,
  .form-select:focus {
    outline: none;
    border-color: #fdc003;
  }

  .form-input:disabled,
  .form-select:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .url-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .url-row .form-input {
    flex: 1;
  }

  .url-type-badge {
    font-size: 12px;
    background: #f0ece4;
    color: #755700;
    padding: 4px 10px;
    border-radius: 12px;
    white-space: nowrap;
    font-weight: 500;
  }

  .form-row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .form-row .form-group {
    flex: 1;
    min-width: 120px;
  }

  .dir-row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .dir-input {
    flex: 1;
    cursor: pointer;
  }

  /* 按鈕 */
  .actions {
    display: flex;
    gap: 8px;
  }

  .btn {
    padding: 10px 20px;
    border: none;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    background: #755700;
    color: white;
  }

  .btn-primary:hover:not(:disabled) {
    background: #8a6800;
  }

  .btn-secondary {
    background: #f0ece4;
    color: #5a5248;
  }

  .btn-secondary:hover:not(:disabled) {
    background: #e8e2d8;
  }

  .btn-danger {
    background: #e57373;
    color: white;
  }

  .btn-danger:hover:not(:disabled) {
    background: #ef5350;
  }

  /* 進度條 */
  .progress-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .progress-bar-container {
    height: 8px;
    background: #e8e2d8;
    border-radius: 4px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #fdc003, #f5a623);
    border-radius: 4px;
    transition: width 0.3s ease;
  }

  .progress-info {
    display: flex;
    gap: 12px;
    font-size: 13px;
    color: #7a7268;
  }

  .progress-pct {
    font-weight: 600;
    color: #755700;
  }

  .progress-postproc {
    font-style: italic;
  }

  .progress-filename {
    font-size: 12px;
    color: #9a9088;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* 結果 */
  .result-box {
    padding: 12px 16px;
    border-radius: 8px;
    font-size: 14px;
    line-height: 1.5;
  }

  .result-box.success {
    background: #e8f5e9;
    border: 1px solid #c8e6c9;
    color: #2e7d32;
  }

  .result-box.fail {
    background: #ffebee;
    border: 1px solid #ffcdd2;
    color: #c62828;
  }

  /* 字幕載入區 */
  .subtitle-section {
    background: #f5f3ef;
    border-radius: 8px;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .subtitle-title {
    font-size: 13px;
    font-weight: 600;
    color: #5a5248;
    margin: 0;
  }

  .subtitle-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .subtitle-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    background: white;
    border-radius: 6px;
    border: 1px solid #e8e2d8;
  }

  .subtitle-name {
    font-size: 13px;
    color: #3d3630;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }

  .btn-small {
    padding: 4px 12px;
    font-size: 12px;
    flex-shrink: 0;
  }

  .subtitle-msg {
    font-size: 12px;
    color: #755700;
    margin: 0;
    font-weight: 500;
  }
</style>
