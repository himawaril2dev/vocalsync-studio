<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { showToast } from "../stores/toast";
  import { t, tSync } from "../i18n";

  const VERSION = "0.2.13";
  const KOFI_URL = "https://ko-fi.com/himawari168";
  const GITHUB_URL = "https://github.com/himawaril2dev/vocalsync-studio";
  const ISSUES_URL = "https://github.com/himawaril2dev/vocalsync-studio/issues";
  const SUPPORT_EMAIL = "himawaril2dev@gmail.com";

  /** 後端回傳的 release 資訊結構，對應 `updates_commands::ReleaseInfo` */
  interface ReleaseInfo {
    tag_name: string;
    html_url: string;
  }

  let checking = $state(false);

  /**
   * 比對兩個版本號：回傳 > 0 表示 latest 較新、= 0 相同、< 0 current 較新。
   * 容忍前綴 "v" 與不同段數（如 "0.2.2" vs "v0.2.2.1"）。
   */
  function compareVersions(current: string, latest: string): number {
    const a = current.replace(/^v/, "").split(".").map(Number);
    const b = latest.replace(/^v/, "").split(".").map(Number);
    for (let i = 0; i < Math.max(a.length, b.length); i++) {
      const diff = (b[i] ?? 0) - (a[i] ?? 0);
      if (diff !== 0) return diff;
    }
    return 0;
  }

  async function copyEmail() {
    try {
      await navigator.clipboard.writeText(SUPPORT_EMAIL);
      showToast(tSync("about.toast.emailCopied"), "success");
    } catch (e) {
      showToast(tSync("about.toast.emailCopyFailed", { error: String(e) }), "error");
    }
  }

  async function checkForUpdates() {
    checking = true;
    try {
      // v0.2.2 起走後端 ureq（參見 updates_commands.rs）。
      // 不再從前端直接 fetch GitHub API，CSP connect-src 因此不必放寬。
      const info = await invoke<ReleaseInfo>("check_latest_release");
      const cmp = compareVersions(VERSION, info.tag_name);
      if (cmp > 0) {
        showToast(tSync("about.toast.updateAvailable", { version: info.tag_name }), "info");
      } else {
        showToast(tSync("about.toast.upToDate", { version: VERSION }), "success");
      }
    } catch (e) {
      showToast(tSync("about.toast.updateCheckFailed", { error: String(e) }), "error");
    } finally {
      checking = false;
    }
  }
</script>

