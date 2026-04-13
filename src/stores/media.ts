import { writable } from "svelte/store";

export interface LoadedMediaInfo {
  duration: number;
  sample_rate: number;
  is_video: boolean;
  video_path: string | null;
  /** WebView 可存取的 URL（透過 convertFileSrc 轉換）*/
  video_url: string | null;
}

export const loadedMedia = writable<LoadedMediaInfo | null>(null);
