<script lang="ts">
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { loadedMedia, basename } from "../stores/media";
  import { lyricsLines, lyricsFileName, type LyricLine } from "../stores/lyrics";
  import { resetBackingState, backingPitchTrack, clearLiveVocalSamples } from "../stores/pitch";
  import { clearLoop, hasRecording } from "../stores/transport";
  import {
    currentMelody,
    detectedMelodySourceKind,
    melodyStatus,
    melodyToPitchTrack,
    resetMelodyState,
    alignmentResult,
    alignmentFineTuneMs,
    melodySourcePath,
    applyAlignmentToMelody,
    alignmentConfidence,
    finalOffsetSecs,
    type MelodyTrack,
    type AlignmentResult,
    type MelodyStatusMessage,
  } from "../stores/melody";
  import { get } from "svelte/store";
  import {
    inputDeviceIndex,
    outputDeviceIndex,
    latencyMs,
    calibrationStatus,
    resetCalibrationStatus,
    pitchEngine,
    type PitchEngineType,
  } from "../stores/settings";
  import CalibrationVisualizer from "../components/CalibrationVisualizer.svelte";
  import DownloadTab from "./DownloadTab.svelte";
  import { t, tSync } from "../i18n";

  /** 各區塊收合狀態 */
  let sections = $state({
    download: true,
    backing: true,
    lyrics: true,
    melody: false,
    device: false,
    calibration: false,
  });

  interface DeviceInfo {
    name: string;
    index: number;
    is_default: boolean;
  }

  interface DeviceList {
    input_devices: DeviceInfo[];
    output_devices: DeviceInfo[];
  }

  interface LoadResult {
    duration: number;
    sample_rate: number;
    is_video: boolean;
    video_path: string | null;
    /** 自動偵測結果：`"midi"` / `"uvr_cache"` / `null` */
    melody_source: string | null;
  }

  /** 目前載入的伴奏路徑（給「重試載入 melody」按鈕用）。
   *  從 loadedMedia store 推導，切換 tab 後 SetupTab re-mount 也能恢復。 */
  let currentBackingPath = $derived($loadedMedia?.file_path ?? null);

  /** AppSettings 子集合（只取此頁需要的欄位）*/
  interface PartialAppSettings {
    calibrated_latency_ms: number | null;
  }

  interface SubtitleStream {
    index: number;
    language: string;
    title: string;
    codec: string;
  }

  /** 伴奏是否已載入（從 store 推導，切 tab 不會丟失） */
  let backingLoaded = $derived($loadedMedia !== null);

  /** 載入過程中的暫時訊息（如「載入中...」「載入失敗：...」），
   *  非 null 時優先顯示，否則由下方 statusText 從 store 自動推導。 */
  let pendingStatusText = $state<string | null>(null);

  let statusText = $derived.by(() => {
    const translate = $t;
    if (pendingStatusText !== null) return pendingStatusText;
    const m = $loadedMedia;
    if (!m) return translate("setup.backing.hint.empty");
    const min = Math.floor(m.duration / 60);
    const sec = Math.floor(m.duration % 60).toString().padStart(2, "0");
    const kind = m.is_video
      ? translate("setup.backing.kind.video")
      : translate("setup.backing.kind.audio");
    return translate("setup.backing.hint.loaded", {
      kind,
      name: m.file_name,
      min,
      sec,
    });
  });

  /**
   * 歌詞載入狀態訊息（以 i18n 鍵 + 變數保存，讓顯示能隨 locale 切換）。
   * `null` = 預設空狀態，顯示 `setup.lyrics.status.empty`。
   */
  type LyricsStatusMessage =
    | null
    | { key: string; vars?: Record<string, string | number> };
  let lyricsStatus = $state<LyricsStatusMessage>(null);

  let lyricsStatusText = $derived.by(() => {
    const translate = $t;
    const m = lyricsStatus;
    return m
      ? translate(m.key, m.vars)
      : translate("setup.lyrics.status.empty");
  });

  let melodyStatusText = $derived.by(() => {
    const translate = $t;
    const m = $melodyStatus;
    if (!m) return translate("setup.melody.status.empty");
    const base = translate(m.key, m.vars);
    if (m.appendKey) {
      return translate(m.appendKey, { ...(m.appendVars ?? {}), status: base });
    }
    return base;
  });

  /** 影片內嵌字幕軌列表（載入影片後自動偵測） */
  let embeddedSubtitles = $state<SubtitleStream[]>([]);
  let subtitleExtracting = $state(false);

  // 裝置列表與校準狀態
  let devices = $state<DeviceList | null>(null);
  let calibrationResultText = $state("");

  let pitchEngineLoaded = false;

  $effect(() => {
    // Component Mount 時：(1) 載入持久化設定 (2) 列舉硬體裝置
    invoke<PartialAppSettings & Record<string, unknown>>("load_settings")
      .then((s) => {
        if (typeof s.calibrated_latency_ms === "number") {
          $latencyMs = Math.round(s.calibrated_latency_ms);
          calibrationResultText = tSync("setup.calibration.result.lastLoaded", {
            ms: $latencyMs,
          });
        }
        if (typeof s.pitch_engine === "string") {
          $pitchEngine = s.pitch_engine as import("../stores/settings").PitchEngineType;
        }
        pitchEngineLoaded = true;
      })
      .catch((e) => console.error("載入設定失敗:", e));

    invoke<DeviceList>("list_devices")
      .then((res) => {
        devices = res;
        if ($inputDeviceIndex === null) {
          const defIn = res.input_devices.find((d) => d.is_default);
          if (defIn) $inputDeviceIndex = defIn.index;
        }
        if ($outputDeviceIndex === null) {
          const defOut = res.output_devices.find((d) => d.is_default);
          if (defOut) $outputDeviceIndex = defOut.index;
        }
      })
      .catch((e) => console.error("列舉裝置失敗:", e));
  });

  // pitchEngine 變更時同步到後端設定
  $effect(() => {
    const engine = $pitchEngine;
    if (!pitchEngineLoaded) return;
    invoke("update_pitch_engine", { engine })
      .catch((e: unknown) => console.error("同步 pitchEngine 設定失敗:", e));
  });

  async function startCalibration() {
    if ($calibrationStatus.isRunning) return;
    resetCalibrationStatus();
    calibrationResultText = tSync("setup.calibration.result.prep");
    try {
      const res: number = await invoke("calibrate_latency", {
        inputDevice: $inputDeviceIndex,
        outputDevice: $outputDeviceIndex,
      });
      $latencyMs = res;
      // 同步寫回持久化儲存
      try {
        await invoke("update_calibrated_latency", { latencyMs: res });
      } catch (e) {
        console.error("儲存校準值失敗:", e);
      }
      const std = $calibrationStatus.stdDevMs;
      calibrationResultText =
        std !== null
          ? tSync("setup.calibration.result.success", { ms: res, std: std.toFixed(1) })
          : tSync("setup.calibration.result.successNoStd", { ms: res });
    } catch (e) {
      calibrationResultText = tSync("setup.calibration.result.failed", { error: String(e) });
    }
  }

  function dismissVisualizer() {
    // 動畫播完後讓 visualizer overlay 收起來
    resetCalibrationStatus();
  }

  async function loadFile() {
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

    pendingStatusText = tSync("setup.backing.hint.loading");
    // 載入新伴奏前先重設旋律相關狀態，避免上一首的灰藍線殘留
    resetBackingState();
    resetMelodyState();
    clearLoop();
    // 換曲 → 舊錄音也會在後端被清掉（engine.load_backing 內已呼叫 clear_recording），
    // 同步前端 hasRecording，避免「匯出/回放」按鈕誤開。
    hasRecording.set(false);
    clearLiveVocalSamples();
    try {
      const result: LoadResult = await invoke("load_backing", { path });

      const videoUrl = result.video_path ? convertFileSrc(result.video_path) : null;
      loadedMedia.set({
        file_path: path,
        file_name: basename(path),
        duration: result.duration,
        sample_rate: result.sample_rate,
        is_video: result.is_video,
        video_path: result.video_path,
        video_url: videoUrl,
      });
      // 載入成功 → 清掉暫時訊息，讓 statusText 從 store 推導
      pendingStatusText = null;

      // 後端偵測結果（提示來源種類）
      detectedMelodySourceKind.set(result.melody_source);

      // 影片格式：自動偵測內嵌字幕軌
      embeddedSubtitles = [];
      if (result.is_video) {
        try {
          const subs = await invoke<SubtitleStream[]>(
            "probe_embedded_subtitles",
            { videoPath: path },
          );
          embeddedSubtitles = subs;
          if (subs.length > 0) {
            lyricsStatus = {
              key: "setup.lyrics.status.subDetected",
              vars: { n: subs.length },
            };
          }
        } catch (err) {
          console.warn("[setup] 字幕軌偵測失敗（ffprobe 不可用或無字幕）", err);
        }
      }

      // 自動載入目標旋律
      await autoLoadMelodyForPath(path);
    } catch (err) {
      pendingStatusText = tSync("setup.backing.hint.loadFailed", { error: String(err) });
    }
  }

  /**
   * 對伴奏檔案自動偵測並載入目標旋律。
   * 成功：寫入 currentMelody store，同時轉換成 PitchTrack 填入 backingPitchTrack
   *       （讓既有的 PitchTimeline 自動畫出灰藍線，不用改 UI）
   * 失敗：只更新 melodyStatus，不 throw（因為沒有 melody 不是致命錯誤）
   *
   * 若來源檔與 backing 檔不同（例如使用者匯入原曲分離的 vocals.wav），
   * 則由 `loadMelodyFile` / `loadVocalsTrack` 觸發對齊。
   */
  async function autoLoadMelodyForPath(backingPath: string): Promise<void> {
    try {
      const track = await invoke<MelodyTrack | null>(
        "auto_load_melody_for_backing",
        { backingPath },
      );
      if (track) {
        await commitMelodyTrack(track, null);
        const sourceLabel = describeMelodySource(track);
        melodyStatus.set({
          key: "setup.melody.status.autoLoaded",
          vars: { source: sourceLabel, n: track.notes.length },
        });
      } else {
        melodyStatus.set({ key: "setup.melody.status.noAutoDetect" });
      }
    } catch (err) {
      melodyStatus.set({
        key: "setup.melody.status.loadFailed",
        vars: { error: String(err) },
      });
    }
  }

  /** 載入 MIDI 檔作為 melody 來源（手動路徑） */
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
      const track = await invoke<MelodyTrack>("load_melody_from_path", {
        path,
      });
      await commitMelodyTrack(track, path);
      const sourceLabel = describeMelodySource(track);
      melodyStatus.set({
        key: "setup.melody.status.loaded",
        vars: { source: sourceLabel, n: track.notes.length },
      });
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
    } catch (err) {
      melodyStatus.set({
        key: "setup.melody.status.loadFailed",
        vars: { error: String(err) },
      });
    }
  }

  /**
   * 載入「乾淨的人聲音檔」作為 melody 來源（Phase 3-new-c 的主力流程）。
   *
   * 使用者預先用 UVR5 / Moises / Demucs CLI 等外部工具，從原曲分離出
   * `vocals.wav` 後，透過此按鈕匯入。後端跑 YIN 提取音符時間軸，
   * 接著自動與練唱伴奏做 cross-correlation 對齊。
   */
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
      // Vocals 與練唱伴奏通常來自不同檔，需要做自動對齊
      await commitMelodyTrack(track, path);
      const sourceLabel = describeMelodySource(track);
      melodyStatus.set({
        key: "setup.melody.status.vocalsLoaded",
        vars: {
          source: sourceLabel,
          n: track.raw_pitch_track?.length ?? track.notes.length,
        },
      });
      // 若練唱伴奏已載入，自動跑對齊
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
    } catch (err) {
      melodyStatus.set({
        key: "setup.melody.status.vocalsFailed",
        vars: { error: String(err) },
      });
    }
  }

  /**
   * 中央聲道消除：對練唱伴奏進行 L-R 差分消除人聲，
   * 再自動提取旋律。適用於 center-panned 的流行歌。
   */
  async function centerChannelCancel(): Promise<void> {
    if (!currentBackingPath) return;
    melodyStatus.set({ key: "setup.melody.status.cancelling" });
    try {
      const track = await invoke<MelodyTrack>("extract_melody_center_cancel", {
        backingPath: currentBackingPath,
      });
      await commitMelodyTrack(track, null); // 同源，不需對齊
      const count = track.raw_pitch_track?.length ?? track.notes.length;
      melodyStatus.set({
        key: "setup.melody.status.cancelDone",
        vars: { n: count },
      });
    } catch (err) {
      melodyStatus.set({
        key: "setup.melody.status.cancelFailed",
        vars: { error: String(err) },
      });
    }
  }

  /**
   * 載入 melody 的共同後處理：寫入 store + 套用當前對齊 offset + 更新 backingPitchTrack。
   *
   * `sourcePath` 是 melody 來源檔的路徑（給對齊用）；若為 null 代表「無實體檔」
   * 或「與練唱伴奏同源」，不需要對齊。
   */
  async function commitMelodyTrack(
    track: MelodyTrack,
    sourcePath: string | null,
  ): Promise<void> {
    currentMelody.set(track);
    melodySourcePath.set(sourcePath);

    // 若切換來源，清掉舊對齊結果（避免用舊 offset 渲染新 melody）
    alignmentResult.set(null);
    alignmentFineTuneMs.set(0);

    // 把 offset=0 的版本立刻推到 backingPitchTrack，下面的 $effect 會在
    // 使用者調整 fine-tune 或對齊完成後再重新套用
    refreshBackingPitchFromMelody();
  }

  /**
   * 對兩個音檔跑 cross-correlation 自動對齊，結果寫入 alignmentResult store。
   */
  async function runAutoAlignment(
    referencePath: string,
    targetPath: string,
  ): Promise<void> {
    if (referencePath === targetPath) {
      // 同一個檔當然不需要對齊，清空結果即可
      alignmentResult.set(null);
      return;
    }
    try {
      const result = await invoke<AlignmentResult>("align_audio_files", {
        referencePath,
        targetPath,
      });
      alignmentResult.set(result);
    } catch (err) {
      console.error("自動對齊失敗:", err);
      alignmentResult.set(null);
      melodyStatus.update((s) => {
        if (!s) {
          return {
            key: "setup.melody.status.empty",
            appendKey: "setup.melody.status.alignFailedAppend",
            appendVars: { error: String(err) },
          };
        }
        return {
          key: s.key,
          vars: s.vars,
          appendKey: "setup.melody.status.alignFailedAppend",
          appendVars: { error: String(err) },
        };
      });
    }
  }

  /**
   * 根據當前 `currentMelody` + `alignmentResult` + `alignmentFineTuneMs`
   * 重新計算對齊後的 PitchTrack 並推到 `backingPitchTrack` 兼容層。
   *
   * 這個函式由 `$effect` 在任一輸入變化時自動呼叫。
   */
  let lastMelodyOffsetSecs: number | null = null;
  let lastMelodyKey: string | null = null;

  function refreshBackingPitchFromMelody(): void {
    const melody = get(currentMelody);
    if (!melody) {
      backingPitchTrack.set(null);
      lastMelodyOffsetSecs = null;
      lastMelodyKey = null;
      return;
    }
    const offsetSecs = finalOffsetSecs(
      get(alignmentResult),
      get(alignmentFineTuneMs),
    );
    // 如果 melody 和 offset 都沒變，跳過重新計算
    const melodyKey = melody.source.type + (melody.total_duration_secs ?? 0);
    if (offsetSecs === lastMelodyOffsetSecs && melodyKey === lastMelodyKey) return;
    lastMelodyOffsetSecs = offsetSecs;
    lastMelodyKey = melodyKey;
    const aligned = applyAlignmentToMelody(melody, offsetSecs);
    backingPitchTrack.set(melodyToPitchTrack(aligned));
  }

  // 當對齊結果或 fine-tune 值變化時，自動刷新灰藍線
  $effect(() => {
    // Svelte 5 reactive 依賴：讀取這三個 store 讓 effect 訂閱
    void $alignmentResult;
    void $alignmentFineTuneMs;
    void $currentMelody;
    refreshBackingPitchFromMelody();
  });

  function describeMelodySource(track: MelodyTrack): string {
    const src = track.source;
    if (src.type === "midi") {
      return tSync("setup.melody.source.midi", { n: src.track_index + 1 });
    }
    if (src.type === "imported_vocals") {
      const voiced = (src.voiced_ratio * 100).toFixed(0);
      return tSync("setup.melody.source.importedVocals", { ratio: voiced });
    }
    return tSync("setup.melody.source.separated", { model: src.model });
  }

  /** 人類可讀的對齊結果描述 */
  function describeAlignmentOffset(result: AlignmentResult | null): string {
    if (!result) return "";
    const secs = result.offset_secs;
    const sign = secs >= 0 ? "+" : "";
    return `${sign}${secs.toFixed(3)} ${tSync("setup.alignment.offset.seconds")}`;
  }

  /** 提取影片內嵌字幕並載入為歌詞 */
  async function extractAndLoadSubtitle(sub: SubtitleStream): Promise<void> {
    if (!currentBackingPath || subtitleExtracting) return;
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
    } catch (err) {
      lyricsStatus = {
        key: "setup.lyrics.status.subFailed",
        vars: { error: String(err) },
      };
    } finally {
      subtitleExtracting = false;
    }
  }

  /** 字幕軌的語言顯示標籤 */
  function subtitleLabel(sub: SubtitleStream): string {
    const parts: string[] = [];
    if (sub.title) parts.push(sub.title);
    else if (sub.language) parts.push(sub.language);
    parts.push(sub.codec);
    return parts.join(" · ");
  }

  async function loadLyrics() {
    const path = await open({
      title: tSync("setup.lyrics.dialog.title"),
      filters: [
        { name: tSync("setup.lyrics.dialog.filter"), extensions: ["lrc", "srt", "vtt", "txt"] },
      ],
    });
    if (!path) return;

    lyricsStatus = { key: "setup.lyrics.status.parsing" };
    try {
      const lines: LyricLine[] = await invoke("load_lyrics", { path });
      lyricsLines.set(lines);
      const fileName = path.split(/[\\\/]/).pop() || "";
      lyricsFileName.set(fileName);
      lyricsStatus = {
        key: "setup.lyrics.status.loaded",
        vars: { n: lines.length, name: fileName },
      };
    } catch (err) {
      lyricsStatus = {
        key: "setup.lyrics.status.loadFailed",
        vars: { error: String(err) },
      };
    }
  }

  function clearLyrics() {
    lyricsLines.set([]);
    lyricsFileName.set("");
    lyricsStatus = null;
  }
