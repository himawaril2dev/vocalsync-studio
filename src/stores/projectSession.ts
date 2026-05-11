import { writable } from "svelte/store";

export const projectSessionReady = writable<boolean>(false);
