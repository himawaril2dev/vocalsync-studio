import { writable } from "svelte/store";

export interface LoadedMediaInfo {
  /** 原始檔案絕對路徑（用於後續操作如重新掃描 melody） */
  file_path: string;
  /** 檔名（不含路徑），用於 UI 顯示 */
  file_name: string;
  duration: number;
  sample_rate: number;
  is_video: boolean;
  video_path: string | null;
  /** WebView 可存取的 URL（透過 convertFileSrc 轉換）*/
  video_url: string | null;
}

export const loadedMedia = writable<LoadedMediaInfo | null>(null);

/** 從絕對路徑取出檔名（不含路徑），跨平台處理 \ 與 /。 */
export function basename(path: string): string {
  const idx = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return idx >= 0 ? path.slice(idx + 1) : path;
}
