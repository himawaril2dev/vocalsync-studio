<script lang="ts">
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { get } from "svelte/store";
  import { t, tSync } from "../i18n";
  import LyricsPrepTools from "./LyricsPrepTools.svelte";
  import UvrGuideModal from "./UvrGuideModal.svelte";
  import { lyricsFileName, lyricsLines, type LyricLine } from "../stores/lyrics";
  import { basename, loadedMedia } from "../stores/media";
  import {
    alignmentConfidence,
    alignmentFineTuneMs,
    alignmentResult,
    currentMelody,
    detectedMelodySourceKind,
    finalOffsetSecs,
    guideVocalPath,
    melodySourcePath,
    melodyStatus,
    resetMelodyState,
    type AlignmentResult,
    type MelodyStatusMessage,
    type MelodyTrack,
    type TranslatableDescriptor,
  } from "../stores/melody";
  import {
    backingPitchTrack,
    clearLiveVocalSamples,
    resetBackingState,
  } from "../stores/pitch";
  import { guideVocalEnabled } from "../stores/settings";
  import {
    activeSongProfileId,
    activeSongProfileTitle,
    startNewSong,
  } from "../stores/songProfiles";
  import { clearLoop, hasRecording } from "../stores/transport";

  interface LoadResult {
    duration: number;
    sample_rate: number;
    is_video: boolean;
    video_path: string | null;
    melody_source: string | null;
  }

  interface SubtitleStream {
    index: number;
    language: string;
    title: string;
    codec: string;
  }

  type LyricsStatusMessage =
    | null
    | { key: string; vars?: Record<string, string | number> };

  const UVR_GUIDE_COLLAPSED_KEY = "vocalsync.uvrGuideCollapsed.v1";

  function readUvrGuideCollapsed(): boolean {
    if (typeof localStorage === "undefined") return false;
    return localStorage.getItem(UVR_GUIDE_COLLAPSED_KEY) === "1";
  }

  let showUvrGuide = $state(false);
  let uvrGuideCollapsed = $state(readUvrGuideCollapsed());

  function toggleUvrGuideCollapsed(): void {
    uvrGuideCollapsed = !uvrGuideCollapsed;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(UVR_GUIDE_COLLAPSED_KEY, uvrGuideCollapsed ? "1" : "0");
    }
  }
  let pendingStatusText = $state<string | null>(null);
  let lyricsStatus = $state<LyricsStatusMessage>(null);
  let embeddedSubtitles = $state<SubtitleStream[]>([]);
  let subtitleExtracting = $state(false);
  let probedSubtitlePath = $state("");

  let backingLoaded = $derived($loadedMedia !== null);
  let currentBackingPath = $derived($loadedMedia?.file_path ?? null);

  let statusText = $derived.by(() => {
    const translate = $t;
    if (pendingStatusText !== null) return pendingStatusText;
    const media = $loadedMedia;
    if (!media) return translate("setup.backing.hint.empty");
    const min = Math.floor(media.duration / 60);
    const sec = Math.floor(media.duration % 60).toString().padStart(2, "0");
    const kind = media.is_video
      ? translate("setup.backing.kind.video")
      : translate("setup.backing.kind.audio");
    return translate("setup.backing.hint.loaded", {
      kind,
      name: media.file_name,
      min,
      sec,
    });
  });

  let lyricsStatusText = $derived.by(() => {
    const translate = $t;
    const message = lyricsStatus;
    if (message) return translate(message.key, message.vars);
    if ($lyricsLines.length > 0) {
      return translate("songLibrary.material.lyrics.current", {
        n: $lyricsLines.length,
        name: $lyricsFileName || translate("songLibrary.material.lyrics.unnamed"),
      });
    }
    return translate("setup.lyrics.status.empty");
  });

  let melodyStatusText = $derived.by(() => {
    const translate = $t;
    const message = $melodyStatus;
    if (!message) return translate("setup.melody.status.empty");
    let mergedVars = message.vars;
    if (message.nestedVars) {
      const translated: Record<string, string | number> = { ...(message.vars ?? {}) };
      for (const [field, desc] of Object.entries(message.nestedVars)) {
        translated[field] = translate(desc.key, desc.vars);
      }
      mergedVars = translated;
    }
    const base = translate(message.key, mergedVars);
    if (message.appendKey) {
      return translate(message.appendKey, { ...(message.appendVars ?? {}), status: base });
    }
    return base;
  });

  $effect(() => {
    const media = $loadedMedia;
    if (!media?.file_path || !media.is_video) {
      embeddedSubtitles = [];
      probedSubtitlePath = "";
      return;
    }
    if (media.file_path === probedSubtitlePath) return;
    probedSubtitlePath = media.file_path;
    void probeEmbeddedSubtitles(media.file_path);
  });

  async function probeEmbeddedSubtitles(videoPath: string): Promise<void> {
    try {
      const subs = await invoke<SubtitleStream[]>("probe_embedded_subtitles", {
        videoPath,
      });
      embeddedSubtitles = subs;
      if (subs.length > 0 && $lyricsLines.length === 0) {
        lyricsStatus = {
          key: "setup.lyrics.status.subDetected",
          vars: { n: subs.length },
        };
      }
    } catch (error) {
      console.warn("[song-library] embedded subtitle probe failed:", error);
    }
  }

  function currentGuideOffsetSecs(): number {
    return finalOffsetSecs(get(alignmentResult), get(alignmentFineTuneMs));
  }

  async function clearGuideVocalTrack(): Promise<void> {
    guideVocalPath.set(null);
    guideVocalEnabled.set(false);
    await invoke("clear_guide_vocal").catch((error) =>
      console.warn("[guide] clear failed:", error),
    );
  }

  async function clearVocalsTrack(): Promise<void> {
    await clearGuideVocalTrack();
    melodyStatus.set({ key: "setup.guide.status.cleared" });
  }

  function clearPitchCurve(): void {
    currentMelody.set(null);
    detectedMelodySourceKind.set(null);
    melodySourcePath.set(null);
    alignmentResult.set(null);
    alignmentFineTuneMs.set(0);
    backingPitchTrack.set(null);
    melodyStatus.set({ key: "setup.melody.status.cleared" });
  }

  async function clearBackingTrack(): Promise<void> {
    try {
      await invoke("clear_backing");
    } catch (error) {
      console.warn("[backing] clear failed:", error);
    }
    loadedMedia.set(null);
    pendingStatusText = null;
    resetBackingState();
    resetMelodyState();
    await clearGuideVocalTrack();
    clearLoop();
    hasRecording.set(false);
    clearLiveVocalSamples();
    embeddedSubtitles = [];
    probedSubtitlePath = "";
    melodyStatus.set(null);
  }

  async function loadGuideVocalTrack(path: string): Promise<void> {
    await invoke("load_guide_vocal", {
      path,
      offsetSecs: currentGuideOffsetSecs(),
    });
    guideVocalPath.set(path);
    guideVocalEnabled.set(true);
  }

  async function syncGuideVocalTiming(): Promise<void> {
    if (!get(guideVocalPath)) return;
    await invoke("set_guide_vocal_offset", {
      offsetSecs: currentGuideOffsetSecs(),
    }).catch((error) => console.warn("[guide] offset sync failed:", error));
  }

  async function loadBackingFromPath(path: string): Promise<void> {
    pendingStatusText = tSync("setup.backing.hint.loading");
    loadedMedia.set(null);
    resetBackingState();
    resetMelodyState();
    await clearGuideVocalTrack();
    clearLoop();
    hasRecording.set(false);
    clearLiveVocalSamples();
    embeddedSubtitles = [];
    probedSubtitlePath = "";

    const result: LoadResult = await invoke("load_backing", { path });
    loadedMedia.set({
      file_path: path,
      file_name: basename(path),
      duration: result.duration,
      sample_rate: result.sample_rate,
      is_video: result.is_video,
      video_path: result.video_path,
      video_url: result.video_path ? convertFileSrc(result.video_path) : null,
    });
    pendingStatusText = null;
    detectedMelodySourceKind.set(result.melody_source);

    if (result.is_video) {
      probedSubtitlePath = path;
      await probeEmbeddedSubtitles(path);
    }

    await autoLoadMelodyForPath(path);
  }

  async function loadFile(): Promise<void> {
    const path = await open({
      title: tSync("setup.backing.dialog.title"),
      filters: [
        {
          name: tSync("setup.backing.dialog.filter"),
          extensions: ["wav", "mp3", "mp4", "m4a", "aac", "flac", "ogg", "mkv", "webm"],
        },
      ],
    });
    if (!path) return;

    // 有啟用中歌曲時匯入不同的伴奏檔：先確認是否要另存為新歌曲，
    // 避免自動儲存把新素材寫進舊歌的存檔（造成歌單失序、時間軸遺失）
    if (get(activeSongProfileId) && path !== currentBackingPath) {
      const replaceCurrent = await ask(
        tSync("setup.backing.guard.message", {
          title: get(activeSongProfileTitle),
        }),
        {
          title: tSync("setup.backing.guard.title"),
          kind: "warning",
          okLabel: tSync("setup.backing.guard.replace"),
          cancelLabel: tSync("setup.backing.guard.startNew"),
        },
      );
      if (!replaceCurrent) {
        await startNewSong();
      }
    }

    try {
      await loadBackingFromPath(path);
    } catch (error) {
      pendingStatusText = tSync("setup.backing.hint.loadFailed", {
        error: String(error),
      });
    }
  }

  async function autoLoadMelodyForPath(backingPath: string): Promise<void> {
    try {
      const track = await invoke<MelodyTrack | null>("auto_load_melody_for_backing", {
        backingPath,
      });
      if (track) {
        await commitMelodyTrack(track, null);
        const sourceDescriptor = describeMelodySource(track);
        melodyStatus.set({
          key: "setup.melody.status.autoLoaded",
          vars: { n: track.notes.length },
          nestedVars: { source: sourceDescriptor },
        });
      } else {
        melodyStatus.set({ key: "setup.melody.status.noAutoDetect" });
      }
    } catch (error) {
      melodyStatus.set({
        key: "setup.melody.status.loadFailed",
        vars: { error: String(error) },
      });
    }
  }

  async function loadMelodyFile(): Promise<void> {
    const path = await open({
      title: tSync("setup.melody.dialog.midi.title"),
      filters: [
        { name: tSync("setup.melody.dialog.midi.filter"), extensions: ["mid", "midi"] },
      ],
    });
    if (!path) return;

    melodyStatus.set({ key: "setup.melody.status.parsing" });
    try {
      const track = await invoke<MelodyTrack>("load_melody_from_path", { path });
      await commitMelodyTrack(track, path);
      await clearGuideVocalTrack();
      const sourceDescriptor = describeMelodySource(track);
      melodyStatus.set({
        key: "setup.melody.status.loaded",
        vars: { n: track.notes.length },
        nestedVars: { source: sourceDescriptor },
      });
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
    } catch (error) {
      melodyStatus.set({
        key: "setup.melody.status.loadFailed",
        vars: { error: String(error) },
      });
    }
  }

  async function loadVocalsTrack(): Promise<void> {
    const path = await open({
      title: tSync("setup.melody.dialog.vocals.title"),
      filters: [
        {
          name: tSync("setup.melody.dialog.vocals.filter"),
          extensions: ["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus"],
        },
      ],
    });
    if (!path) return;

    melodyStatus.set({ key: "setup.melody.status.parsingVocals" });
    try {
      const track = await invoke<MelodyTrack>("load_vocals_and_extract_melody", {
        vocalsPath: path,
      });
      await commitMelodyTrack(track, path);
      const sourceDescriptor = describeMelodySource(track);
      melodyStatus.set({
        key: "setup.melody.status.vocalsLoaded",
        vars: { n: track.raw_pitch_track?.length ?? track.notes.length },
        nestedVars: { source: sourceDescriptor },
      });
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
      try {
        await loadGuideVocalTrack(path);
        await syncGuideVocalTiming();
      } catch (guideError) {
        guideVocalPath.set(null);
        guideVocalEnabled.set(false);
        melodyStatus.update((status) =>
          status
            ? {
                ...status,
                appendKey: "setup.guide.status.loadFailedAppend",
                appendVars: { error: String(guideError) },
              }
            : status,
        );
      }
    } catch (error) {
      melodyStatus.set({
        key: "setup.melody.status.vocalsFailed",
        vars: { error: String(error) },
      });
    }
  }

  async function commitMelodyTrack(
    track: MelodyTrack,
    sourcePath: string | null,
  ): Promise<void> {
    currentMelody.set(track);
    melodySourcePath.set(sourcePath);
    alignmentResult.set(null);
    alignmentFineTuneMs.set(0);
  }

  async function runAutoAlignment(
    referencePath: string,
    targetPath: string,
  ): Promise<void> {
    if (referencePath === targetPath) {
      alignmentResult.set(null);
      return;
    }
    try {
      const result = await invoke<AlignmentResult>("align_audio_files", {
        referencePath,
        targetPath,
      });
      alignmentResult.set(result);
    } catch (error) {
      console.error("[song-library] audio alignment failed:", error);
      alignmentResult.set(null);
      melodyStatus.update((status: MelodyStatusMessage) => {
        if (!status) {
          return {
            key: "setup.melody.status.empty",
            appendKey: "setup.melody.status.alignFailedAppend",
            appendVars: { error: String(error) },
          };
        }
        return {
          key: status.key,
          vars: status.vars,
          appendKey: "setup.melody.status.alignFailedAppend",
          appendVars: { error: String(error) },
        };
      });
    }
  }

  function describeMelodySource(track: MelodyTrack): TranslatableDescriptor {
    const source = track.source;
    if (source.type === "midi") {
      return {
        key: "setup.melody.source.midi",
        vars: { n: source.track_index + 1 },
      };
    }
    if (source.type === "imported_vocals") {
      const voiced = (source.voiced_ratio * 100).toFixed(0);
      return {
        key: "setup.melody.source.importedVocals",
        vars: { ratio: voiced },
      };
    }
    return {
      key: "setup.melody.source.separated",
      vars: { model: source.model },
    };
  }

  function describeAlignmentOffset(result: AlignmentResult | null): string {
    if (!result) return "";
    const seconds = result.offset_secs;
    const sign = seconds >= 0 ? "+" : "";
    return `${sign}${seconds.toFixed(3)} ${tSync("setup.alignment.offset.seconds")}`;
  }

  /** 現有歌詞（可能含調好的時間軸）即將被覆蓋時，先請使用者確認 */
  async function confirmOverwriteLyrics(): Promise<boolean> {
    const lines = get(lyricsLines);
    if (lines.length === 0) return true;
    const timed = lines.filter((line) => line.end_ms > line.start_ms).length;
    return ask(
      tSync("setup.lyrics.guard.reimportMessage", { n: lines.length, timed }),
      { title: tSync("setup.lyrics.guard.reimportTitle"), kind: "warning" },
    );
  }

  async function extractAndLoadSubtitle(sub: SubtitleStream): Promise<void> {
    if (!currentBackingPath || subtitleExtracting) return;
    if (!(await confirmOverwriteLyrics())) return;
    subtitleExtracting = true;
    lyricsStatus = {
      key: "setup.lyrics.status.subExtracting",
      vars: { index: sub.index, lang: sub.language || sub.codec },
    };
    try {
      const srtPath = await invoke<string>("extract_embedded_subtitle", {
        videoPath: currentBackingPath,
        streamIndex: sub.index,
        outputDir: null,
      });
      const lines: LyricLine[] = await invoke("load_lyrics", { path: srtPath });
      lyricsLines.set(lines);
      const fileName = srtPath.replace(/\\/g, "/").split("/").pop() ?? "";
      lyricsFileName.set(fileName);
      lyricsStatus = {
        key: "setup.lyrics.status.subExtracted",
        vars: { n: lines.length, name: fileName },
      };
    } catch (error) {
      lyricsStatus = {
        key: "setup.lyrics.status.subFailed",
        vars: { error: String(error) },
      };
    } finally {
      subtitleExtracting = false;
    }
  }

  function subtitleLabel(sub: SubtitleStream): string {
    const parts: string[] = [];
    if (sub.title) parts.push(sub.title);
    else if (sub.language) parts.push(sub.language);
    parts.push(sub.codec);
    return parts.join(" / ");
  }

  async function loadLyrics(): Promise<void> {
    const path = await open({
      title: tSync("setup.lyrics.dialog.title"),
      filters: [
        { name: tSync("setup.lyrics.dialog.filter"), extensions: ["lrc", "srt", "vtt", "txt"] },
      ],
    });
    if (!path) return;
    if (!(await confirmOverwriteLyrics())) return;

    lyricsStatus = { key: "setup.lyrics.status.parsing" };
    try {
      const lines: LyricLine[] = await invoke("load_lyrics", { path });
      lyricsLines.set(lines);
      const fileName = path.split(/[\\/]/).pop() || "";
      lyricsFileName.set(fileName);
      lyricsStatus = {
        key: "setup.lyrics.status.loaded",
        vars: { n: lines.length, name: fileName },
      };
    } catch (error) {
      lyricsStatus = {
        key: "setup.lyrics.status.loadFailed",
        vars: { error: String(error) },
      };
    }
  }

  async function clearLyrics(): Promise<void> {
    const lines = get(lyricsLines);
    if (lines.length > 0) {
      const confirmed = await ask(
        tSync("setup.lyrics.guard.clearMessage", { n: lines.length }),
        { title: tSync("setup.lyrics.guard.clearTitle"), kind: "warning" },
      );
      if (!confirmed) return;
    }
    lyricsLines.set([]);
    lyricsFileName.set("");
    lyricsStatus = null;
  }