</script>

<div class="setup-page">
  <!-- YouTube 下載 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.download = !sections.download}>
      <h2>{$t("setup.section.download")}</h2>
      <span class="chevron" class:open={sections.download}>▸</span>
    </button>
    {#if sections.download}
      <div class="section-body">
        <DownloadTab />
      </div>
    {/if}
  </div>

  <!-- 練唱伴奏 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.backing = !sections.backing}>
      <h2>{$t("setup.section.backing")}</h2>
      <span class="chevron" class:open={sections.backing}>▸</span>
    </button>
    {#if sections.backing}
      <div class="section-body">
        <p class="hint">{statusText}</p>
        <p class="sub-hint">
          {$t("setup.backing.subHint")}
        </p>
        <div class="actions">
          <button class="btn primary" onclick={loadFile}>{$t("setup.backing.action.import")}</button>
        </div>
      </div>
    {/if}
  </div>

  <!-- 歌詞 / 字幕 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.lyrics = !sections.lyrics}>
      <h2>{$t("setup.section.lyrics")}</h2>
      <span class="chevron" class:open={sections.lyrics}>▸</span>
    </button>
    {#if sections.lyrics}
      <div class="section-body">
        <p class="hint">{lyricsStatusText}</p>

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
      <button class="btn primary" onclick={loadLyrics}>{$t("setup.lyrics.action.load")}</button>
      {#if $lyricsLines.length > 0}
        <button class="btn secondary" onclick={clearLyrics}>{$t("setup.lyrics.action.clear")}</button>
      {/if}
    </div>
      </div>
    {/if}
  </div>

  <!-- 目標旋律 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.melody = !sections.melody}>
      <h2>{$t("setup.section.melody")}</h2>
      <span class="chevron" class:open={sections.melody}>▸</span>
    </button>
    {#if sections.melody}
      <div class="section-body">
    <p class="hint">{melodyStatusText}</p>

    {#if $currentMelody === null}
      <p class="sub-hint">
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
      <button
        class="btn secondary"
        onclick={centerChannelCancel}
        disabled={!backingLoaded}
        title={$t("setup.melody.action.centerCancel.title")}
      >
        {$t("setup.melody.action.centerCancel")}
      </button>
      {#if currentBackingPath && $currentMelody === null}
        <button
          class="btn ghost"
          onclick={() => currentBackingPath && autoLoadMelodyForPath(currentBackingPath)}
        >
          {$t("setup.melody.action.rescan")}
        </button>
      {/if}
    </div>

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
          <p class="alignment-hint">
            {$t("setup.alignment.hint.sameSource")}
          </p>
        {:else if $alignmentResult}
          <p class="alignment-hint">
            {$t("setup.alignment.hint.offset", {
              offset: describeAlignmentOffset($alignmentResult),
              ratio: $alignmentResult.peak_to_mean_ratio.toFixed(1),
            })}
          </p>
        {:else}
          <p class="alignment-hint">
            {$t("setup.alignment.hint.pending")}
          </p>
        {/if}

        <div class="fine-tune-row">
          <label for="fine_tune">{$t("setup.alignment.fineTune.label")}</label>
          <input
            id="fine_tune"
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
      </div>
    {/if}
  </div>

  <!-- 裝置選擇 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.device = !sections.device}>
      <h2>{$t("setup.section.device")}</h2>
      <span class="chevron" class:open={sections.device}>▸</span>
    </button>
    {#if sections.device}
      <div class="section-body">
        <div class="device-selectors">
          <div class="selector-item">
            <label for="input_dev">{$t("setup.device.input.label")}</label>
            <select id="input_dev" bind:value={$inputDeviceIndex}>
              {#if devices}
                {#each devices.input_devices as d}
                  <option value={d.index}>{d.name}</option>
                {/each}
              {:else}
                <option>{$t("setup.device.loading")}</option>
              {/if}
            </select>
          </div>
          <div class="selector-item">
            <label for="output_dev">{$t("setup.device.output.label")}</label>
            <select id="output_dev" bind:value={$outputDeviceIndex}>
              {#if devices}
                {#each devices.output_devices as d}
                  <option value={d.index}>{d.name}</option>
                {/each}
              {:else}
                <option>{$t("setup.device.loading")}</option>
              {/if}
            </select>
          </div>
          <div class="selector-item">
            <label for="pitch_engine">{$t("setup.device.pitch.label")}</label>
            <select id="pitch_engine" bind:value={$pitchEngine}>
              <option value="auto">{$t("setup.device.pitchEngine.auto")}</option>
              <option value="crepe">{$t("setup.device.pitchEngine.crepe")}</option>
              <option value="yin">{$t("setup.device.pitchEngine.yin")}</option>
            </select>
          </div>
        </div>
      </div>
    {/if}
  </div>

  <!-- 延遲校準 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.calibration = !sections.calibration}>
      <h2>{$t("setup.section.calibration")}</h2>
      <span class="chevron" class:open={sections.calibration}>▸</span>
    </button>
    {#if sections.calibration}
      <div class="section-body">
        <p class="hint">{$t("setup.calibration.hint")}</p>

        <div class="slider-row">
          <span>{$latencyMs} ms</span>
          <input type="range" min="0" max="500" bind:value={$latencyMs} class="latency-slider" />
        </div>

        <div class="calibrate-box">
          <h4>{$t("setup.calibration.auto.title")}</h4>
          <p class="hint">{$t("setup.calibration.auto.hint")}</p>
          <div class="actions">
            <button class="btn primary calibrate-btn" onclick={startCalibration} disabled={$calibrationStatus.isRunning}>
              {$calibrationStatus.isRunning ? $t("setup.calibration.action.running") : $t("setup.calibration.action.start")}
            </button>
          </div>
          {#if calibrationResultText}
            <p class="calibration-result">{calibrationResultText}</p>
          {/if}
        </div>
      </div>
    {/if}
  </div>
</div>

<CalibrationVisualizer
  onFinish={dismissVisualizer}
/>

<style>
  .setup-page {
    padding: var(--space-xl);
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
    height: 100%;
    overflow-y: auto;
  }

  .card {
    background: #fff;
    border-radius: 12px;
    padding: 0;
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 14px 20px;
    border: none;
    background: transparent;
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .section-header:hover {
    background: var(--color-bg-hover);
  }

  .section-header h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--color-text);
  }

  .chevron {
    font-size: 14px;
    color: var(--color-text-muted);
    transition: transform var(--transition-normal);
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  .section-body {
    padding: 0 20px 16px;
  }

  .card h2 {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 600;
    color: #3d3630;
  }

  .hint {
    margin: 0 0 16px;
    font-size: 14px;
    color: #7a7268;
  }

  .sub-hint {
    margin: -8px 0 12px;
    font-size: 12px;
    color: #a0958a;
    line-height: 1.5;
  }

  .actions {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
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
    font-size: 14px;
    font-weight: 600;
    color: #3d3630;
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
    font-size: 12px;
    color: #5c5248;
    line-height: 1.5;
  }

  .fine-tune-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .fine-tune-row label {
    font-size: 13px;
    color: #5c5248;
    white-space: nowrap;
  }

  .fine-tune-row input[type="range"] {
    flex: 1;
    accent-color: #d35400;
  }

  .fine-tune-value {
    min-width: 62px;
    text-align: right;
    font-size: 13px;
    font-weight: 600;
    color: #d35400;
    font-variant-numeric: tabular-nums;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .device-selectors {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .selector-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .selector-item label {
    font-size: 13px;
    color: #7a7268;
    font-weight: 500;
  }

  .selector-item select {
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid #e8e2d8;
    background: #faf8f4;
    font-size: 14px;
    color: #3d3630;
    outline: none;
  }
  
  .slider-row {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 24px;
  }

  .slider-row span {
    font-size: 14px;
    font-weight: bold;
    color: #755700;
    min-width: 50px;
  }

  .latency-slider {
    flex: 1;
    accent-color: #755700;
  }

  .calibrate-box {
    background: #fdfaf5;
    border: 1px solid #e8e2d8;
    padding: 16px;
    border-radius: 8px;
  }

  .calibrate-box h4 {
    margin: 0 0 8px 0;
    color: #3d3630;
    font-size: 14px;
  }
  
  .calibrate-btn {
    background: #d35400;
  }
  
  .calibrate-btn:hover {
    background: #a04000;
  }

  .calibration-result {
    margin: 12px 0 0 0;
    font-size: 13px;
    color: #d35400;
    font-weight: bold;
  }

  /* 內嵌字幕區 */
  .embedded-subs {
    background: #f0f4ff;
    border: 1px solid #d0daf0;
    border-radius: 8px;
    padding: 10px 14px;
    margin-bottom: 8px;
  }

  .sub-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }

  .sub-btn {
    background: white;
    border: 1px solid #93b4f5;
    color: #2563eb;
    font-size: 12px;
    padding: 4px 12px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .sub-btn:hover:not(:disabled) {
    background: #e8f0ff;
  }

  .sub-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
