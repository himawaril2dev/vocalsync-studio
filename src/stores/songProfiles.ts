import { invoke } from "@tauri-apps/api/core";
import { get, writable, type Readable } from "svelte/store";
import { lyricsFileName, lyricsLines } from "./lyrics";
import { loadedMedia } from "./media";
import {
  alignmentFineTuneMs,
  alignmentResult,
  currentMelody,
  guideVocalPath,
  melodySourcePath,
} from "./melody";
import {
  applySongSessionSnapshot,
  createSongSessionSnapshot,
  projectSessionReady,
  resetWorkspaceForNewSong,
  sanitizeSongSession,
} from "./projectSession";
import {
  autoBalanceMixin,
  autoBalanceVocalPreset,
  backingVolume,
  exportNamingMode,
  guideVocalEnabled,
  guideVolume,
  micGain,
} from "./settings";
import { loopA, loopB, pitchSemitones, speed } from "./transport";

export interface SongProfileStatus {
  backingSet: boolean;
  backingExists: boolean;
  guideVocalSet: boolean;
  guideVocalExists: boolean;
  lyricsCount: number;
  timedLyricsCount: number;
  melodyPresent: boolean;
}

export interface SongProfileSummary {
  id: string;
  title: string;
  artist: string | null;
  createdAtUnix: number;
  updatedAtUnix: number;
  lastOpenedAtUnix: number;
  status: SongProfileStatus;
}

interface SongProfilePayload {
  profile: SongProfileSummary;
  sessionJson: string;
}

interface PersistedActiveSongProfile {
  id: string;
  title: string;
  artist: string | null;
}

const ACTIVE_SONG_PROFILE_STORAGE_KEY = "vocalsync.activeSongProfile.v1";
const ACTIVE_SONG_PROFILE_AUTO_SAVE_MS = 800;

export const songProfiles = writable<SongProfileSummary[]>([]);
export const activeSongProfileId = writable<string | null>(null);
export const activeSongProfileTitle = writable<string>("");
export const activeSongProfileArtist = writable<string | null>(null);
export const songProfileDirty = writable<boolean>(false);
export const songProfileApplying = writable<boolean>(false);

let activeSongProfileAutoSaveTimer: number | null = null;
let activeSongProfileSaving = false;

function clearActiveSongProfileAutoSaveTimer(): void {
  if (activeSongProfileAutoSaveTimer === null || typeof window === "undefined") return;
  window.clearTimeout(activeSongProfileAutoSaveTimer);
  activeSongProfileAutoSaveTimer = null;
}

function scheduleActiveSongProfileAutoSave(): void {
  if (typeof window === "undefined") return;
  clearActiveSongProfileAutoSaveTimer();
  activeSongProfileAutoSaveTimer = window.setTimeout(() => {
    activeSongProfileAutoSaveTimer = null;
    void flushActiveSongProfileSave().catch((error) =>
      console.warn("[song-library] active profile auto-save failed:", error),
    );
  }, ACTIVE_SONG_PROFILE_AUTO_SAVE_MS);
}

function releaseApplyingSoon(): void {
  if (typeof window === "undefined") {
    songProfileApplying.set(false);
    return;
  }
  window.setTimeout(() => songProfileApplying.set(false), 0);
}

function readPersistedActiveProfile(): PersistedActiveSongProfile | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(ACTIVE_SONG_PROFILE_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PersistedActiveSongProfile>;
    if (typeof parsed.id !== "string" || typeof parsed.title !== "string") return null;
    return {
      id: parsed.id,
      title: parsed.title,
      artist: typeof parsed.artist === "string" ? parsed.artist : null,
    };
  } catch {
    return null;
  }
}

function persistActiveProfile(profile: PersistedActiveSongProfile): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(ACTIVE_SONG_PROFILE_STORAGE_KEY, JSON.stringify(profile));
}

function clearPersistedActiveProfile(): void {
  if (typeof localStorage === "undefined") return;
  localStorage.removeItem(ACTIVE_SONG_PROFILE_STORAGE_KEY);
}

function setActiveProfile(profile: PersistedActiveSongProfile): void {
  activeSongProfileId.set(profile.id);
  activeSongProfileTitle.set(profile.title);
  activeSongProfileArtist.set(profile.artist);
  persistActiveProfile(profile);
  songProfileDirty.set(false);
}

function clearActiveProfile(): void {
  activeSongProfileId.set(null);
  activeSongProfileTitle.set("");
  activeSongProfileArtist.set(null);
  clearPersistedActiveProfile();
  songProfileDirty.set(false);
}