<div class="about-page">
  <div class="hero">
    <h1 class="app-name">VocalSync Studio</h1>
    <span class="version">v{VERSION}</span>
    <p class="tagline">{$t("about.tagline")}</p>
  </div>

  <div class="disclosure">
    <strong class="disclosure-title">📢 {$t("about.disclosure.title")}</strong>
    <p class="disclosure-body">{$t("about.disclosure.body")}</p>
  </div>

  <div class="card">
    <h2>{$t("about.notes.title")}</h2>
    <div class="notes">
      <div class="note-item">
        <span class="note-num">1</span>
        <div>
          <strong>{$t("about.notes.1.title")}</strong>
          <p>{$t("about.notes.1.body")}</p>
        </div>
      </div>
      <div class="note-item">
        <span class="note-num">2</span>
        <div>
          <strong>{$t("about.notes.2.title")}</strong>
          <p>{$t("about.notes.2.body")}</p>
          <ul>
            <li>{$t("about.notes.2.li1.before")}<a href="https://github.com/Anjok07/ultimatevocalremovergui" target="_blank" rel="noopener">UVR5</a>{$t("about.notes.2.li1.middle")}<a href="https://moises.ai" target="_blank" rel="noopener">Moises</a>{$t("about.notes.2.li1.after")}</li>
            <li>{$t("about.notes.2.li2")}</li>
            <li>{$t("about.notes.2.li3")}</li>
          </ul>
        </div>
      </div>
      <div class="note-item">
        <span class="note-num">3</span>
        <div>
          <strong>{$t("about.notes.3.title")}</strong>
          <p>{$t("about.notes.3.body")}</p>
        </div>
      </div>
      <div class="note-item">
        <span class="note-num">4</span>
        <div>
          <strong>{$t("about.notes.4.title")}</strong>
          <p>{$t("about.notes.4.body")}</p>
        </div>
      </div>
    </div>
  </div>

  <div class="card">
    <h2>{$t("about.shortcuts.title")}</h2>
    <div class="shortcuts">
      <div class="shortcut"><kbd>Space</kbd><span>{$t("about.shortcuts.playPause")}</span></div>
      <div class="shortcut"><kbd>R</kbd><span>{$t("about.shortcuts.record")}</span></div>
      <div class="shortcut"><kbd>Esc</kbd><span>{$t("about.shortcuts.stop")}</span></div>
      <div class="shortcut"><kbd>A</kbd><span>{$t("about.shortcuts.loopA")}</span></div>
      <div class="shortcut"><kbd>B</kbd><span>{$t("about.shortcuts.loopB")}</span></div>
      <div class="shortcut"><kbd>+</kbd><span>{$t("about.shortcuts.pitchUp")}</span></div>
      <div class="shortcut"><kbd>-</kbd><span>{$t("about.shortcuts.pitchDown")}</span></div>
    </div>
  </div>

  <div class="card">
    <h2>{$t("about.license.title")}</h2>
    <p class="license-text">
      {$t("about.license.main.body1")}<strong>{$t("about.license.main.mit")}</strong>{$t("about.license.main.body2")}
    </p>
    <p class="license-note">
      {$t("about.license.main.note")}
    </p>

    <h3 class="license-subheading">{$t("about.license.ytdlp.heading")}</h3>
    <p class="license-note">
      {$t("about.license.ytdlp.body.before")}<a href="https://github.com/yt-dlp/yt-dlp" target="_blank" rel="noopener">yt-dlp</a>{$t("about.license.ytdlp.body.middle")}<a href="https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE" target="_blank" rel="noopener">The Unlicense</a>{$t("about.license.ytdlp.body.after")}
    </p>

    <h3 class="license-subheading">{$t("about.license.ffmpeg.heading")}</h3>
    <p class="license-note">
      {$t("about.license.ffmpeg.body.before")}<a href="https://ffmpeg.org/" target="_blank" rel="noopener">FFmpeg</a>{$t("about.license.ffmpeg.body.middle")}<a href="https://ffmpeg.org/legal.html" target="_blank" rel="noopener">LGPL-2.1-or-later</a>{$t("about.license.ffmpeg.body.after")}
    </p>

    <p class="license-disclaimer">
      {$t("about.license.disclaimer")}
    </p>
  </div>

  <div class="card creator-card">
    <h2>{$t("about.creator.title")}</h2>
    <p class="creator-name">himawari</p>
    <p class="creator-desc">
      {$t("about.creator.desc")}
    </p>
    <a
      class="donate-btn"
      href={KOFI_URL}
      target="_blank"
      rel="noopener"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 8h1a4 4 0 0 1 0 8h-1" />
        <path d="M2 8h16v9a4 4 0 0 1-4 4H6a4 4 0 0 1-4-4V8z" />
        <line x1="6" y1="1" x2="6" y2="4" />
        <line x1="10" y1="1" x2="10" y2="4" />
        <line x1="14" y1="1" x2="14" y2="4" />
      </svg>
      {$t("about.creator.donate")}
    </a>
  </div>

  <div class="card feedback-card">
    <h2>{$t("about.feedback.title")}</h2>
    <p class="feedback-desc">
      {$t("about.feedback.desc")}
    </p>
    <div class="feedback-row">
      <a
        class="link-btn feedback-btn"
        href={ISSUES_URL}
        target="_blank"
        rel="noopener"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
        {$t("about.feedback.githubIssues")}
      </a>
      <a
        class="link-btn feedback-btn"
        href="mailto:{SUPPORT_EMAIL}?subject={encodeURIComponent($t('about.feedback.mailSubject', { version: VERSION }))}"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z" />
          <polyline points="22,6 12,13 2,6" />
        </svg>
        {$t("about.feedback.email")}
      </a>
    </div>
    <button
      type="button"
      class="email-chip"
      onclick={copyEmail}
      title={$t("about.feedback.emailCopyTitle")}
    >
      <span class="email-label">{$t("about.feedback.emailLabel")}</span>
      <span class="email-value">{SUPPORT_EMAIL}</span>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
      </svg>
    </button>
  </div>

  <div class="card links-card">
    <h2>{$t("about.links.title")}</h2>
    <div class="link-row">
      <a class="link-btn" href={GITHUB_URL} target="_blank" rel="noopener">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z"/>
        </svg>
        {$t("about.links.github")}
      </a>
      <button class="link-btn update-btn" onclick={checkForUpdates} disabled={checking}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="23 4 23 10 17 10" />
          <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
        </svg>
        {checking ? $t("about.links.checking") : $t("about.links.checkUpdate")}
      </button>
    </div>
  </div>

  <p class="footer">
    Built with Tauri + Svelte + Rust
  </p>
</div>

