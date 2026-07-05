<script lang="ts">
  import { onMount } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { t, tSync } from "../i18n";
  import { hasRecording, isTransportRunning } from "../stores/transport";
  import { loadedMedia } from "../stores/media";
  import SongMaterialSetup from "../components/SongMaterialSetup.svelte";
  import { showToast } from "../stores/toast";
  import {
    activeSongProfileId,
    activeSongProfileTitle,
    deleteSongProfile,
    loadSongProfile,
    refreshSongProfiles,
    renameSongProfile,
    saveCurrentSongProfile,
    songProfileDirty,
    songProfiles,
    startNewSong,
    updateActiveSongProfile,
    type SongProfileSummary,
  } from "../stores/songProfiles";

  let searchText = $state("");
  let newTitle = $state("");
  let newArtist = $state("");
  let busyId = $state<string | null>(null);
  let creating = $state(false);
  let updating = $state(false);
  let editingId = $state<string | null>(null);
  let editTitle = $state("");
  let editArtist = $state("");
  let startingNew = $state(false);

  let filteredProfiles = $derived.by(() => {
    const query = searchText.trim().toLowerCase();
    if (!query) return $songProfiles;
    return $songProfiles.filter((profile) => {
      const haystack = `${profile.title} ${profile.artist ?? ""}`.toLowerCase();
      return haystack.includes(query);
    });
  });

  let suggestedTitle = $derived($loadedMedia?.file_name.replace(/\.[^.]+$/, "").trim() ?? "");

  onMount(() => {
    void refreshSongProfiles().catch((error) =>
      showToast(tSync("songLibrary.toast.loadFailed", { error: String(error) }), "error"),
    );
  });

  $effect(() => {
    const suggestion = suggestedTitle;
    if (!newTitle.trim() && suggestion) {
      newTitle = suggestion;
    }
  });

  function formatDate(unix: number): string {
    if (!unix) return tSync("songLibrary.date.never");
    return new Date(unix * 1000).toLocaleString();
  }

  function backingStatus(profile: SongProfileSummary): string {
    if (profile.status.backingSet && profile.status.backingExists) {
      return tSync("songLibrary.status.backing.ok");
    }
    if (profile.status.backingSet) return tSync("songLibrary.status.backing.missing");
    return tSync("songLibrary.status.backing.empty");
  }

  function guideStatus(profile: SongProfileSummary): string {
    if (profile.status.guideVocalSet && profile.status.guideVocalExists) {
      return tSync("songLibrary.status.guide.ok");
    }
    if (profile.status.guideVocalSet) return tSync("songLibrary.status.guide.missing");
    return tSync("songLibrary.status.guide.empty");
  }

  function lyricsStatus(profile: SongProfileSummary): string {
    if (profile.status.lyricsCount <= 0) return tSync("songLibrary.status.lyrics.empty");
    return tSync("songLibrary.status.lyrics.count", {
      count: profile.status.lyricsCount,
      timed: profile.status.timedLyricsCount,
    });
  }

  function melodyStatus(profile: SongProfileSummary): string {
    return profile.status.melodyPresent
      ? tSync("songLibrary.status.melody.ok")
      : tSync("songLibrary.status.melody.empty");
  }

  async function createProfile(): Promise<void> {
    if (creating || !newTitle.trim()) return;
    creating = true;
    try {
      const profile = await saveCurrentSongProfile(
        null,
        newTitle.trim(),
        newArtist.trim() || null,
      );
      newTitle = "";
      newArtist = "";
      showToast(tSync("songLibrary.toast.saved", { title: profile.title }), "success");
    } catch (error) {
      showToast(tSync("songLibrary.toast.saveFailed", { error: String(error) }), "error");
    } finally {
      creating = false;
    }
  }

  async function updateProfile(): Promise<void> {
    if (updating || !$activeSongProfileId) return;
    updating = true;
    try {
      const profile = await updateActiveSongProfile();
      showToast(tSync("songLibrary.toast.updated", { title: profile.title }), "success");
    } catch (error) {
      showToast(tSync("songLibrary.toast.updateFailed", { error: String(error) }), "error");
    } finally {
      updating = false;
    }
  }

  async function startNewSongFlow(): Promise<void> {
    if ($isTransportRunning || startingNew || busyId) return;
    const confirmed = await ask(tSync("songLibrary.startNew.message"), {
      title: tSync("songLibrary.startNew.title"),
      kind: "warning",
    });
    if (!confirmed) return;
    startingNew = true;
    try {
      await startNewSong();
      newTitle = "";
      newArtist = "";
      showToast(tSync("songLibrary.startNew.done"), "success");
    } catch (error) {
      showToast(tSync("songLibrary.startNew.failed", { error: String(error) }), "error");
    } finally {
      startingNew = false;
    }
  }

  async function loadProfile(profile: SongProfileSummary): Promise<void> {
    if ($isTransportRunning || busyId) return;
    if ($hasRecording) {
      const confirmed = await ask(
        tSync("songLibrary.load.confirmMessage", { title: profile.title }),
        { title: tSync("songLibrary.load.confirmTitle"), kind: "warning" },
      );
      if (!confirmed) return;
    }
    busyId = profile.id;
    try {
      const payload = await loadSongProfile(profile.id);
      showToast(
        tSync("songLibrary.toast.loaded", { title: payload.profile.title }),
        "success",
      );
    } catch (error) {
      showToast(tSync("songLibrary.toast.loadOneFailed", { error: String(error) }), "error");
    } finally {
      busyId = null;
    }
  }

  function beginRename(profile: SongProfileSummary): void {
    editingId = profile.id;
    editTitle = profile.title;
    editArtist = profile.artist ?? "";
  }

  async function commitRename(profile: SongProfileSummary): Promise<void> {
    if (!editingId || !editTitle.trim()) return;
    busyId = profile.id;
    try {
      const renamed = await renameSongProfile(
        profile.id,
        editTitle.trim(),
        editArtist.trim() || null,
      );
      editingId = null;
      showToast(tSync("songLibrary.toast.renamed", { title: renamed.title }), "success");
    } catch (error) {
      showToast(tSync("songLibrary.toast.renameFailed", { error: String(error) }), "error");
    } finally {
      busyId = null;
    }
  }

  async function removeProfile(profile: SongProfileSummary): Promise<void> {
    if (busyId) return;
    const confirmed = await ask(
      tSync("songLibrary.delete.message", { title: profile.title }),
      { title: tSync("songLibrary.delete.title"), kind: "warning" },
    );
    if (!confirmed) return;

    busyId = profile.id;
    try {
      await deleteSongProfile(profile.id);
      showToast(tSync("songLibrary.toast.deleted", { title: profile.title }), "success");
    } catch (error) {
      showToast(tSync("songLibrary.toast.deleteFailed", { error: String(error) }), "error");
    } finally {
      busyId = null;
    }
  }