function restoreActiveProfileFromStorage(): void {
  if (get(activeSongProfileId)) return;
  const persisted = readPersistedActiveProfile();
  if (persisted) {
    setActiveProfile(persisted);
  }
}

export function markActiveSongProfileDirty(): void {
  if (
    get(projectSessionReady) &&
    get(activeSongProfileId) &&
    !get(songProfileApplying) &&
    !activeSongProfileSaving
  ) {
    songProfileDirty.set(true);
    scheduleActiveSongProfileAutoSave();
  }
}

function trackActiveSongProfileDirty(stores: Readable<unknown>[]): void {
  for (const store of stores) {
    let initialized = false;
    store.subscribe(() => {
      if (!initialized) {
        initialized = true;
        return;
      }
      markActiveSongProfileDirty();
    });
  }
}

trackActiveSongProfileDirty([
  loadedMedia,
  lyricsFileName,
  lyricsLines,
  currentMelody,
  melodySourcePath,
  guideVocalPath,
  guideVocalEnabled,
  alignmentResult,
  alignmentFineTuneMs,
  backingVolume,
  micGain,
  guideVolume,
  autoBalanceMixin,
  autoBalanceVocalPreset,
  exportNamingMode,
  loopA,
  loopB,
  speed,
  pitchSemitones,
]);

restoreActiveProfileFromStorage();

export async function refreshSongProfiles(): Promise<SongProfileSummary[]> {
  const profiles = await invoke<SongProfileSummary[]>("list_song_profiles");
  songProfiles.set(profiles);
  const activeId = get(activeSongProfileId);
  if (activeId && !profiles.some((profile) => profile.id === activeId)) {
    clearActiveProfile();
  }
  return profiles;
}

export async function loadSongProfile(profileId: string): Promise<SongProfilePayload> {
  songProfileApplying.set(true);
  try {
    const payload = await invoke<SongProfilePayload>("load_song_profile", { profileId });
    const parsed = JSON.parse(payload.sessionJson);
    const session = sanitizeSongSession(parsed);
    if (!session) {
      throw new Error("Song profile session is invalid");
    }
    await applySongSessionSnapshot(session);
    setActiveProfile(payload.profile);
    await refreshSongProfiles();
    return payload;
  } finally {
    releaseApplyingSoon();
  }
}

export async function saveCurrentSongProfile(
  profileId: string | null,
  title: string,
  artist: string | null,
): Promise<SongProfileSummary> {
  const profile = await invoke<SongProfileSummary>("save_song_profile", {
    profileId,
    title,
    artist,
    sessionJson: JSON.stringify(createSongSessionSnapshot()),
  });
  setActiveProfile(profile);
  await refreshSongProfiles();
  return profile;
}

export async function updateActiveSongProfile(): Promise<SongProfileSummary> {
  const profileId = get(activeSongProfileId);
  const title = get(activeSongProfileTitle);
  if (!profileId || !title.trim()) {
    throw new Error("No active song profile is selected");
  }
  return saveCurrentSongProfile(profileId, title, get(activeSongProfileArtist));
}

export async function flushActiveSongProfileSave(): Promise<SongProfileSummary | null> {
  clearActiveSongProfileAutoSaveTimer();
  if (!get(projectSessionReady) || !get(activeSongProfileId) || !get(songProfileDirty)) {
    return null;
  }
  activeSongProfileSaving = true;
  try {
    return await updateActiveSongProfile();
  } finally {
    activeSongProfileSaving = false;
  }
}

/**
 * 開始一首新歌曲：先把啟用中歌曲的最新狀態存回歌單，
 * 再解除啟用並清空工作區，讓之後的匯入與調整不會覆寫舊歌的存檔。
 */
export async function startNewSong(): Promise<void> {
  try {
    await flushActiveSongProfileSave();
  } catch (error) {
    console.warn("[song-library] flush before new song failed:", error);
  }
  clearActiveSongProfileAutoSaveTimer();
  clearActiveProfile();
  await resetWorkspaceForNewSong();
}

export async function renameSongProfile(
  profileId: string,
  title: string,
  artist: string | null,
): Promise<SongProfileSummary> {
  const profile = await invoke<SongProfileSummary>("rename_song_profile", {
    profileId,
    title,
    artist,
  });
  if (get(activeSongProfileId) === profileId) {
    setActiveProfile(profile);
  }
  await refreshSongProfiles();
  return profile;
}

export async function deleteSongProfile(profileId: string): Promise<void> {
  await invoke("delete_song_profile", { profileId });
  if (get(activeSongProfileId) === profileId) {
    clearActiveProfile();
  }
  await refreshSongProfiles();
}
