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
    if (pendingStatusText !== null) return pendingStatusText;
    const m = $loadedMedia;
    if (!m) return "請載入伴奏檔案開始練唱";
    const min = Math.floor(m.duration / 60);
    const sec = Math.floor(m.duration % 60);
    const kind = m.is_video ? "影片" : "音訊";
    return `已載入${kind}：${m.file_name}（${min}:${sec.toString().padStart(2, "0")}）`;
  });

  let lyricsStatus = $state("尚未載入歌詞");

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
          calibrationResultText = `已載入上次校準值：${$latencyMs} ms`;
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
    calibrationResultText = "校準中，請看著畫面的球，碰到準線時對麥克風拍手...";
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
          ? `校準完成！延遲 ${res} ms（標準差 ${std.toFixed(1)} ms）`
          : `校準完成！延遲 ${res} ms`;
    } catch (e) {
      calibrationResultText = `校準失敗：${e}`;
    }
  }

  function dismissVisualizer() {
    // 動畫播完後讓 visualizer overlay 收起來
    resetCalibrationStatus();
  }

  async function loadFile() {
    const path = await open({
      title: "選擇伴奏檔案",
      filters: [
        {
          name: "音訊/影片",
          extensions: ["wav", "mp3", "mp4", "m4a", "aac", "flac", "ogg", "mkv", "webm"],
        },
      ],
    });
    if (!path) return;

    pendingStatusText = "載入中...";
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
            lyricsStatus = `偵測到 ${subs.length} 個內嵌字幕軌，可直接提取為歌詞`;
          }
        } catch (err) {
          console.warn("[setup] 字幕軌偵測失敗（ffprobe 不可用或無字幕）", err);
        }
      }

      // 自動載入目標旋律
      await autoLoadMelodyForPath(path);
    } catch (err) {
      pendingStatusText = `載入失敗：${err}`;
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
        melodyStatus.set(
          `已自動載入目標旋律（${sourceLabel}，共 ${track.notes.length} 個音符）`,
        );
      } else {
        melodyStatus.set(
          "沒有自動偵測到目標旋律。可改用下方「匯入人聲軌」或「載入 MIDI」",
        );
      }
    } catch (err) {
      melodyStatus.set(`旋律載入失敗：${err}`);
    }
  }

  /** 載入 MIDI 檔作為 melody 來源（手動路徑） */
  async function loadMelodyFile(): Promise<void> {
    const path = await open({
      title: "選擇 MIDI 旋律檔",
      filters: [
        { name: "MIDI", extensions: ["mid", "midi"] },
      ],
    });
    if (!path) return;

    melodyStatus.set("正在解析旋律軌...");
    try {
      const track = await invoke<MelodyTrack>("load_melody_from_path", {
        path,
      });
      await commitMelodyTrack(track, path);
      const sourceLabel = describeMelodySource(track);
      melodyStatus.set(
        `已載入旋律（${sourceLabel}，${track.notes.length} 個音符）。若時間對不上，請用 fine-tune 微調。`,
      );
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
    } catch (err) {
      melodyStatus.set(`旋律載入失敗：${err}`);
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
      title: "選擇已分離的人聲音檔 (vocals.wav)",
      filters: [
        {
          name: "音訊",
          extensions: ["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus"],
        },
      ],
    });
    if (!path) return;

    melodyStatus.set("分析人聲音高中（YIN + 群聚）...");
    try {
      const track = await invoke<MelodyTrack>("load_vocals_and_extract_melody", {
        vocalsPath: path,
      });
      // Vocals 與練唱伴奏通常來自不同檔，需要做自動對齊
      await commitMelodyTrack(track, path);
      const sourceLabel = describeMelodySource(track);
      melodyStatus.set(
        `已匯入人聲軌（${sourceLabel}，${track.raw_pitch_track?.length ?? track.notes.length} 個音高樣本）`,
      );
      // 若練唱伴奏已載入，自動跑對齊
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
    } catch (err) {
      melodyStatus.set(`人聲分析失敗：${err}`);
    }
  }

  /**
   * 中央聲道消除：對練唱伴奏進行 L-R 差分消除人聲，
   * 再自動提取旋律。適用於 center-panned 的流行歌。
   */
  async function centerChannelCancel(): Promise<void> {
    if (!currentBackingPath) return;
    melodyStatus.set("中央聲道消除 + 音高分析中...");
    try {
      const track = await invoke<MelodyTrack>("extract_melody_center_cancel", {
        backingPath: currentBackingPath,
      });
      await commitMelodyTrack(track, null); // 同源，不需對齊
      const count = track.raw_pitch_track?.length ?? track.notes.length;
      melodyStatus.set(`中央聲道消除完成（${count} 個音高樣本）`);
    } catch (err) {
      melodyStatus.set(`中央聲道消除失敗：${err}`);
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
      melodyStatus.update(
        (s) => `${s}（自動對齊失敗：${err}，請用 fine-tune 手動微調）`,
      );
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
      return `MIDI：Track ${src.track_index + 1}`;
    }
    if (src.type === "imported_vocals") {
      const voiced = (src.voiced_ratio * 100).toFixed(0);
      return `voiced ${voiced}%`;
    }
    return `人聲分離：${src.model}`;
  }

  /** 人類可讀的對齊結果描述 */
  function describeAlignmentOffset(result: AlignmentResult | null): string {
    if (!result) return "";
    const secs = result.offset_secs;
    const sign = secs >= 0 ? "+" : "";
    return `${sign}${secs.toFixed(3)} 秒`;
  }

  /** 提取影片內嵌字幕並載入為歌詞 */
  async function extractAndLoadSubtitle(sub: SubtitleStream): Promise<void> {
    if (!currentBackingPath || subtitleExtracting) return;
    subtitleExtracting = true;
    lyricsStatus = `提取字幕軌 #${sub.index}（${sub.language || sub.codec}）...`;
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
      lyricsStatus = `已提取 ${lines.length} 行歌詞（${fileName}）`;
    } catch (err) {
      lyricsStatus = `字幕提取失敗：${err}`;
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
      title: "選擇歌詞檔案",
      filters: [
        { name: "歌詞檔", extensions: ["lrc", "srt", "vtt", "txt"] },
      ],
    });
    if (!path) return;

    lyricsStatus = "解析中...";
    try {
      const lines: LyricLine[] = await invoke("load_lyrics", { path });
      lyricsLines.set(lines);
      const fileName = path.split(/[\\\/]/).pop() || "";
      lyricsFileName.set(fileName);
      lyricsStatus = `已載入 ${lines.length} 行歌詞（${fileName}）`;
    } catch (err) {
      lyricsStatus = `載入失敗：${err}`;
    }
  }

  function clearLyrics() {
    lyricsLines.set([]);
    lyricsFileName.set("");
    lyricsStatus = "尚未載入歌詞";
  }
