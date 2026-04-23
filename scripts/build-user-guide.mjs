#!/usr/bin/env node
/**
 * Build self-contained HTML versions of USER_GUIDE for the portable zip.
 *
 * Input:  docs/USER_GUIDE{,.en,.ja}.md
 * Output: dist-docs/user-guide-{zh,en,ja}.html
 *
 * - Single-file HTML: inline CSS, no external fonts / JS / network calls.
 * - GitHub-style heading slugs so the in-doc TOC anchors resolve.
 * - Rewrites cross-language markdown links (USER_GUIDE.en.md → user-guide-en.html).
 * - Side panel TOC (auto-generated from h2/h3) + floating language switcher.
 */

import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { marked } from "marked";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");

/** @type {{ code: "zh" | "en" | "ja", label: string, src: string, out: string, lang: string, dir: string }[]} */
const LANGS = [
  {
    code: "zh",
    label: "繁體中文",
    src: "docs/USER_GUIDE.md",
    out: "dist-docs/user-guide-zh.html",
    lang: "zh-Hant",
    dir: "ltr",
  },
  {
    code: "en",
    label: "English",
    src: "docs/USER_GUIDE.en.md",
    out: "dist-docs/user-guide-en.html",
    lang: "en",
    dir: "ltr",
  },
  {
    code: "ja",
    label: "日本語",
    src: "docs/USER_GUIDE.ja.md",
    out: "dist-docs/user-guide-ja.html",
    lang: "ja",
    dir: "ltr",
  },
];

// GitHub-style slug: lowercase, strip punctuation/symbols, spaces → dashes, keep Unicode letters+digits.
function githubSlug(text) {
  return String(text)
    .toLowerCase()
    .trim()
    .replace(/<[^>]+>/g, "") // strip any inline HTML
    .replace(/[\u0000-\u001f]/g, "")
    .replace(/[!"#$%&'()*+,./:;<=>?@[\\\]^`{|}~。，、：；！？「」『』（）《》【】·．]/g, "")
    // GitHub-style: replace each whitespace char individually so "a · b" (becomes "a  b" after ·-strip) → "a--b", matching the source TOC links.
    .replace(/\s/g, "-");
}

// Escape for HTML attribute values.
function escapeAttr(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[c]));
}

// Escape HTML text.
function escapeText(s) {
  return String(s).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
}

/**
 * Render markdown → { html, toc }.
 *   - Custom heading renderer assigns GitHub-style IDs and records h2/h3 for the sidebar TOC.
 *   - Link renderer rewrites cross-language .md references to local .html siblings.
 */
function render(md) {
  const toc = [];
  const usedIds = new Map();

  const renderer = new marked.Renderer();

  renderer.heading = function heading({ tokens, depth }) {
    const text = this.parser.parseInline(tokens);
    const raw = tokens.map((t) => ("text" in t ? t.text : "")).join("");
    let id = githubSlug(raw);
    // Disambiguate repeat slugs the GitHub way: `-1`, `-2`, ...
    const seen = usedIds.get(id) ?? 0;
    usedIds.set(id, seen + 1);
    if (seen > 0) id = `${id}-${seen}`;
    if (depth === 2 || depth === 3) {
      toc.push({ depth, id, text: raw.trim() });
    }
    return `<h${depth} id="${escapeAttr(id)}"><a class="anchor" href="#${escapeAttr(id)}" aria-hidden="true">#</a>${text}</h${depth}>\n`;
  };

  renderer.link = function link({ href, title, tokens }) {
    const text = this.parser.parseInline(tokens);
    let mapped = href;
    if (href === "USER_GUIDE.md") mapped = "user-guide-zh.html";
    else if (href === "USER_GUIDE.en.md") mapped = "user-guide-en.html";
    else if (href === "USER_GUIDE.ja.md") mapped = "user-guide-ja.html";
    const titleAttr = title ? ` title="${escapeAttr(title)}"` : "";
    const isExternal = /^https?:/i.test(mapped);
    const extAttr = isExternal ? ' target="_blank" rel="noopener noreferrer"' : "";
    return `<a href="${escapeAttr(mapped)}"${titleAttr}${extAttr}>${text}</a>`;
  };

  marked.use({ renderer, gfm: true, breaks: false });

  const html = marked.parse(md);
  return { html, toc };
}

function renderTocSidebar(toc, strings) {
  if (toc.length === 0) return "";
  const items = toc
    .map(({ depth, id, text }) => {
      const cls = depth === 2 ? "toc-h2" : "toc-h3";
      return `<li class="${cls}"><a href="#${escapeAttr(id)}">${escapeText(text)}</a></li>`;
    })
    .join("\n      ");
  return `
  <aside class="sidebar" aria-label="${escapeAttr(strings.tocAria)}">
    <div class="sidebar-inner">
      <h2 class="sidebar-title">${escapeText(strings.tocHeading)}</h2>
      <ul class="toc">
      ${items}
      </ul>
    </div>
  </aside>`;
}

