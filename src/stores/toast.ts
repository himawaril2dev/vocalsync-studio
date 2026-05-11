import { writable } from "svelte/store";

export type ToastLevel = "info" | "success" | "warning" | "error";

export interface Toast {
  id: number;
  message: string;
  level: ToastLevel;
}

let nextId = 0;
const timerMap = new Map<number, ReturnType<typeof setTimeout>>();

export const toasts = writable<Toast[]>([]);

/**
 * 顯示一則 toast 訊息，預設 3 秒後自動消失。
 */
export function showToast(
  message: string,
  level: ToastLevel = "info",
  durationMs: number = 3000,
): void {
  const id = nextId++;
  toasts.update((list) => [...list, { id, message, level }]);

  if (durationMs > 0) {
    const timer = setTimeout(() => dismissToast(id), durationMs);
    timerMap.set(id, timer);
  }
}

export function dismissToast(id: number): void {
  const timer = timerMap.get(id);
  if (timer) {
    clearTimeout(timer);
    timerMap.delete(id);
  }
  toasts.update((list) => list.filter((t) => t.id !== id));
}
