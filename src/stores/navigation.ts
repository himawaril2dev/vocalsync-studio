import { writable } from "svelte/store";

export type AppTab = "setup" | "songs" | "recording" | "pitch" | "about";

export const activeTab = writable<AppTab>("setup");
