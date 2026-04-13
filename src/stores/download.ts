import { writable, derived } from "svelte/store";

// ── 型別定義 ──────────────────────────────────────────────────

export interface ToolStatus {
  ytdlp_available: boolean;
  ytdlp_version: string | null;
  ytdlp_path: string | null;
  ffmpeg_available: boolean;
  ffmpeg_version: string | null;
}

export interface InstallProgress {
  percent: number;
  status: "downloading" | "finished" | "error";
  message: string;
}

export interface DownloadProgress {
  percent: number;
  filename: string;
  status: "downloading" | "postprocessing" | "finished" | "error";
  downloaded: string;
  total: string;
  speed: string;
  eta: string;
}

export interface DownloadResult {
  success: boolean;
  message: string;
  output_dir: string;
  /** 下載完成後在 output_dir 中找到的字幕檔案路徑 */
  subtitle_paths: string[];
}

export type DownloadFormat = "video" | "mp3" | "m4a" | "wav" | "subtitle_only";
export type VideoQuality = "best" | "1080p" | "720p" | "480p" | "360p";
export type SubtitleLang =
  | "traditional_chinese"
  | "simplified_chinese"
  | "english"
  | "japanese"
  | "all"
  | "none";
export type UrlType = "video" | "playlist" | "channel";
export type DownloadStatus = "idle" | "downloading" | "postprocessing" | "finished" | "error" | "cancelled";

// ── Stores ──────────────────────────────────────────────────────

/** 工具安裝狀態 */
export const toolStatus = writable<ToolStatus | null>(null);

/** 目前下載進度 */
export const downloadProgress = writable<DownloadProgress | null>(null);

/** 下載狀態 */
export const downloadStatus = writable<DownloadStatus>("idle");

/** 最後一次下載結果 */
export const lastResult = writable<DownloadResult | null>(null);

/** 偵測到的 URL 類型 */
export const detectedUrlType = writable<UrlType | null>(null);

/** 是否正在下載中 */
export const isDownloading = derived(downloadStatus, ($s) =>
  $s === "downloading" || $s === "postprocessing"
);

/** yt-dlp 安裝進度 */
export const installProgress = writable<InstallProgress | null>(null);

/** 是否正在安裝 yt-dlp */
export const isInstalling = writable(false);

/** FFmpeg 安裝進度 */
export const ffmpegInstallProgress = writable<InstallProgress | null>(null);

/** 是否正在安裝 FFmpeg */
export const isInstallingFfmpeg = writable(false);

// ── Reset ───────────────────────────────────────────────────────

export function resetDownloadState(): void {
  downloadProgress.set(null);
  downloadStatus.set("idle");
  lastResult.set(null);
  detectedUrlType.set(null);
}