</script>

<div class="setup-page">
  <!-- YouTube 下載 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.download = !sections.download}>
      <h2>YouTube 下載</h2>
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
      <h2>練唱伴奏</h2>
      <span class="chevron" class:open={sections.backing}>▸</span>
    </button>
    {#if sections.backing}
      <div class="section-body">
        <p class="hint">{statusText}</p>
        <p class="sub-hint">
          載入伴奏版 / off vocal，避免原唱蓋過自己的聲音。
        </p>
        <div class="actions">
          <button class="btn primary" onclick={loadFile}>本機匯入</button>
        </div>
      </div>
    {/if}
  </div>

  <!-- 歌詞 / 字幕 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.lyrics = !sections.lyrics}>
      <h2>歌詞 / 字幕</h2>
      <span class="chevron" class:open={sections.lyrics}>▸</span>
    </button>
    {#if sections.lyrics}
      <div class="section-body">
        <p class="hint">{lyricsStatus}</p>

    {#if embeddedSubtitles.length > 0}
      <div class="embedded-subs">
        <p class="sub-hint">偵測到影片內嵌字幕，點擊即可提取為歌詞：</p>
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
      <button class="btn primary" onclick={loadLyrics}>載入 LRC / SRT / VTT</button>
      {#if $lyricsLines.length > 0}
        <button class="btn secondary" onclick={clearLyrics}>清除歌詞</button>
      {/if}
    </div>
      </div>
    {/if}
  </div>

  <!-- 目標旋律 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.melody = !sections.melody}>
      <h2>目標旋律來源</h2>
      <span class="chevron" class:open={sections.melody}>▸</span>
    </button>
    {#if sections.melody}
      <div class="section-body">
    <p class="hint">{$melodyStatus}</p>

    {#if $currentMelody === null}
      <p class="sub-hint">
        請匯入<strong>預先分離好的人聲軌</strong>或<strong>手動載入 MIDI</strong>作為參考旋律。
      </p>
    {/if}

    <div class="actions">
      <button
        class="btn primary"
        onclick={loadVocalsTrack}
        disabled={!backingLoaded}
        title="匯入使用者預先用 UVR5 / Moises 分離好的乾淨人聲軌"
      >
        🎤 匯入人聲軌 (vocals.wav)
      </button>
      <button
        class="btn secondary"
        onclick={loadMelodyFile}
        disabled={!backingLoaded}
      >
        載入 MIDI
      </button>
      <button
        class="btn secondary"
        onclick={centerChannelCancel}
        disabled={!backingLoaded}
        title="對立體聲伴奏進行 L-R 差分消除人聲（適用 center-panned 流行歌，mono 不適用）"
      >
        🔇 快速消人聲
      </button>
      {#if currentBackingPath && $currentMelody === null}
        <button
          class="btn ghost"
          onclick={() => currentBackingPath && autoLoadMelodyForPath(currentBackingPath)}
        >
          重新掃描
        </button>
      {/if}
    </div>

    {#if $currentMelody}
      <div class="alignment-box">
        <div class="alignment-header">
          <span class="alignment-title">時間軸對齊</span>
          {#if $melodySourcePath === null}
            <span class="badge badge-muted">不需對齊</span>
          {:else if $alignmentResult}
            {#if alignmentConfidence($alignmentResult) === "high"}
              <span class="badge badge-high">高信心</span>
            {:else if alignmentConfidence($alignmentResult) === "medium"}
              <span class="badge badge-medium">中信心</span>
            {:else}
              <span class="badge badge-low">低信心，建議微調</span>
            {/if}
          {:else}
            <span class="badge badge-muted">尚未對齊</span>
          {/if}
        </div>

        {#if $melodySourcePath === null}
          <p class="alignment-hint">
            目前 melody 來源與練唱伴奏視為同源，若時間對不上可用下方的 fine-tune 微調。
          </p>
        {:else if $alignmentResult}
          <p class="alignment-hint">
            自動偵測到偏移：<strong>{describeAlignmentOffset($alignmentResult)}</strong>
            （peak/mean = {$alignmentResult.peak_to_mean_ratio.toFixed(1)}）
          </p>
        {:else}
          <p class="alignment-hint">
            對齊結果尚未計算。若已載入練唱伴奏與人聲軌兩個不同檔，
            系統會自動跑 cross-correlation。
          </p>
        {/if}

        <div class="fine-tune-row">
          <label for="fine_tune">手動微調：</label>
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
              title="重置微調"
            >
              歸零
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
      <h2>裝置選擇</h2>
      <span class="chevron" class:open={sections.device}>▸</span>
    </button>
    {#if sections.device}
      <div class="section-body">
        <div class="device-selectors">
          <div class="selector-item">
            <label for="input_dev">麥克風 (Input):</label>
            <select id="input_dev" bind:value={$inputDeviceIndex}>
              {#if devices}
                {#each devices.input_devices as d}
                  <option value={d.index}>{d.name}</option>
                {/each}
              {:else}
                <option>載入中...</option>
              {/if}
            </select>
          </div>
          <div class="selector-item">
            <label for="output_dev">耳機 (Output):</label>
            <select id="output_dev" bind:value={$outputDeviceIndex}>
              {#if devices}
                {#each devices.output_devices as d}
                  <option value={d.index}>{d.name}</option>
                {/each}
              {:else}
                <option>載入中...</option>
              {/if}
            </select>
          </div>
          <div class="selector-item">
            <label for="pitch_engine">音高偵測引擎:</label>
            <select id="pitch_engine" bind:value={$pitchEngine}>
              <option value="auto">自動（有 CREPE 模型則用 CREPE）</option>
              <option value="crepe">CREPE AI（推薦，較準確）</option>
              <option value="yin">YIN（輕量，低延遲）</option>
            </select>
          </div>
        </div>
      </div>
    {/if}
  </div>

  <!-- 延遲校準 -->
  <div class="card">
    <button class="section-header" onclick={() => sections.calibration = !sections.calibration}>
      <h2>延遲校準</h2>
      <span class="chevron" class:open={sections.calibration}>▸</span>
    </button>
    {#if sections.calibration}
      <div class="section-body">
        <p class="hint">補償錄音時的軟硬體延遲（重啟後會自動載入上次校準值）</p>

        <div class="slider-row">
          <span>{$latencyMs} ms</span>
          <input type="range" min="0" max="500" bind:value={$latencyMs} class="latency-slider" />
        </div>

        <div class="calibrate-box">
          <h4>自動打拍子校準</h4>
          <p class="hint">戴上耳機後點擊下方按鈕。系統會發出 8 次木魚聲（前 2 次暖身、後 6 次量測），請跟著節奏對著麥克風拍手或發出短促音。</p>
          <div class="actions">
            <button class="btn primary calibrate-btn" onclick={startCalibration} disabled={$calibrationStatus.isRunning}>
              {$calibrationStatus.isRunning ? "校準進行中..." : "開始互動式校準"}
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