</script>

<div class="material-stack">
  <section class="material-card">
    <h2>{$t("setup.section.song")}</h2>
    <div class="section-body song-source-section">
      <p class="hint">{$t("songLibrary.material.song.hint")}</p>

      <div class="uvr-guide-block">
        <button
          class="uvr-guide-toggle"
          type="button"
          onclick={toggleUvrGuideCollapsed}
          aria-expanded={!uvrGuideCollapsed}
        >
          <span class="uvr-guide-toggle-icon">{uvrGuideCollapsed ? "▸" : "▾"}</span>
          <span>{$t("setup.uvrGuide.toggle")}</span>
        </button>

        {#if !uvrGuideCollapsed}
          <button class="uvr-info-card" type="button" onclick={() => (showUvrGuide = true)}>
            <div>
              <strong>{$t("uvrGuide.entry.title")}</strong>
              <p>{$t("uvrGuide.entry.body")}</p>
            </div>
            <span>{$t("uvrGuide.entry.action")}</span>
          </button>

          <div class="uvr-flow-grid">
            <article class="uvr-step-card">
              <span>1</span>
              <div>
                <strong>{$t("setup.separation.external.step1.title")}</strong>
                <p>{$t("setup.separation.external.step1.body")}</p>
              </div>
            </article>
            <article class="uvr-step-card">
              <span>2</span>
              <div>
                <strong>{$t("setup.separation.external.step2.title")}</strong>
                <p>{$t("setup.separation.external.step2.body")}</p>
              </div>
            </article>
            <article class="uvr-step-card">
              <span>3</span>
              <div>
                <strong>{$t("setup.separation.external.step3.title")}</strong>
                <p>{$t("setup.separation.external.step3.body")}</p>
              </div>
            </article>
          </div>
        {/if}
      </div>

      <div class="source-grid">
        <section class="source-panel">
          <div class="source-panel-header">
            <span>1</span>
            <div>
              <strong>{$t("setup.song.backing.title")}</strong>
              <p>{statusText}</p>
            </div>
          </div>
          <p class="sub-hint source-hint">{$t("setup.backing.subHint")}</p>
          <div class="actions">
            <button class="btn primary" onclick={loadFile}>
              {$t("setup.backing.action.import")}
            </button>
            {#if backingLoaded}
              <button class="btn secondary" onclick={clearBackingTrack}>
                {$t("setup.backing.action.clear")}
              </button>
            {/if}
          </div>
        </section>

        <section class="source-panel">
          <div class="source-panel-header">
            <span>2</span>
            <div>
              <strong>{$t("setup.song.melody.title")}</strong>
              <p>{melodyStatusText}</p>
            </div>
          </div>

          {#if $currentMelody === null}
            <p class="sub-hint source-hint">
              {$t("setup.melody.hint.empty.prefix")}<strong>{$t("setup.melody.hint.empty.vocals")}</strong>{$t("setup.melody.hint.empty.or")}<strong>{$t("setup.melody.hint.empty.midi")}</strong>{$t("setup.melody.hint.empty.suffix")}
            </p>
          {/if}

          <div class="actions">
            <button
              class="btn primary"
              onclick={loadVocalsTrack}
              disabled={!backingLoaded}
              title={$t("setup.melody.action.importVocals.title")}
            >
              {$t("setup.melody.action.importVocals")}
            </button>
            <button
              class="btn secondary"
              onclick={loadMelodyFile}
              disabled={!backingLoaded}
            >
              {$t("setup.melody.action.loadMidi")}
            </button>
            {#if currentBackingPath && $currentMelody === null}
              <button
                class="btn ghost"
                onclick={() => currentBackingPath && autoLoadMelodyForPath(currentBackingPath)}
              >
                {$t("setup.melody.action.rescan")}
              </button>
            {/if}
            {#if $guideVocalPath}
              <button class="btn secondary" onclick={clearVocalsTrack}>
                {$t("setup.melody.action.clearVocals")}
              </button>
            {/if}
            {#if $currentMelody}
              <button class="btn secondary" onclick={clearPitchCurve}>
                {$t("setup.melody.action.clearPitch")}
              </button>
            {/if}
          </div>

          {#if $guideVocalPath}
            <label class="guide-toggle" title={$guideVocalPath}>
              <input type="checkbox" bind:checked={$guideVocalEnabled} />
              <span>{$t("setup.guide.toggle", { name: basename($guideVocalPath) })}</span>
            </label>
            <p class="sub-hint guide-hint">{$t("setup.guide.hint")}</p>
          {/if}

          {#if $currentMelody}
            <div class="alignment-box">
              <div class="alignment-header">
                <span class="alignment-title">{$t("setup.alignment.title")}</span>
                {#if $melodySourcePath === null}
                  <span class="badge badge-muted">{$t("setup.alignment.badge.noNeed")}</span>
                {:else if $alignmentResult}
                  {#if alignmentConfidence($alignmentResult) === "high"}
                    <span class="badge badge-high">{$t("setup.alignment.badge.high")}</span>
                  {:else if alignmentConfidence($alignmentResult) === "medium"}
                    <span class="badge badge-medium">{$t("setup.alignment.badge.medium")}</span>
                  {:else}
                    <span class="badge badge-low">{$t("setup.alignment.badge.low")}</span>
                  {/if}
                {:else}
                  <span class="badge badge-muted">{$t("setup.alignment.badge.notAligned")}</span>
                {/if}
              </div>

              {#if $melodySourcePath === null}
                <p class="alignment-hint">{$t("setup.alignment.hint.sameSource")}</p>
              {:else if $alignmentResult}
                <p class="alignment-hint">
                  {$t("setup.alignment.hint.offset", {
                    offset: describeAlignmentOffset($alignmentResult),
                    ratio: $alignmentResult.peak_to_mean_ratio.toFixed(1),
                  })}
                </p>
              {:else}
                <p class="alignment-hint">{$t("setup.alignment.hint.pending")}</p>
              {/if}

              <div class="fine-tune-row">
                <label for="song_fine_tune">{$t("setup.alignment.fineTune.label")}</label>
                <input
                  id="song_fine_tune"
                  type="range"
                  min="-500"
                  max="500"
                  step="1"
                  bind:value={$alignmentFineTuneMs}
                />
                <span class="fine-tune-value">
                  {$alignmentFineTuneMs >= 0 ? "+" : ""}{$alignmentFineTuneMs} ms
                </span>
                {#if $alignmentFineTuneMs !== 0}
                  <button
                    class="btn ghost tiny"
                    onclick={() => alignmentFineTuneMs.set(0)}
                    title={$t("setup.alignment.fineTune.reset.title")}
                  >
                    {$t("setup.alignment.fineTune.reset.text")}
                  </button>
                {/if}
              </div>
            </div>
          {/if}
        </section>
      </div>

      <p class="sub-hint">{$t("setup.separation.external.orderHint")}</p>
    </div>
  </section>

  <section class="material-card">
    <h2>{$t("setup.section.lyrics")}</h2>
    <div class="section-body">
      <p class="hint">{lyricsStatusText}</p>
      <p class="sub-hint lyrics-feature-hint">{$t("songLibrary.material.lyrics.hint")}</p>

      {#if embeddedSubtitles.length > 0}
        <div class="embedded-subs">
          <p class="sub-hint">{$t("setup.lyrics.subTitle")}</p>
          <div class="sub-list">
            {#each embeddedSubtitles as sub}
              <button
                class="btn sub-btn"
                onclick={() => extractAndLoadSubtitle(sub)}
                disabled={subtitleExtracting}
              >
                #{sub.index} {subtitleLabel(sub)}
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <div class="actions">
        <button class="btn primary" onclick={loadLyrics}>
          {$t("setup.lyrics.action.load")}
        </button>
        {#if $lyricsLines.length > 0}
          <button class="btn secondary" onclick={clearLyrics}>
            {$t("setup.lyrics.action.clear")}
          </button>
        {/if}
      </div>
      <LyricsPrepTools />
    </div>
  </section>
</div>

{#if showUvrGuide}
  <UvrGuideModal onClose={() => (showUvrGuide = false)} />
{/if}

<style>
  .material-stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .material-card {
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: var(--space-lg);
  }

  h2,
  p {
    margin: 0;
  }

  .material-card h2 {
    margin-bottom: var(--space-md);
    color: var(--color-text);
    font-size: 1.05rem;
  }

  .section-body,
  .song-source-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .hint {
    margin: 0;
    font-size: 14px;
    color: #7a7268;
  }

  .sub-hint {
    margin: 0;
    color: #a0958a;
    font-size: 12px;
    line-height: 1.5;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .btn {
    padding: 8px 20px;
    border: none;
    border-radius: 8px;
    font-size: 14px;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn.primary {
    background: #755700;
    color: #fff;
  }

  .btn.primary:hover {
    background: #5c4400;
  }

  .btn.secondary {
    background: #f0ece4;
    color: #7a7268;
  }

  .btn.ghost {
    background: transparent;
    color: #7a7268;
    border: 1px solid #e8e2d8;
  }

  .btn.ghost:hover:not(:disabled) {
    background: #faf8f4;
    color: #3d3630;
  }

  .btn.tiny {
    padding: 4px 12px;
    font-size: 12px;
  }

  .btn:disabled,
  .sub-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .uvr-guide-block {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .uvr-guide-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    align-self: flex-start;
    padding: 6px 12px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-bg);
    color: var(--color-text-secondary);
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .uvr-guide-toggle:hover {
    background: var(--color-bg-hover);
    color: var(--color-text);
  }

  .uvr-guide-toggle-icon {
    color: var(--color-brand);
    font-size: 0.8rem;
  }

  .uvr-info-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    width: 100%;
    padding: 16px 18px;
    border: 1px solid rgba(159, 122, 0, 0.26);
    border-radius: 16px;
    background:
      radial-gradient(circle at top left, rgba(253, 192, 3, 0.16), transparent 34%),
      #fffaf1;
    color: #3d3630;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color 0.16s ease,
      transform 0.16s ease,
      box-shadow 0.16s ease;
  }

  .uvr-info-card:hover {
    border-color: rgba(117, 87, 0, 0.46);
    box-shadow: 0 12px 28px rgba(117, 87, 0, 0.12);
    transform: translateY(-1px);
  }

  .uvr-info-card strong {
    display: block;
    margin-bottom: 5px;
    color: #3d3630;
    font-size: 15px;
  }

  .uvr-info-card p {
    color: #6f655b;
    font-size: 13px;
    line-height: 1.65;
  }

  .uvr-info-card > span {
    flex: 0 0 auto;
    padding: 9px 14px;
    border-radius: 999px;
    background: #755700;
    color: #fff;
    font-size: 12px;
    font-weight: 700;
    white-space: nowrap;
  }

  .uvr-flow-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
  }

  .uvr-step-card,
  .source-panel {
    border: 1px solid #e8e2d8;
    border-radius: 14px;
    background: #fdfaf5;
  }

  .uvr-step-card {
    display: flex;
    gap: 12px;
    padding: 14px 16px;
  }

  .uvr-step-card span,
  .source-panel-header > span {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #755700;
    color: #fff;
    font-size: 13px;
    font-weight: 800;
  }

  .uvr-step-card strong,
  .source-panel-header strong {
    display: block;
    margin-bottom: 5px;
    color: #3d3630;
    font-size: 14px;
  }

  .uvr-step-card p,
  .source-panel-header p {
    color: #5c5248;
    font-size: 13px;
    line-height: 1.6;
  }

  .source-grid {
    display: grid;
    grid-template-columns: minmax(280px, 0.9fr) minmax(340px, 1.1fr);
    gap: 12px;
  }

  .source-panel {
    padding: 16px;
    background: #fff;
  }

  .source-panel-header {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    margin-bottom: 12px;
  }

  .guide-toggle {
    margin-top: 12px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 999px;
    background: #f8f3ea;
    color: #5c5248;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }

  .guide-toggle input {
    accent-color: #8a6500;
  }

  .guide-hint {
    margin-top: 8px;
  }

  .alignment-box {
    margin-top: 18px;
    padding: 14px 16px;
    background: #fdfaf5;
    border: 1px solid #e8e2d8;
    border-radius: 8px;
  }

  .alignment-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }

  .alignment-title {
    color: #3d3630;
    font-size: 14px;
    font-weight: 600;
  }

  .badge {
    padding: 2px 10px;
    border-radius: 10px;
    font-size: 11px;
    font-weight: 600;
  }

  .badge-high {
    background: #e8f5e9;
    color: #2e7d32;
  }

  .badge-medium {
    background: #fff4e5;
    color: #b76e00;
  }

  .badge-low {
    background: #fde8e8;
    color: #b71c1c;
  }

  .badge-muted {
    background: #f0ece4;
    color: #7a7268;
  }

  .alignment-hint {
    margin: 4px 0 12px;
    color: #5c5248;
    font-size: 12px;
    line-height: 1.5;
  }

  .fine-tune-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .fine-tune-row label {
    color: #5c5248;
    font-size: 13px;
    white-space: nowrap;
  }

  .fine-tune-row input[type="range"] {
    flex: 1;
    accent-color: #d35400;
  }

  .fine-tune-value {
    min-width: 62px;
    color: #d35400;
    font-size: 13px;
    font-weight: 600;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .lyrics-feature-hint {
    margin-top: -8px;
  }

  .embedded-subs {
    padding: 10px 14px;
    border: 1px solid #d0daf0;
    border-radius: 8px;
    background: #f0f4ff;
  }

  .sub-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }

  .sub-btn {
    padding: 4px 12px;
    border: 1px solid #93b4f5;
    border-radius: 6px;
    background: white;
    color: #2563eb;
    font-size: 12px;
  }

  .sub-btn:hover:not(:disabled) {
    background: #e8f0ff;
  }

  @media (max-width: 900px) {
    .uvr-info-card {
      align-items: stretch;
      flex-direction: column;
    }

    .uvr-info-card > span {
      align-self: flex-start;
    }

    .uvr-flow-grid,
    .source-grid {
      grid-template-columns: 1fr;
    }

    .fine-tune-row {
      align-items: stretch;
      flex-direction: column;
    }
  }
</style>