</script>

<div class="song-library-page">
  <header class="song-library-header">
    <div>
      <h1>{$t("songLibrary.title")}</h1>
      <p>{$t("songLibrary.subtitle")}</p>
    </div>
    <div class="current-song-bar">
      <div class="current-song-info">
        <span class="current-label">{$t("songLibrary.current.label")}</span>
        <strong class="current-title">
          {$activeSongProfileTitle || $t("songLibrary.current.none")}
        </strong>
        {#if $activeSongProfileId}
          {#if $songProfileDirty}
            <span class="state-badge dirty">{$t("songLibrary.current.dirty")}</span>
          {:else}
            <span class="state-badge saved">{$t("songLibrary.current.saved")}</span>
          {/if}
        {/if}
      </div>
      <div class="header-actions">
        <button
          class="secondary-action"
          onclick={startNewSongFlow}
          disabled={startingNew || $isTransportRunning}
        >
          {$t("songLibrary.action.startNew")}
        </button>
        {#if $activeSongProfileId}
          <button
            class="primary-action"
            onclick={updateProfile}
            disabled={updating || !$songProfileDirty}
          >
            {$t($songProfileDirty ? "songLibrary.action.saveActive" : "songLibrary.action.saved")}
          </button>
        {/if}
      </div>
    </div>
  </header>

  <SongMaterialSetup />

  <section class="create-panel">
    <div class="create-copy">
      <strong>{$t("songLibrary.create.title")}</strong>
      <span>{$t("songLibrary.create.hint")}</span>
    </div>
    <input
      type="text"
      bind:value={newTitle}
      placeholder={$t("songLibrary.create.titlePlaceholder")}
      aria-label={$t("songLibrary.create.titlePlaceholder")}
    />
    <input
      type="text"
      bind:value={newArtist}
      placeholder={$t("songLibrary.create.artistPlaceholder")}
      aria-label={$t("songLibrary.create.artistPlaceholder")}
    />
    <button
      class="primary-action"
      onclick={createProfile}
      disabled={creating || !newTitle.trim() || !$loadedMedia}
      title={$loadedMedia ? "" : $t("songLibrary.create.disabledNoBacking")}
    >
      {$t("songLibrary.action.saveNew")}
    </button>
  </section>

  <section class="library-toolbar">
    <input
      type="search"
      bind:value={searchText}
      placeholder={$t("songLibrary.search.placeholder")}
      aria-label={$t("songLibrary.search.placeholder")}
    />
    <span>{$t("songLibrary.count", { count: filteredProfiles.length })}</span>
  </section>

  {#if filteredProfiles.length === 0}
    <section class="empty-state">
      <h2>{$t("songLibrary.empty.title")}</h2>
      <p>{$t("songLibrary.empty.body")}</p>
    </section>
  {:else}
    <section class="song-grid">
      {#each filteredProfiles as profile}
        <article
          class="song-card"
          class:active={profile.id === $activeSongProfileId}
        >
          {#if editingId === profile.id}
            <div class="rename-fields">
              <input
                type="text"
                bind:value={editTitle}
                aria-label={$t("songLibrary.rename.titleLabel")}
              />
              <input
                type="text"
                bind:value={editArtist}
                aria-label={$t("songLibrary.rename.artistLabel")}
              />
            </div>
          {:else}
            <div class="song-title-row">
              <div>
                <h2>{profile.title}</h2>
                <p>{profile.artist ?? $t("songLibrary.artist.empty")}</p>
              </div>
              {#if profile.id === $activeSongProfileId}
                <span class="active-badge">{$t("songLibrary.badge.active")}</span>
              {/if}
            </div>
          {/if}

          <div class="status-grid">
            <span class:missing={profile.status.backingSet && !profile.status.backingExists}>
              {backingStatus(profile)}
            </span>
            <span>{lyricsStatus(profile)}</span>
            <span>{melodyStatus(profile)}</span>
            <span class:missing={profile.status.guideVocalSet && !profile.status.guideVocalExists}>
              {guideStatus(profile)}
            </span>
          </div>

          <p class="updated-at">
            {$t("songLibrary.updatedAt", { time: formatDate(profile.updatedAtUnix) })}
          </p>

          <div class="song-actions">
            {#if editingId === profile.id}
              <button
                class="primary-action small"
                onclick={() => commitRename(profile)}
                disabled={busyId === profile.id || !editTitle.trim()}
              >
                {$t("songLibrary.action.renameSave")}
              </button>
              <button class="secondary-action small" onclick={() => (editingId = null)}>
                {$t("songLibrary.action.cancel")}
              </button>
            {:else}
              {#if profile.id === $activeSongProfileId}
                <button
                  class="primary-action small"
                  onclick={updateProfile}
                  disabled={updating || !$songProfileDirty}
                >
                  {$t($songProfileDirty ? "songLibrary.action.saveActive" : "songLibrary.action.saved")}
                </button>
              {:else}
                <button
                  class="primary-action small"
                  onclick={() => loadProfile(profile)}
                  disabled={$isTransportRunning || busyId === profile.id}
                >
                  {$t("songLibrary.action.load")}
                </button>
              {/if}
              <button class="secondary-action small" onclick={() => beginRename(profile)}>
                {$t("songLibrary.action.rename")}
              </button>
              <button
                class="danger-action small"
                onclick={() => removeProfile(profile)}
                disabled={busyId === profile.id}
              >
                {$t("songLibrary.action.delete")}
              </button>
            {/if}
          </div>
        </article>
      {/each}
    </section>
  {/if}
</div>

<style>
  .song-library-page {
    height: 100%;
    overflow-y: auto;
    padding: var(--space-xl);
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .song-library-header,
  .create-panel,
  .library-toolbar {
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
  }

  .song-library-header {
    padding: var(--space-xl);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-lg);
  }

  .current-song-bar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: var(--space-md) var(--space-lg);
    min-width: 0;
  }

  .current-song-info {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }

  .current-label {
    flex-shrink: 0;
    color: var(--color-text-secondary);
    font-size: 0.85rem;
  }

  .current-title {
    color: var(--color-text);
    font-size: 1rem;
    max-width: 340px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state-badge {
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    font-size: 0.75rem;
    font-weight: 800;
    padding: 3px 8px;
  }

  .state-badge.dirty {
    background: var(--color-warning-bg);
    color: var(--color-warning-text);
  }

  .state-badge.saved {
    background: rgba(117, 87, 0, 0.12);
    color: var(--color-brand);
  }

  .header-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    align-items: center;
  }

  h1,
  h2,
  p {
    margin: 0;
  }

  h1 {
    font-size: 1.6rem;
    color: var(--color-text);
  }

  .song-library-header p,
  .create-copy span,
  .updated-at,
  .song-title-row p,
  .library-toolbar span {
    color: var(--color-text-secondary);
    font-size: 0.9rem;
  }

  .create-panel {
    padding: var(--space-lg);
    display: grid;
    grid-template-columns: minmax(180px, 1.1fr) minmax(180px, 1fr) minmax(140px, 0.8fr) auto;
    gap: var(--space-md);
    align-items: center;
  }

  .create-copy {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  input[type="text"],
  input[type="search"] {
    width: 100%;
    min-width: 0;
    height: 38px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-bg);
    color: var(--color-text);
    padding: 0 var(--space-md);
    font: inherit;
  }

  .library-toolbar {
    padding: var(--space-md) var(--space-lg);
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }

  .song-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-md);
  }

  .song-card,
  .empty-state {
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: var(--space-lg);
  }

  .song-card.active {
    border-color: var(--color-brand);
    box-shadow: 0 0 0 1px var(--color-brand);
  }

  .song-title-row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-md);
    align-items: flex-start;
  }

  .song-title-row h2 {
    color: var(--color-text);
    font-size: 1.05rem;
    overflow-wrap: anywhere;
  }

  .active-badge {
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: rgba(117, 87, 0, 0.12);
    color: var(--color-brand);
    font-size: 0.75rem;
    font-weight: 800;
    padding: 3px 7px;
  }

  .status-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-xs);
    margin: var(--space-md) 0;
  }

  .status-grid span {
    border-radius: var(--radius-sm);
    background: var(--color-bg);
    color: var(--color-text-secondary);
    font-size: 0.8rem;
    padding: 6px 8px;
    overflow-wrap: anywhere;
  }

  .status-grid span.missing {
    background: rgba(239, 68, 68, 0.1);
    color: var(--color-danger);
  }

  .rename-fields {
    display: grid;
    gap: var(--space-sm);
  }

  .song-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    margin-top: var(--space-md);
  }

  .primary-action,
  .secondary-action,
  .danger-action {
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    height: 38px;
    padding: 0 var(--space-lg);
    font-weight: 800;
    cursor: pointer;
    white-space: nowrap;
  }

  .primary-action {
    background: var(--color-brand);
    color: #fff;
  }

  .secondary-action {
    background: var(--color-bg);
    border-color: var(--color-border);
    color: var(--color-text);
  }

  .danger-action {
    background: rgba(239, 68, 68, 0.1);
    color: var(--color-danger);
  }

  .small {
    height: 32px;
    padding: 0 var(--space-md);
    font-size: 0.85rem;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .empty-state {
    text-align: center;
    color: var(--color-text-secondary);
  }

  .empty-state h2 {
    color: var(--color-text);
    margin-bottom: var(--space-sm);
  }

  @media (max-width: 960px) {
    .song-library-header,
    .library-toolbar {
      align-items: stretch;
      flex-direction: column;
    }

    .create-panel {
      grid-template-columns: 1fr;
    }
  }
</style>