<style>
  .about-page {
    padding: var(--space-xl);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-lg);
    height: 100%;
    overflow-y: auto;
  }

  .about-page > * {
    width: 100%;
    max-width: 600px;
  }

  .hero {
    text-align: center;
    padding: var(--space-xl) 0 var(--space-sm);
  }

  .app-name {
    font-size: 28px;
    font-weight: 700;
    color: var(--color-brand);
    margin: 0;
  }

  .version {
    display: inline-block;
    margin-top: var(--space-xs);
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-muted);
    background: var(--color-bg-hover);
    padding: 2px var(--space-sm);
    border-radius: var(--radius-sm);
  }

  .tagline {
    margin: var(--space-md) 0 0;
    font-size: 14px;
    color: var(--color-text-secondary);
  }

  .disclosure {
    padding: var(--space-md) var(--space-lg);
    background: var(--color-bg-hover);
    border-left: 3px solid var(--color-info);
    border-radius: var(--radius-md);
    text-align: left;
  }

  .disclosure-title {
    display: block;
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text);
    margin-bottom: var(--space-xs);
  }

  .disclosure-body {
    margin: 0;
    font-size: 12px;
    color: var(--color-text-secondary);
    line-height: 1.7;
  }

  .card {
    background: var(--color-bg-surface);
    border-radius: var(--radius-xl);
    padding: var(--space-xl);
  }

  .card h2 {
    margin: 0 0 var(--space-md);
    font-size: 15px;
    font-weight: 600;
    color: var(--color-text);
  }

  /* 使用須知 */
  .notes {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .note-item {
    display: flex;
    gap: var(--space-md);
  }

  .note-num {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--color-brand);
    color: #fff;
    font-size: 13px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    margin-top: 2px;
  }

  .note-item strong {
    display: block;
    font-size: 14px;
    color: var(--color-text);
    margin-bottom: var(--space-xs);
  }

  .note-item p {
    margin: 0;
    font-size: 13px;
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .note-item ul {
    margin: var(--space-xs) 0 0;
    padding-left: var(--space-xl);
    font-size: 13px;
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .note-item a {
    color: var(--color-info);
    text-decoration: none;
  }

  .note-item a:hover {
    text-decoration: underline;
  }

  /* 快捷鍵 */
  .shortcuts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-sm);
  }

  .shortcut {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: 13px;
    color: var(--color-text-secondary);
  }

  kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    padding: 2px var(--space-sm);
    background: var(--color-bg-hover);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text);
  }

  /* 授權 */
  .license-text {
    margin: 0 0 var(--space-sm);
    font-size: 13px;
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .license-note {
    margin: 0 0 var(--space-sm);
    font-size: 12px;
    color: var(--color-text-muted);
    line-height: 1.7;
  }

  .license-note a {
    color: var(--color-info);
    text-decoration: none;
  }

  .license-note a:hover {
    text-decoration: underline;
  }

  .license-subheading {
    margin: var(--space-md) 0 var(--space-xs);
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text);
  }

  .license-disclaimer {
    margin: var(--space-md) 0 0;
    padding: var(--space-sm) var(--space-md);
    background: var(--color-bg-hover);
    border-left: 3px solid var(--color-accent);
    border-radius: var(--radius-sm);
    font-size: 11px;
    color: var(--color-text-muted);
    line-height: 1.6;
  }

  /* 製作者 */
  .creator-card {
    text-align: center;
  }

  .creator-name {
    margin: 0 0 var(--space-sm);
    font-size: 18px;
    font-weight: 600;
    color: var(--color-brand);
  }

  .creator-desc {
    margin: 0 0 var(--space-lg);
    font-size: 13px;
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .donate-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-xl);
    background: var(--color-accent);
    color: #3d2b00;
    border-radius: var(--radius-lg);
    font-size: 14px;
    font-weight: 600;
    text-decoration: none;
    transition: background var(--transition-fast);
  }

  .donate-btn:hover {
    background: var(--color-accent-hover);
  }

  /* 問題回報 */
  .feedback-card {
    text-align: center;
  }

  .feedback-desc {
    margin: 0 0 var(--space-lg);
    font-size: 13px;
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .feedback-row {
    display: flex;
    justify-content: center;
    gap: var(--space-md);
    flex-wrap: wrap;
    margin-bottom: var(--space-md);
  }

  .feedback-btn {
    color: var(--color-text);
  }

  .email-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md);
    background: var(--color-bg-hover);
    border: 1px dashed var(--color-border);
    border-radius: var(--radius-md);
    font-size: 12px;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .email-chip:hover {
    background: var(--color-bg-surface);
    border-color: var(--color-brand);
    color: var(--color-text);
  }

  .email-label {
    font-weight: 600;
    color: var(--color-text-muted);
  }

  .email-value {
    font-family: var(--font-mono);
    letter-spacing: 0.2px;
  }

  /* 連結與更新 */
  .links-card {
    text-align: center;
  }

  .link-row {
    display: flex;
    justify-content: center;
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .link-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-xl);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-bg-surface);
    color: var(--color-text);
    font-size: 13px;
    font-weight: 500;
    text-decoration: none;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .link-btn:hover:not(:disabled) {
    background: var(--color-bg-hover);
    border-color: var(--color-border-light);
  }

  .update-btn {
    color: var(--color-brand);
    border-color: var(--color-brand);
  }

  .update-btn:hover:not(:disabled) {
    background: #fdf8ee;
  }

  .link-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .footer {
    text-align: center;
    font-size: 11px;
    color: var(--color-text-faint);
    padding-bottom: var(--space-lg);
    margin: 0;
  }
</style>
