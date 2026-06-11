<script lang="ts">
  import { onMount, onDestroy } from "svelte";
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
    guideVocalPath,
    applyAlignmentToMelody,
    alignmentConfidence,
    finalOffsetSecs,
    type MelodyTrack,
    type AlignmentResult,
    type MelodyStatusMessage,
    type TranslatableDescriptor,
  } from "../stores/melody";
  import { get } from "svelte/store";
  import {
    inputDeviceIndex,
    outputDeviceIndex,
    latencyMs,
    pitchEngine,
    guideVocalEnabled,
  } from "../stores/settings";
  import LatencyFineTuneControl from "../components/LatencyFineTuneControl.svelte";
  import DownloadTab from "./DownloadTab.svelte";
  import { t, tSync } from "../i18n";
  import { projectSessionReady } from "../stores/projectSession";

  type SetupSectionKey =
    | "download"
    | "device"
    | "calibration";
  type SetupSections = Record<SetupSectionKey, boolean>;

  const SETUP_SECTIONS_STORAGE_KEY = "vocalsync.setup.sections.v1";
  const PROJECT_SESSION_VERSION = 1;
  const DEFAULT_SECTIONS: SetupSections = {
    download: true,
    device: true,
    calibration: true,
  };

  /** 各區塊收合狀態 */
  let sections = $state<SetupSections>({ ...DEFAULT_SECTIONS });
  let sectionsLoaded = $state(false);

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

  interface ProjectSession {
    version: typeof PROJECT_SESSION_VERSION;
    backingPath: string | null;
    lyricsFileName: string;
    lyricsLines: LyricLine[];
    melody: MelodyTrack | null;
    melodySourcePath: string | null;
    guideVocalPath: string | null;
    guideVocalEnabled: boolean;
    alignmentResult: AlignmentResult | null;
    alignmentFineTuneMs: number;
  }

  /** 目前載入的伴奏路徑（給「重試載入 melody」按鈕用）。
   *  從 loadedMedia store 推導，切換 tab 後 SetupTab re-mount 也能恢復。 */
  let currentBackingPath = $derived($loadedMedia?.file_path ?? null);

  /** AppSettings 子集合（只取此頁需要的欄位）*/
  interface LatencyCalibrationProfile {
    key: string;
    input_device: string;
    output_device: string;
    sample_rate: number;
    latency_ms: number;
    confidence: string;
    updated_at_unix: number;
  }

  interface PartialAppSettings {
    calibrated_latency_ms: number | null;
    calibrated_latency_profiles?: LatencyCalibrationProfile[];
    pitch_engine?: string;
  }

  type CalibrationConfidence = "high" | "medium" | "low" | "manual" | "estimated";

  interface CalibrationResult {
    latency_ms: number;
    confidence: CalibrationConfidence;
    rounds_used: number;
    valid_beats: number;
    measurement_beats: number;
    std_dev_ms: number;
    round_spread_ms: number;
    applied_recommended: boolean;
    diagnostic: string;
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
    // 先把 nestedVars（如 source descriptor）翻成文字，再 merge 進外層 vars，
    // 這樣切換 locale 時連內層片段也會跟著重翻。
    let mergedVars = m.vars;
    if (m.nestedVars) {
      const translated: Record<string, string | number> = { ...(m.vars ?? {}) };
      for (const [field, desc] of Object.entries(m.nestedVars)) {
        translated[field] = translate(desc.key, desc.vars);
      }
      mergedVars = translated;
    }
    const base = translate(m.key, mergedVars);
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
  let loadedSettings = $state<PartialAppSettings | null>(null);
  let skipNextProfileApply = $state(false);
  let calibrationResultText = $state("");
  let manualCalibrationOpen = $state(false);
  let systemCalibrationBusy = $state(false);
  let rhythmVoiceCalibrationOpen = $state(false);
  let rhythmVoiceCalibrationBusy = $state(false);
  let calibrationBusy = $derived(systemCalibrationBusy || rhythmVoiceCalibrationBusy);
  let deviceMsg = $state("");

  let pitchEngineLoaded = false;
  let projectSessionLoaded = false;
  let projectSessionRestoring = false;
  let projectSessionSaveTimer: number | null = null;

  function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null;
  }

  function currentSampleRate(): number {
    return $loadedMedia?.sample_rate ?? 44100;
  }

  function selectedDeviceName(list: DeviceInfo[] | undefined, index: number | null): string | null {
    if (!list || index === null) return null;
    return list.find((device) => device.index === index)?.name ?? null;
  }

  function currentInputDeviceName(): string | null {
    return selectedDeviceName(devices?.input_devices, $inputDeviceIndex);
  }

  function currentOutputDeviceName(): string | null {
    return selectedDeviceName(devices?.output_devices, $outputDeviceIndex);
  }

  function latencyProfileKey(
    inputName: string,
    outputName: string,
    sampleRate: number,
  ): string {
    return `${inputName.trim().toLowerCase()}|${outputName.trim().toLowerCase()}|${sampleRate}`;
  }

  function applyStoredLatencyForCurrentDevices(settings: PartialAppSettings): void {
    const inputName = currentInputDeviceName();
    const outputName = currentOutputDeviceName();
    const sampleRate = currentSampleRate();
    const profile =
      inputName && outputName
        ? settings.calibrated_latency_profiles?.find(
            (item) => item.key === latencyProfileKey(inputName, outputName, sampleRate),
          )
        : null;

    if (profile && typeof profile.latency_ms === "number") {
      $latencyMs = Math.round(profile.latency_ms);
      calibrationResultText = tSync("setup.calibration.result.profileLoaded", {
        ms: $latencyMs,
        confidence: tSync(`calibration.confidence.${profile.confidence}`),
      });
      return;
    }

    if (typeof settings.calibrated_latency_ms === "number") {
      $latencyMs = Math.round(settings.calibrated_latency_ms);
      calibrationResultText = tSync("setup.calibration.result.lastLoaded", {
        ms: $latencyMs,
      });
    }
  }

  function updateLocalLatencyProfile(result: CalibrationResult): void {
    if (!loadedSettings) return;
    const inputName = currentInputDeviceName();
    const outputName = currentOutputDeviceName();
    const sampleRate = currentSampleRate();
    if (!inputName || !outputName) {
      loadedSettings = {
        ...loadedSettings,
        calibrated_latency_ms: result.latency_ms,
      };
      return;
    }

    const key = latencyProfileKey(inputName, outputName, sampleRate);
    const profile: LatencyCalibrationProfile = {
      key,
      input_device: inputName,
      output_device: outputName,
      sample_rate: sampleRate,
      latency_ms: result.latency_ms,
      confidence: result.confidence,
      updated_at_unix: Math.floor(Date.now() / 1000),
    };
    const profiles = [...(loadedSettings.calibrated_latency_profiles ?? [])];
    const existingIdx = profiles.findIndex((item) => item.key === key);
    if (existingIdx >= 0) {
      profiles[existingIdx] = profile;
    } else {
      profiles.push(profile);
    }
    loadedSettings = {
      ...loadedSettings,
      calibrated_latency_ms: result.latency_ms,
      calibrated_latency_profiles: profiles,
    };
  }

  function sanitizeLyricsLines(value: unknown): LyricLine[] {
    if (!Array.isArray(value)) return [];
    return value.filter((line): line is LyricLine => {
      if (!isRecord(line)) return false;
      return (
        typeof line.start_ms === "number" &&
        typeof line.end_ms === "number" &&
        typeof line.text === "string"
      );
    });
  }

  function sanitizeProjectSession(value: unknown): ProjectSession | null {
    if (!isRecord(value) || value.version !== PROJECT_SESSION_VERSION) return null;
    return {
      version: PROJECT_SESSION_VERSION,
      backingPath: typeof value.backingPath === "string" ? value.backingPath : null,
      lyricsFileName:
        typeof value.lyricsFileName === "string" ? value.lyricsFileName : "",
      lyricsLines: sanitizeLyricsLines(value.lyricsLines),
      melody:
        isRecord(value.melody) && Array.isArray(value.melody.notes)
          ? (value.melody as MelodyTrack)
          : null,
      melodySourcePath:
        typeof value.melodySourcePath === "string" ? value.melodySourcePath : null,
      guideVocalPath:
        typeof value.guideVocalPath === "string" ? value.guideVocalPath : null,
      guideVocalEnabled: value.guideVocalEnabled === true,
      alignmentResult: isRecord(value.alignmentResult)
        ? (value.alignmentResult as AlignmentResult)
        : null,
      alignmentFineTuneMs:
        typeof value.alignmentFineTuneMs === "number"
          ? value.alignmentFineTuneMs
          : 0,
    };
  }

  function createProjectSessionSnapshot(): ProjectSession {
    return {
      version: PROJECT_SESSION_VERSION,
      backingPath: get(loadedMedia)?.file_path ?? null,
      lyricsFileName: get(lyricsFileName),
      lyricsLines: get(lyricsLines),
      melody: get(currentMelody),
      melodySourcePath: get(melodySourcePath),
      guideVocalPath: get(guideVocalPath),
      guideVocalEnabled: get(guideVocalEnabled),
      alignmentResult: get(alignmentResult),
      alignmentFineTuneMs: get(alignmentFineTuneMs),
    };
  }

  async function saveProjectSessionNow(): Promise<void> {
    if (!projectSessionLoaded || projectSessionRestoring) return;
    try {
      await invoke("save_project_session", {
        sessionJson: JSON.stringify(createProjectSessionSnapshot()),
      });
    } catch (err) {
      console.warn("[setup] project session save failed:", err);
    }
  }

  function scheduleProjectSessionSave(): void {
    if (!projectSessionLoaded || projectSessionRestoring) return;
    if (projectSessionSaveTimer !== null) {
      window.clearTimeout(projectSessionSaveTimer);
    }
    projectSessionSaveTimer = window.setTimeout(() => {
      projectSessionSaveTimer = null;
      void saveProjectSessionNow();
    }, 200);
  }

  async function restoreProjectSession(): Promise<void> {
    projectSessionRestoring = true;
    try {
      const raw = await invoke<string | null>("load_project_session");
      const session = raw ? sanitizeProjectSession(JSON.parse(raw)) : null;
      if (!session) return;

      if (session.lyricsLines.length > 0) {
        lyricsLines.set(session.lyricsLines);
        lyricsFileName.set(session.lyricsFileName);
        lyricsStatus = {
          key: "setup.lyrics.status.restored",
          vars: { n: session.lyricsLines.length },
        };
      }

      if (session.backingPath) {
        try {
          await loadBackingFromPath(session.backingPath, {
            resetDependents: false,
            autoDetectMelody: false,
            probeSubtitles: false,
          });
        } catch (err) {
          pendingStatusText = tSync("setup.backing.hint.restoreFailed", {
            error: String(err),
          });
        }
      }

      if (session.melody) {
        currentMelody.set(session.melody);
        melodySourcePath.set(session.melodySourcePath);
        alignmentResult.set(session.alignmentResult);
        alignmentFineTuneMs.set(session.alignmentFineTuneMs);
        refreshBackingPitchFromMelody();
        melodyStatus.set({
          key: "setup.melody.status.restored",
          vars: { n: session.melody.notes.length },
        });
      }

      if (session.guideVocalPath) {
        try {
          await loadGuideVocalTrack(session.guideVocalPath);
          guideVocalEnabled.set(session.guideVocalEnabled);
          await syncGuideVocalTiming();
        } catch (err) {
          guideVocalPath.set(null);
          guideVocalEnabled.set(false);
          melodyStatus.update((s) =>
            s
              ? {
                  ...s,
                  appendKey: "setup.guide.status.loadFailedAppend",
                  appendVars: { error: String(err) },
                }
              : s,
          );
        }
      }
    } catch (err) {
      console.warn("[setup] project session restore failed:", err);
    } finally {
      projectSessionRestoring = false;
      projectSessionLoaded = true;
      projectSessionReady.set(true);
      void saveProjectSessionNow();
    }
  }

  function loadPersistedSections(): void {
    try {
      const raw = localStorage.getItem(SETUP_SECTIONS_STORAGE_KEY);
      if (!raw) {
        sections = { ...DEFAULT_SECTIONS };
        return;
      }
      const saved = JSON.parse(raw) as Partial<Record<SetupSectionKey, unknown>>;
      sections = { ...DEFAULT_SECTIONS };
      for (const key of Object.keys(DEFAULT_SECTIONS) as SetupSectionKey[]) {
        if (typeof saved[key] === "boolean") {
          sections[key] = saved[key];
        }
      }
    } catch (err) {
      console.warn("[setup] section layout restore failed:", err);
      sections = { ...DEFAULT_SECTIONS };
    }
  }

  function toggleSection(key: SetupSectionKey): void {
    sections[key] = !sections[key];
  }

  onMount(() => {
    loadPersistedSections();
    sectionsLoaded = true;
    if (get(projectSessionReady)) {
      projectSessionLoaded = true;
    } else {
      void restoreProjectSession();
    }
  });

  onDestroy(() => {
    if (projectSessionSaveTimer !== null) {
      window.clearTimeout(projectSessionSaveTimer);
      projectSessionSaveTimer = null;
    }
    void saveProjectSessionNow();
  });

  $effect(() => {
    const snapshot = JSON.stringify(sections);
    if (!sectionsLoaded) return;
    try {
      localStorage.setItem(SETUP_SECTIONS_STORAGE_KEY, snapshot);
    } catch (err) {
      console.warn("[setup] section layout save failed:", err);
    }
  });

  $effect(() => {
    void $loadedMedia;
    void $lyricsFileName;
    void $lyricsLines;
    void $currentMelody;
    void $melodySourcePath;
    void $guideVocalPath;
    void $guideVocalEnabled;
    void $alignmentResult;
    void $alignmentFineTuneMs;
    scheduleProjectSessionSave();
  });

  $effect(() => {
    // Component Mount 時：(1) 載入持久化設定 (2) 列舉硬體裝置
    invoke<PartialAppSettings & Record<string, unknown>>("load_settings")
      .then((s) => {
        loadedSettings = s;
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

  $effect(() => {
    const settings = loadedSettings;
    void devices;
    void $inputDeviceIndex;
    void $outputDeviceIndex;
    void $loadedMedia?.sample_rate;
    if (!settings || !devices) return;
    if (skipNextProfileApply) {
      skipNextProfileApply = false;
      return;
    }
    applyStoredLatencyForCurrentDevices(settings);
  });

  // pitchEngine 變更時同步到後端設定
  $effect(() => {
    const engine = $pitchEngine;
    if (!pitchEngineLoaded) return;
    invoke("update_pitch_engine", { engine })
      .catch((e: unknown) => console.error("同步 pitchEngine 設定失敗:", e));
  });

  function latencyDiagnosticText(code: string | null | undefined): string {
    const key = code
      ? `setup.calibration.diagnostic.${code}`
      : "setup.calibration.diagnostic.low_confidence";
    return tSync(key);
  }

  function clampLatency(value: number): number {
    return Math.max(0, Math.min(5000, Math.round(value)));
  }

  function calibrationConfidenceText(confidence: string): string {
    return tSync(`calibration.confidence.${confidence}`);
  }

  async function saveLatencyProfile(
    latencyMsValue: number,
    confidence: CalibrationConfidence,
  ): Promise<void> {
    const latency = clampLatency(latencyMsValue);
    $latencyMs = latency;
    await invoke("update_calibrated_latency", {
      latencyMs: latency,
      inputDeviceName: currentInputDeviceName(),
      outputDeviceName: currentOutputDeviceName(),
      sampleRate: currentSampleRate(),
      confidence,
    });
    skipNextProfileApply = true;
    updateLocalLatencyProfile({
      latency_ms: latency,
      confidence,
      rounds_used: 0,
      valid_beats: 0,
      measurement_beats: 0,
      std_dev_ms: 0,
      round_spread_ms: 0,
      applied_recommended: true,
      diagnostic: "",
    });
  }

  async function startSystemCalibration(): Promise<void> {
    if (calibrationBusy) return;
    const oldLatencyMs = $latencyMs;
    systemCalibrationBusy = true;
    calibrationResultText = tSync("setup.calibration.system.result.prep");
    try {
      const res: CalibrationResult = await invoke("estimate_system_latency", {
        inputDevice: $inputDeviceIndex,
        outputDevice: $outputDeviceIndex,
        sampleRate: currentSampleRate(),
      });
      const confidence = calibrationConfidenceText(res.confidence);

      if (res.applied_recommended) {
        await saveLatencyProfile(res.latency_ms, res.confidence);
        calibrationResultText = tSync("setup.calibration.system.result.success", {
          ms: res.latency_ms,
          confidence,
          diagnostic: latencyDiagnosticText(res.diagnostic),
          spread: res.round_spread_ms.toFixed(1),
        });
      } else {
        $latencyMs = oldLatencyMs;
        calibrationResultText = tSync("setup.calibration.system.result.kept", {
          oldMs: oldLatencyMs,
          ms: res.latency_ms,
          confidence,
          reason: latencyDiagnosticText(res.diagnostic),
        });
      }
    } catch (e) {
      $latencyMs = oldLatencyMs;
      calibrationResultText = tSync("setup.calibration.result.failed", { error: String(e) });
    } finally {
      systemCalibrationBusy = false;
    }
  }

  async function startRhythmVoiceCalibration(): Promise<void> {
    if (calibrationBusy) return;
    const oldLatencyMs = $latencyMs;
    rhythmVoiceCalibrationBusy = true;
    calibrationResultText = tSync("setup.calibration.rhythmVoice.result.prep");
    try {
      const res: CalibrationResult = await invoke("calibrate_latency_rhythm_voice", {
        inputDevice: $inputDeviceIndex,
        outputDevice: $outputDeviceIndex,
        sampleRate: currentSampleRate(),
      });
      const confidence = calibrationConfidenceText(res.confidence);
      const shouldApply =
        res.applied_recommended &&
        (res.confidence === "high" || res.confidence === "medium");

      if (shouldApply) {
        await saveLatencyProfile(res.latency_ms, res.confidence);
        calibrationResultText = tSync("setup.calibration.rhythmVoice.result.success", {
          ms: res.latency_ms,
          confidence,
          valid: res.valid_beats,
          total: res.measurement_beats,
          spread: res.round_spread_ms.toFixed(1),
        });
      } else {
        $latencyMs = oldLatencyMs;
        calibrationResultText = tSync("setup.calibration.rhythmVoice.result.kept", {
          oldMs: oldLatencyMs,
          ms: res.latency_ms,
          confidence,
          reason: latencyDiagnosticText(res.diagnostic),
        });
      }
    } catch (e) {
      $latencyMs = oldLatencyMs;
      calibrationResultText = tSync("setup.calibration.result.failed", { error: String(e) });
    } finally {
      rhythmVoiceCalibrationBusy = false;
    }
  }

  async function openCrepeModelFolder() {
    try {
      const path = await invoke<string>("open_crepe_model_folder");
      deviceMsg = tSync("setup.device.modelFolder.opened", { path });
    } catch (err) {
      deviceMsg = tSync("setup.device.modelFolder.failed", { error: String(err) });
    }
  }

  function currentGuideOffsetSecs(): number {
    return finalOffsetSecs(get(alignmentResult), get(alignmentFineTuneMs));
  }

  async function clearGuideVocalTrack(): Promise<void> {
    guideVocalPath.set(null);
    guideVocalEnabled.set(false);
    await invoke("clear_guide_vocal").catch((err) =>
      console.warn("[guide] clear failed:", err),
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
    lastMelodyOffsetSecs = null;
    lastMelodyKey = null;
    melodyStatus.set({ key: "setup.melody.status.cleared" });
  }

  async function clearBackingTrack(): Promise<void> {
    try {
      await invoke("clear_backing");
    } catch (err) {
      console.warn("[backing] clear failed:", err);
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
    }).catch((err) => console.warn("[guide] offset sync failed:", err));
  }

  interface LoadBackingOptions {
    resetDependents?: boolean;
    autoDetectMelody?: boolean;
    probeSubtitles?: boolean;
  }

  async function loadBackingFromPath(
    path: string,
    options: LoadBackingOptions = {},
  ): Promise<void> {
    const resetDependents = options.resetDependents ?? true;
    const autoDetectMelody = options.autoDetectMelody ?? true;
    const probeSubtitles = options.probeSubtitles ?? true;
    pendingStatusText = tSync("setup.backing.hint.loading");

    if (resetDependents) {
      loadedMedia.set(null);
      resetBackingState();
      resetMelodyState();
      await clearGuideVocalTrack();
      clearLoop();
      hasRecording.set(false);
      clearLiveVocalSamples();
      embeddedSubtitles = [];
    }

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
    pendingStatusText = null;
    detectedMelodySourceKind.set(result.melody_source);

    embeddedSubtitles = [];
    if (probeSubtitles && result.is_video) {
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
        console.warn("[setup] embedded subtitle probe failed:", err);
      }
    }

    if (autoDetectMelody) {
      await autoLoadMelodyForPath(path);
    }
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

    try {
      await loadBackingFromPath(path);
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
        const sourceDescriptor = describeMelodySource(track);
        melodyStatus.set({
          key: "setup.melody.status.autoLoaded",
          vars: { n: track.notes.length },
          nestedVars: { source: sourceDescriptor },
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
      const sourceDescriptor = describeMelodySource(track);
      melodyStatus.set({
        key: "setup.melody.status.vocalsLoaded",
        vars: {
          n: track.raw_pitch_track?.length ?? track.notes.length,
        },
        nestedVars: { source: sourceDescriptor },
      });
      // 若練唱伴奏已載入，自動跑對齊
      if (currentBackingPath) {
        await runAutoAlignment(path, currentBackingPath);
      }
      try {
        await loadGuideVocalTrack(path);
        await syncGuideVocalTiming();
      } catch (guideErr) {
        guideVocalPath.set(null);
        guideVocalEnabled.set(false);
        melodyStatus.update((s) =>
          s
            ? {
                ...s,
                appendKey: "setup.guide.status.loadFailedAppend",
                appendVars: { error: String(guideErr) },
              }
            : s,
        );
      }
    } catch (err) {
      melodyStatus.set({
        key: "setup.melody.status.vocalsFailed",
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

  let lastGuideSyncKey = "";

  $effect(() => {
    const path = $guideVocalPath;
    const offsetSecs = finalOffsetSecs($alignmentResult, $alignmentFineTuneMs);
    const enabled = $guideVocalEnabled;
    const syncKey = `${path ?? ""}|${offsetSecs.toFixed(6)}|${enabled}`;
    if (syncKey === lastGuideSyncKey) return;
    lastGuideSyncKey = syncKey;

    if (!path) {
      void invoke("set_guide_vocal_enabled", { enabled: false }).catch(() => {});
      return;
    }
    void invoke("set_guide_vocal_offset", { offsetSecs }).catch((err) =>
      console.warn("[guide] offset sync failed:", err),
    );
    void invoke("set_guide_vocal_enabled", { enabled }).catch((err) =>
      console.warn("[guide] enabled sync failed:", err),
    );
  });

  /**
   * 回傳一個「待翻譯的 descriptor」，而非立刻翻譯後的字串。
   * 這樣 melodyStatus 存進 store 之後，使用者切換 locale 時
   * `melodyStatusText` 的 `$derived.by` 會重新翻譯這段，不會卡舊語言。
   */
  function describeMelodySource(track: MelodyTrack): TranslatableDescriptor {
    const src = track.source;
    if (src.type === "midi") {
      return {
        key: "setup.melody.source.midi",
        vars: { n: src.track_index + 1 },
      };
    }
    if (src.type === "imported_vocals") {
      const voiced = (src.voiced_ratio * 100).toFixed(0);
      return {
        key: "setup.melody.source.importedVocals",
        vars: { ratio: voiced },
      };
    }
    return {
      key: "setup.melody.source.separated",
      vars: { model: src.model },
    };
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
    <button class="section-header" onclick={() => toggleSection("download")}>
      <h2>{$t("setup.section.download")}</h2>
      <span class="chevron" class:open={sections.download}>▸</span>
    </button>
    {#if sections.download}
      <div class="section-body">
        <DownloadTab />
      </div>
    {/if}
  </div>

  <!-- 歌曲與音高來源 -->
  <!-- Song-specific material setup moved to SongLibraryTab. -->

  <div class="card">
    <button class="section-header" onclick={() => toggleSection("device")}>
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
        <div class="actions device-actions">
          <button class="btn secondary" onclick={openCrepeModelFolder}>
            {$t("setup.device.modelFolder.openCrepe")}
          </button>
        </div>
        {#if deviceMsg}
          <p class="sub-hint">{deviceMsg}</p>
        {/if}
      </div>
    {/if}
  </div>

  <!-- 延遲校準 -->
  <div class="card">
    <button class="section-header" onclick={() => toggleSection("calibration")}>
      <h2>{$t("setup.section.calibration")}</h2>
      <span class="chevron" class:open={sections.calibration}>▸</span>
    </button>
    {#if sections.calibration}
      <div class="section-body">
        <p class="hint">{$t("setup.calibration.hint")}</p>

        <div class="calibration-main">
          <div>
            <span class="calibration-value">{$latencyMs} ms</span>
            <p class="fine-tune-note">{$t("setup.calibration.system.hint")}</p>
          </div>
          <button
            class="btn primary calibrate-btn"
            onclick={startSystemCalibration}
            disabled={calibrationBusy}
          >
            {systemCalibrationBusy ? $t("setup.calibration.system.running") : $t("setup.calibration.system.action")}
          </button>
        </div>

        <button
          type="button"
          class="calibration-toggle"
          onclick={() => (manualCalibrationOpen = !manualCalibrationOpen)}
        >
          {$t("setup.calibration.manual.toggle")}
        </button>
        {#if manualCalibrationOpen}
          <LatencyFineTuneControl
            title={$t("setup.calibration.manual.toggle")}
            description={$t("setup.calibration.manual.description")}
            disabled={calibrationBusy}
          />
        {/if}

        <button
          type="button"
          class="calibration-toggle"
          onclick={() => (rhythmVoiceCalibrationOpen = !rhythmVoiceCalibrationOpen)}
        >
          {$t("setup.calibration.rhythmVoice.toggle")}
        </button>
        {#if rhythmVoiceCalibrationOpen}
          <div class="rhythm-calibration-panel">
            <p class="fine-tune-note">{$t("setup.calibration.rhythmVoice.hint")}</p>
            <button
              type="button"
              class="btn secondary rhythm-calibrate-btn"
              onclick={startRhythmVoiceCalibration}
              disabled={calibrationBusy}
            >
              {rhythmVoiceCalibrationBusy
                ? $t("setup.calibration.rhythmVoice.running")
                : $t("setup.calibration.rhythmVoice.action")}
            </button>
          </div>
        {/if}

        {#if calibrationResultText}
          <p class="calibration-result">{calibrationResultText}</p>
        {/if}
      </div>
    {/if}
  </div>
</div>

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

  .btn.primary:hover:not(:disabled) {
    background: #5c4400;
  }

  .btn.secondary {
    background: #f0ece4;
    color: #7a7268;
  }

  .btn.secondary:hover:not(:disabled) {
    background: #e6ded0;
    color: #3d3630;
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
  
  .calibration-main {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px;
    border: 1px solid #e8e2d8;
    border-radius: 8px;
    background: #fdfaf5;
  }

  .calibration-value {
    display: block;
    margin-bottom: 4px;
    color: #755700;
    font-size: 22px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .calibration-toggle {
    width: 100%;
    margin-top: 10px;
    padding: 10px 12px;
    border: 1px solid #e8e2d8;
    border-radius: 8px;
    background: #fff;
    color: #5c5248;
    font-size: 13px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }

  .calibration-toggle:hover {
    background: #faf8f4;
  }

  .rhythm-calibration-panel {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 8px;
    padding: 12px;
    border: 1px solid #e8e2d8;
    border-radius: 8px;
    background: #fff;
  }

  .rhythm-calibrate-btn {
    flex: 0 0 auto;
  }

  .calibration-result {
    margin: 12px 0 0 0;
    font-size: 13px;
    color: #d35400;
    font-weight: bold;
  }

  .fine-tune-note {
    margin: 10px 0 0 0;
    font-size: 12px;
    line-height: 1.45;
    color: #6b6258;
  }

  @media (max-width: 760px) {
    .calibration-main {
      align-items: stretch;
      flex-direction: column;
    }

    .rhythm-calibration-panel {
      align-items: stretch;
      flex-direction: column;
    }

  }


</style>
