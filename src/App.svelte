<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import SetupTab from "./tabs/SetupTab.svelte";
  import RecordingTab from "./tabs/RecordingTab.svelte";
  import PitchAnalysisTab from "./tabs/PitchAnalysisTab.svelte";
  import AboutTab from "./tabs/AboutTab.svelte";
  import {
    setupEventListeners,
    teardownEventListeners,
  } from "./lib/events";
  import ToastContainer from "./components/ToastContainer.svelte";
  import LanguageSwitcher from "./components/LanguageSwitcher.svelte";
  import { t } from "./i18n";

  let activeTab = $state<"setup" | "recording" | "pitch" | "about">("setup");
  let boundaryError = $state<Error | null>(null);
  let isMaximized = $state(false);

  const appWindow = getCurrentWindow();

  async function updateMaximized() {
    isMaximized = await appWindow.isMaximized();
  }

  function startDrag(e: MouseEvent) {
    if ((e.target as HTMLElement).closest(".titlebar-buttons")) return;
    appWindow.startDragging();
  }

  function handleTitlebarDblClick(e: MouseEvent) {
    if ((e.target as HTMLElement).closest(".titlebar-buttons")) return;
    appWindow.toggleMaximize();
  }

  let unlistenResize: (() => void) | null = null;

  onMount(async () => {
    setupEventListeners();
    updateMaximized();
    unlistenResize = await appWindow.onResized(updateMaximized);
  });

  onDestroy(() => {
    teardownEventListeners();
    unlistenResize?.();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="titlebar" onmousedown={startDrag} ondblclick={handleTitlebarDblClick}>
  <span class="titlebar-title">VocalSync Studio</span>
  <div class="titlebar-buttons">
    <button class="titlebar-btn" onclick={() => appWindow.minimize()} aria-label={$t("app.titlebar.minimize")}>
      <svg width="10" height="1" viewBox="0 0 10 1"><rect width="10" height="1" fill="currentColor"/></svg>
    </button>
    <button class="titlebar-btn" onclick={() => appWindow.toggleMaximize()} aria-label={$t("app.titlebar.maximize")}>
      {#if isMaximized}
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1">
          <rect x="2" y="0" width="8" height="8" rx="1"/>
          <rect x="0" y="2" width="8" height="8" rx="1" fill="var(--color-bg)"/>
        </svg>
      {:else}
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1">
          <rect x="0.5" y="0.5" width="9" height="9" rx="1"/>
        </svg>
      {/if}
    </button>
    <button class="titlebar-btn close-btn" onclick={() => appWindow.close()} aria-label={$t("app.titlebar.close")}>
      <svg width="10" height="10" viewBox="0 0 10 10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round">
        <line x1="1" y1="1" x2="9" y2="9"/><line x1="9" y1="1" x2="1" y2="9"/>
      </svg>
    </button>
  </div>
</div>

<div class="app-layout">
  <!-- 左側導覽列 -->
  <aside class="sidebar">
    <div class="sidebar-brand">
      <span class="brand-name">VocalSync</span>
      <span class="brand-sub">{$t("app.brand.sub")}</span>
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
    <nav class="sidebar-nav" role="tablist" aria-label={$t("app.nav.aria")}>
      <button
        class="nav-btn"
        class:active={activeTab === "setup"}
        onclick={() => (activeTab = "setup")}
        role="tab"
        aria-selected={activeTab === "setup"}
        tabindex={activeTab === "setup" ? 0 : -1}
      >
        {$t("app.nav.setup")}
      </button>
      <button
        class="nav-btn"
        class:active={activeTab === "recording"}
        onclick={() => (activeTab = "recording")}
        role="tab"
        aria-selected={activeTab === "recording"}
        tabindex={activeTab === "recording" ? 0 : -1}
      >
        {$t("app.nav.recording")}
      </button>
      <button
        class="nav-btn"
        class:active={activeTab === "pitch"}
        onclick={() => (activeTab = "pitch")}
        role="tab"
        aria-selected={activeTab === "pitch"}
        tabindex={activeTab === "pitch" ? 0 : -1}
      >
        {$t("app.nav.pitch")}
      </button>

      <div class="nav-spacer"></div>

      <button
        class="nav-btn about-btn"
        class:active={activeTab === "about"}
        onclick={() => (activeTab = "about")}
        role="tab"
        aria-selected={activeTab === "about"}
        tabindex={activeTab === "about" ? 0 : -1}
      >
        {$t("app.nav.about")}
      </button>
    </nav>

    <LanguageSwitcher />
  </aside>

  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
  <main class="main-content" role="tabpanel">
    <svelte:boundary onerror={(e: unknown) => { boundaryError = e instanceof Error ? e : new Error(String(e)); console.error('[App] 未預期的錯誤:', e); }}>
      {#if activeTab === "setup"}
        <SetupTab />
      {:else if activeTab === "recording"}
        <RecordingTab />
      {:else if activeTab === "pitch"}
        <PitchAnalysisTab />
      {:else}
        <AboutTab />
      {/if}
      {#snippet failed()}
        <div class="error-boundary">
          <h2>{$t("app.error.title")}</h2>
          <p class="error-msg">{boundaryError?.message ?? $t("app.error.unknown")}</p>
          <button class="error-retry-btn" onclick={() => { boundaryError = null; }}>
            {$t("app.error.retry")}
          </button>
        </div>
      {/snippet}
    </svelte:boundary>
  </main>
</div>

<ToastContainer />

<style>
  :global(:root) {
    /* ── 色彩 Token ── */
    --color-brand: #755700;
    --color-brand-hover: #8a6800;
    --color-brand-dark: #5c4400;
    --color-accent: #fdc003;
    --color-accent-hover: #ecb200;

    --color-text: #3d3630;
    --color-text-secondary: #7a7268;
    --color-text-muted: #b0a898;
    --color-text-faint: #d0ccc4;

    --color-bg: #f5f2ed;
    --color-bg-surface: #fff;
    --color-bg-sidebar: #faf8f4;
    --color-bg-hover: #f0ece4;
    --color-bg-active: #e8e2d8;

    --color-border: #e8e2d8;
    --color-border-light: #d0ccc4;

    --color-danger: #c0392b;
    --color-danger-hover: #a33025;
    --color-success: #00c853;
    --color-info: #2563eb;
    --color-info-bg: #e0edff;
    --color-warning-bg: #fef3c7;
    --color-warning-text: #92400e;

    /* ── 間距 Token ── */
    --space-xs: 4px;
    --space-sm: 8px;
    --space-md: 12px;
    --space-lg: 16px;
    --space-xl: 20px;
    --space-2xl: 24px;

    /* ── 圓角 Token ── */
    --radius-sm: 6px;
    --radius-md: 8px;
    --radius-lg: 10px;
    --radius-xl: 12px;

    /* ── 動態效果 Token ── */
    --transition-fast: 0.15s ease;
    --transition-normal: 0.2s ease;

    /* ── 字型 Token ── */
    --font-mono: "Consolas", "Fira Code", monospace;
  }

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: "Noto Sans TC", system-ui, sans-serif;
    background: var(--color-bg);
    color: var(--color-text);
    overflow: hidden;
    height: 100vh;
  }

  :global(*) {
    box-sizing: border-box;
  }

  :global(:focus-visible) {
    outline: 2px solid var(--color-brand);
    outline-offset: 2px;
  }

  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 32px;
    background: var(--color-bg);
    user-select: none;
    -webkit-user-select: none;
    padding-left: var(--space-lg);
    flex-shrink: 0;
  }

  .titlebar-title {
    font-size: 12px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .titlebar-buttons {
    display: flex;
    height: 100%;
  }

  .titlebar-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .titlebar-btn:hover {
    background: var(--color-bg-hover);
  }

  .close-btn:hover {
    background: var(--color-danger);
    color: #fff;
  }

  .app-layout {
    display: flex;
    height: calc(100vh - 32px);
  }

  .sidebar {
    width: 180px;
    min-width: 180px;
    background: var(--color-bg-sidebar);
    display: flex;
    flex-direction: column;
    padding: var(--space-xl) 0;
    border-right: 1px solid var(--color-border);
  }

  .sidebar-brand {
    padding: 0 var(--space-xl) var(--space-2xl);
    display: flex;
    flex-direction: column;
  }

  .brand-name {
    font-size: 20px;
    font-weight: 700;
    color: var(--color-brand);
  }

  .brand-sub {
    font-size: 10px;
    font-weight: 400;
    color: var(--color-text-muted);
    letter-spacing: 1.5px;
    margin-top: 2px;
  }

  .sidebar-nav {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: 0 10px;
    flex: 1;
  }

  .nav-spacer {
    flex: 1;
  }

  .about-btn {
    font-size: 13px;
    color: var(--color-text-muted);
  }

  .nav-btn {
    display: flex;
    align-items: center;
    width: 100%;
    padding: 10px var(--space-md);
    border: none;
    background: transparent;
    border-radius: var(--radius-md);
    font-size: 15px;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: all var(--transition-normal);
    text-align: left;
    position: relative;
  }

  .nav-btn:hover {
    background: var(--color-bg-hover);
    color: var(--color-text);
  }

  .nav-btn.active {
    color: var(--color-brand);
    font-weight: 600;
    background: transparent;
  }

  .nav-btn.active::after {
    content: "";
    position: absolute;
    right: 0;
    top: 50%;
    transform: translateY(-50%);
    width: 4px;
    height: 24px;
    background: var(--color-accent);
    border-radius: 2px;
  }

  .main-content {
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }

  .error-boundary {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: var(--space-2xl);
    text-align: center;
    gap: var(--space-md);
  }

  .error-boundary h2 {
    font-size: 18px;
    color: var(--color-danger);
    margin: 0;
  }

  .error-msg {
    font-size: 13px;
    color: var(--color-text-secondary);
    max-width: 480px;
    line-height: 1.6;
    margin: 0;
    word-break: break-word;
  }

  .error-retry-btn {
    padding: var(--space-sm) var(--space-xl);
    background: var(--color-brand);
    color: #fff;
    border: none;
    border-radius: var(--radius-md);
    font-size: 14px;
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .error-retry-btn:hover {
    background: var(--color-brand-hover);
  }

</style>
