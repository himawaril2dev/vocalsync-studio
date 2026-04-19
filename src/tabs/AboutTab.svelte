<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { showToast } from "../stores/toast";

  const VERSION = "0.2.3";
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
      showToast("已複製電子信箱到剪貼簿", "success");
    } catch (e) {
      showToast(`複製失敗：${e}`, "error");
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
        showToast(`有新版本 ${info.tag_name} 可用，點選 GitHub 連結前往下載`, "info");
      } else {
        showToast(`目前已是最新版本 v${VERSION}`, "success");
      }
    } catch (e) {
      showToast(`檢查更新失敗：${e}`, "error");
    } finally {
      checking = false;
    }
  }
</script>

<div class="about-page">
  <div class="hero">
    <h1 class="app-name">VocalSync Studio</h1>
    <span class="version">v{VERSION}</span>
    <p class="tagline">練唱輔助工具，讓每一次練習都聽得見進步</p>
    <p class="ai-badge">100% AI-Crafted — 從架構設計、前後端程式碼到 UI，全程由 AI 生成</p>
  </div>

  <div class="card">
    <h2>使用須知</h2>
    <div class="notes">
      <div class="note-item">
        <span class="note-num">1</span>
        <div>
          <strong>準備伴奏</strong>
          <p>載入 off vocal / 伴奏版音檔（MP3、WAV、M4A 等皆可），或直接從 YouTube 下載。避免使用含原唱的版本，以免干擾錄音。</p>
        </div>
      </div>
      <div class="note-item">
        <span class="note-num">2</span>
        <div>
          <strong>目標旋律（選用）</strong>
          <p>若想在音高曲線上看到「正確音高」參考線，可透過以下方式取得：</p>
          <ul>
            <li>使用 <a href="https://github.com/Anjok07/ultimatevocalremovergui" target="_blank" rel="noopener">UVR5</a> 或 <a href="https://moises.ai" target="_blank" rel="noopener">Moises</a> 等工具，從原曲分離出乾淨人聲軌再匯入</li>
            <li>載入 MIDI 檔作為旋律參考</li>
            <li>使用內建的「快速消人聲」功能（適用立體聲 center-panned 音源）</li>
          </ul>
        </div>
      </div>
      <div class="note-item">
        <span class="note-num">3</span>
        <div>
          <strong>延遲校準</strong>
          <p>首次使用建議進行延遲校準（準備頁底部），讓錄音的時間軸與伴奏精確對齊。校準值會自動儲存。</p>
        </div>
      </div>
      <div class="note-item">
        <span class="note-num">4</span>
        <div>
          <strong>歌詞同步</strong>
          <p>支援 LRC / SRT / VTT 格式。下載 YouTube 字幕後可直接載入為歌詞，也可在同步編輯器中手動標記時間戳。</p>
        </div>
      </div>
    </div>
  </div>

  <div class="card">
    <h2>快捷鍵</h2>
    <div class="shortcuts">
      <div class="shortcut"><kbd>Space</kbd><span>播放 / 暫停</span></div>
      <div class="shortcut"><kbd>R</kbd><span>開始錄音</span></div>
      <div class="shortcut"><kbd>Esc</kbd><span>停止</span></div>
      <div class="shortcut"><kbd>A</kbd><span>設定循環 A 點</span></div>
      <div class="shortcut"><kbd>B</kbd><span>設定循環 B 點</span></div>
      <div class="shortcut"><kbd>+</kbd><span>升半音</span></div>
      <div class="shortcut"><kbd>-</kbd><span>降半音</span></div>
    </div>
  </div>

  <div class="card">
    <h2>授權</h2>
    <p class="license-text">
      VocalSync Studio 以 <strong>MIT License</strong> 開源發佈。
      你可以自由使用、修改及散佈本軟體，但不提供任何擔保。
    </p>
    <p class="license-note">
      本軟體使用的 CREPE 音高偵測模型由 NYU MARL 開發，同樣以 MIT License 授權。
      Tauri、Svelte、Symphonia、cpal、rustfft 等主要框架與函式庫皆為 MIT / Apache-2.0。
    </p>

    <h3 class="license-subheading">yt-dlp（Unlicense）</h3>
    <p class="license-note">
      <a href="https://github.com/yt-dlp/yt-dlp" target="_blank" rel="noopener">yt-dlp</a>
      以 <a href="https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE" target="_blank" rel="noopener">The Unlicense</a>
      釋出（等同公有領域）。本程式以 CLI subprocess 方式呼叫未修改的
      <code>yt-dlp.exe</code>，不做靜態連結。透過 yt-dlp 下載的內容是否
      符合當地著作權與 YouTube 服務條款，由使用者自行負責。
    </p>

    <h3 class="license-subheading">FFmpeg（LGPL 2.1+ / 可能為 GPL）</h3>
    <p class="license-note">
      <a href="https://ffmpeg.org/" target="_blank" rel="noopener">FFmpeg</a>
      預設以
      <a href="https://ffmpeg.org/legal.html" target="_blank" rel="noopener">LGPL-2.1-or-later</a>
      授權；若啟用 libx264 / libx265 等特定編碼器則改為 GPL-2.0-or-later。
      本程式以 CLI subprocess 方式呼叫 <code>ffmpeg</code> / <code>ffprobe</code>，
      不靜態連結 libav* 函式庫；本倉庫內未附帶 FFmpeg binaries。
      請自行留意所用 build 的具體授權版本（essentials build 通常為 LGPL；
      full build 含 GPL 組件）。
    </p>

    <p class="license-disclaimer">
      免責聲明：本工具僅為本地練唱輔助。透過本工具處理、下載的音訊內容，
      其著作權與合法使用責任均歸使用者本人，開發者不對任何不當使用負責。
    </p>
  </div>

  <div class="card creator-card">
    <h2>製作者</h2>
    <p class="creator-name">himawari</p>
    <p class="creator-desc">
      如果這個工具對你的練唱有幫助，歡迎請我喝杯咖啡，讓開發可以持續下去。
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
      Support on Ko-fi
    </a>
  </div>

  <div class="card feedback-card">
    <h2>問題回報</h2>
    <p class="feedback-desc">
      使用遇到問題、想回報 bug 或有功能建議都很歡迎，可以透過以下兩種方式聯絡我：
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
        GitHub Issues
      </a>
      <a
        class="link-btn feedback-btn"
        href="mailto:{SUPPORT_EMAIL}?subject=[VocalSync%20Studio%20v{VERSION}]%20問題回報"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z" />
          <polyline points="22,6 12,13 2,6" />
        </svg>
        寄信回報
      </a>
    </div>
    <button
      type="button"
      class="email-chip"
      onclick={copyEmail}
      title="點一下複製"
    >
      <span class="email-label">信箱</span>
      <span class="email-value">{SUPPORT_EMAIL}</span>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
      </svg>
    </button>
  </div>

  <div class="card links-card">
    <h2>連結與更新</h2>
    <div class="link-row">
      <a class="link-btn" href={GITHUB_URL} target="_blank" rel="noopener">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z"/>
        </svg>
        GitHub
      </a>
      <button class="link-btn update-btn" onclick={checkForUpdates} disabled={checking}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="23 4 23 10 17 10" />
          <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
        </svg>
        {checking ? "檢查中..." : "檢查更新"}
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

  .ai-badge {
    margin: var(--space-sm) 0 0;
    font-size: 12px;
    color: var(--color-brand);
    font-weight: 500;
    letter-spacing: 0.3px;
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

  .license-note code {
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 1px 4px;
    background: var(--color-bg-hover);
    border-radius: var(--radius-sm);
    color: var(--color-text);
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