function renderLangSwitcher(activeCode) {
  const links = LANGS.map(({ code, label, out }) => {
    const file = out.split("/").pop();
    const active = code === activeCode ? ' aria-current="page"' : "";
    const cls = code === activeCode ? "lang-link active" : "lang-link";
    return `<a class="${cls}" href="${escapeAttr(file)}"${active}>${escapeText(label)}</a>`;
  }).join('<span class="lang-sep">·</span>');
  return `<nav class="lang-switcher" aria-label="Language">${links}</nav>`;
}

const STRINGS = {
  zh: { tocHeading: "目錄", tocAria: "本頁目錄", backToTop: "回到頂端", titleSuffix: "使用說明" },
  en: { tocHeading: "Contents", tocAria: "On this page", backToTop: "Back to top", titleSuffix: "User Guide" },
  ja: { tocHeading: "目次", tocAria: "このページの目次", backToTop: "トップへ戻る", titleSuffix: "ユーザーガイド" },
};

const INLINE_CSS = `
  :root {
    --bg: #ffffff;
    --fg: #1f2328;
    --muted: #57606a;
    --border: #d0d7de;
    --accent: #5b6cff;
    --accent-hover: #4050d9;
    --code-bg: #f6f8fa;
    --code-fg: #1f2328;
    --sidebar-bg: #f8fafc;
    --table-alt: #fafbfc;
    --blockquote-bg: #f6f8fa;
    --link: #4050d9;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #0d1117;
      --fg: #e6edf3;
      --muted: #8b949e;
      --border: #30363d;
      --accent: #7c8cff;
      --accent-hover: #a4b0ff;
      --code-bg: #161b22;
      --code-fg: #e6edf3;
      --sidebar-bg: #0b1017;
      --table-alt: #111720;
      --blockquote-bg: #161b22;
      --link: #a4b0ff;
    }
  }
  * { box-sizing: border-box; }
  html { scroll-behavior: smooth; scroll-padding-top: 72px; }
  body {
    margin: 0;
    background: var(--bg);
    color: var(--fg);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Hiragino Sans", "Noto Sans CJK TC", "Noto Sans CJK JP", "Microsoft JhengHei", "Yu Gothic", sans-serif;
    font-size: 16px;
    line-height: 1.7;
    -webkit-font-smoothing: antialiased;
  }
  a { color: var(--link); text-decoration: none; }
  a:hover { text-decoration: underline; }
  .topbar {
    position: sticky; top: 0; z-index: 10;
    background: color-mix(in srgb, var(--bg) 85%, transparent);
    backdrop-filter: saturate(140%) blur(10px);
    border-bottom: 1px solid var(--border);
    padding: 12px 20px;
    display: flex; justify-content: space-between; align-items: center;
    gap: 16px;
  }
  .topbar-brand { font-weight: 600; color: var(--fg); font-size: 15px; }
  .lang-switcher { font-size: 14px; color: var(--muted); }
  .lang-link { color: var(--muted); padding: 2px 6px; border-radius: 4px; }
  .lang-link:hover { background: var(--code-bg); text-decoration: none; }
  .lang-link.active { color: var(--fg); font-weight: 600; }
  .lang-sep { color: var(--border); margin: 0 4px; }
  .layout {
    max-width: 1200px; margin: 0 auto;
    display: grid; grid-template-columns: 260px minmax(0, 1fr);
    gap: 40px; padding: 24px 24px 80px;
  }
  .sidebar {
    position: sticky; top: 72px; align-self: start;
    max-height: calc(100vh - 96px); overflow: auto;
    padding: 20px 16px; background: var(--sidebar-bg);
    border: 1px solid var(--border); border-radius: 10px;
    font-size: 14px;
  }
  .sidebar-title { margin: 0 0 12px; font-size: 13px;
    text-transform: uppercase; letter-spacing: 0.06em;
    color: var(--muted); font-weight: 600; }
  .toc { list-style: none; padding: 0; margin: 0; }
  .toc li { margin: 2px 0; }
  .toc li.toc-h3 { padding-left: 16px; font-size: 13px; color: var(--muted); }
  .toc li a { display: block; padding: 4px 8px; border-radius: 4px; color: inherit; }
  .toc li a:hover { background: color-mix(in srgb, var(--accent) 14%, transparent); text-decoration: none; color: var(--fg); }
  main.content { min-width: 0; }
  main.content h1 { font-size: 2em; margin: 0.2em 0 0.6em; padding-bottom: 0.3em; border-bottom: 1px solid var(--border); }
  main.content h2 { font-size: 1.5em; margin: 2em 0 0.8em; padding-bottom: 0.3em; border-bottom: 1px solid var(--border); }
  main.content h3 { font-size: 1.2em; margin: 1.6em 0 0.6em; }
  main.content h4 { font-size: 1.05em; margin: 1.3em 0 0.5em; }
  main.content p { margin: 0.6em 0 1em; }
  main.content ul, main.content ol { padding-left: 1.6em; }
  main.content li { margin: 0.2em 0; }
  main.content blockquote {
    margin: 1em 0; padding: 12px 16px; background: var(--blockquote-bg);
    border-left: 4px solid var(--accent); border-radius: 4px; color: var(--muted);
  }
  main.content blockquote p:first-child { margin-top: 0; }
  main.content blockquote p:last-child { margin-bottom: 0; }
  main.content blockquote strong { color: var(--fg); }
  main.content hr { border: 0; border-top: 1px solid var(--border); margin: 2em 0; }
  main.content code {
    font-family: "SF Mono", "Consolas", "Menlo", "DejaVu Sans Mono", monospace;
    font-size: 0.92em; padding: 2px 6px; border-radius: 4px;
    background: var(--code-bg); color: var(--code-fg);
  }
  main.content pre {
    background: var(--code-bg); border: 1px solid var(--border);
    border-radius: 6px; padding: 12px 14px; overflow: auto;
  }
  main.content pre code { padding: 0; background: transparent; font-size: 0.88em; }
  main.content table {
    border-collapse: collapse; margin: 1em 0; display: block; overflow-x: auto;
    max-width: 100%;
  }
  main.content table th, main.content table td {
    border: 1px solid var(--border); padding: 8px 12px; text-align: left; vertical-align: top;
  }
  main.content table th { background: var(--code-bg); font-weight: 600; }
  main.content table tr:nth-child(even) td { background: var(--table-alt); }
  main.content img { max-width: 100%; height: auto; border-radius: 6px; }
  .anchor {
    color: var(--border); margin-right: 8px; font-weight: 400;
    opacity: 0; transition: opacity 0.2s; text-decoration: none;
  }
  h1:hover .anchor, h2:hover .anchor, h3:hover .anchor, h4:hover .anchor { opacity: 1; }
  .anchor:hover { color: var(--accent); text-decoration: none; }
  .back-to-top {
    position: fixed; right: 24px; bottom: 24px; z-index: 20;
    padding: 8px 14px; border-radius: 999px; font-size: 13px;
    background: var(--accent); color: white; border: none; cursor: pointer;
    box-shadow: 0 6px 20px rgba(0,0,0,0.18);
    opacity: 0; pointer-events: none; transition: opacity 0.2s;
    font-family: inherit;
  }
  .back-to-top.visible { opacity: 1; pointer-events: auto; }
  .back-to-top:hover { background: var(--accent-hover); }
  @media (max-width: 900px) {
    .layout { grid-template-columns: 1fr; padding: 16px; gap: 20px; }
    .sidebar { position: static; max-height: none; }
    html { scroll-padding-top: 60px; }
  }
  @media print {
    .topbar, .sidebar, .back-to-top { display: none !important; }
    .layout { display: block; padding: 0; }
    main.content { font-size: 11pt; }
    a { color: inherit; text-decoration: underline; }
  }
`;

const INLINE_JS = `
  (function () {
    var btn = document.getElementById('backToTop');
    if (!btn) return;
    function onScroll() {
      if (window.scrollY > 400) btn.classList.add('visible');
      else btn.classList.remove('visible');
    }
    window.addEventListener('scroll', onScroll, { passive: true });
    btn.addEventListener('click', function (e) {
      e.preventDefault();
      window.scrollTo({ top: 0, behavior: 'smooth' });
    });
    onScroll();
  })();
`;

function buildPage({ lang, dir, code, label, html, toc }) {
  const strings = STRINGS[code];
  const title = `VocalSync Studio ${strings.titleSuffix}`;
  const sidebar = renderTocSidebar(toc, strings);
  const switcher = renderLangSwitcher(code);
  return `<!DOCTYPE html>
<html lang="${escapeAttr(lang)}" dir="${escapeAttr(dir)}">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeText(title)}</title>
  <meta name="description" content="${escapeText(title)} — ${escapeText(label)}">
  <meta name="color-scheme" content="light dark">
  <style>${INLINE_CSS}</style>
</head>
<body>
  <header class="topbar">
    <div class="topbar-brand">VocalSync Studio · ${escapeText(strings.titleSuffix)}</div>
    ${switcher}
  </header>
  <div class="layout">${sidebar}
    <main class="content">${html}</main>
  </div>
  <button id="backToTop" class="back-to-top" type="button" title="${escapeAttr(strings.backToTop)}">${escapeText(strings.backToTop)} ↑</button>
  <script>${INLINE_JS}</script>
</body>
</html>
`;
}

async function build() {
  const outDir = resolve(ROOT, "dist-docs");
  await mkdir(outDir, { recursive: true });

  for (const entry of LANGS) {
    const srcPath = resolve(ROOT, entry.src);
    const outPath = resolve(ROOT, entry.out);
    const md = await readFile(srcPath, "utf8");
    const { html, toc } = render(md);
    const page = buildPage({ ...entry, html, toc });
    await writeFile(outPath, page, "utf8");
    const sizeKb = (Buffer.byteLength(page, "utf8") / 1024).toFixed(1);
    console.log(`  ✓ ${entry.out}  (${sizeKb} KB, ${toc.length} TOC entries)`);
  }
  console.log("\nUSER_GUIDE HTML bundle ready in dist-docs/.");
}

build().catch((err) => {
  console.error(err);
  process.exit(1);
});
