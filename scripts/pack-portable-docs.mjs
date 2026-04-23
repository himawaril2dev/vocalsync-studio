#!/usr/bin/env node
/**
 * Copy the generated USER_GUIDE HTMLs into the latest Tauri portable folder
 * and repackage the distribution zip so GitHub release users get offline docs.
 *
 *   1. Runs `npm run build:docs` if `dist-docs/*.html` are missing.
 *   2. Resolves the target portable folder for the current package.json version
 *      (src-tauri/target/release/bundle/portable/VocalSync Studio Portable <ver>).
 *   3. Copies the 3 HTML guides (+ a small landing page) into the folder root.
 *   4. Rebuilds `VocalSync.Studio.Portable.<ver>.zip` via PowerShell
 *      `Compress-Archive` so the documents ship in the release artifact.
 *
 * Design notes
 * ------------
 * - We intentionally do NOT rely on `bundle.resources` in tauri.conf.json,
 *   because Tauri v2 drops `../` paths into an `_up_/` subfolder inside the
 *   bundle — we want the guides at the portable root next to the .exe.
 * - We do not hand-roll zip compression (no archiver dep). PowerShell 5.1 is
 *   already required for the Tauri toolchain on Windows.
 */

import { readFile, copyFile, writeFile, mkdir, access, stat } from "node:fs/promises";
import { spawn } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");

const HTML_FILES = ["user-guide-zh.html", "user-guide-en.html", "user-guide-ja.html"];

async function exists(p) {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

async function runNpmBuildDocs() {
  console.log("→ Running `npm run build:docs` ...");
  await new Promise((resolvePromise, rejectPromise) => {
    const isWin = process.platform === "win32";
    const child = spawn(isWin ? "npm.cmd" : "npm", ["run", "build:docs"], {
      cwd: ROOT,
      stdio: "inherit",
      shell: false,
    });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`build:docs exited with code ${code}`));
    });
  });
}

function landingPageHtml() {
  return `<!DOCTYPE html>
<html lang="zh-Hant">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>VocalSync Studio — User Guide</title>
<style>
  :root { color-scheme: light dark; }
  body { margin: 0; font-family: -apple-system, "Segoe UI", "Hiragino Sans", "Noto Sans CJK TC", "Noto Sans CJK JP", "Microsoft JhengHei", sans-serif; background: #fafbfc; color: #1f2328; display: grid; place-items: center; min-height: 100vh; }
  @media (prefers-color-scheme: dark) { body { background: #0d1117; color: #e6edf3; } .card { background: #161b22 !important; border-color: #30363d !important; } a.btn { background: #30363d !important; color: #e6edf3 !important; } a.btn:hover { background: #3d444d !important; } }
  .card { background: white; border: 1px solid #d0d7de; border-radius: 12px; padding: 40px 48px; max-width: 520px; width: calc(100% - 32px); box-shadow: 0 12px 40px rgba(0,0,0,0.05); }
  h1 { margin: 0 0 8px; font-size: 24px; }
  p.sub { margin: 0 0 28px; color: #57606a; font-size: 14px; }
  .list { display: grid; gap: 10px; }
  a.btn { display: flex; justify-content: space-between; align-items: center; padding: 14px 18px; border-radius: 8px; background: #f6f8fa; color: #1f2328; text-decoration: none; font-size: 15px; border: 1px solid transparent; transition: background 0.15s, border-color 0.15s; }
  a.btn:hover { background: #eaeef2; border-color: #d0d7de; }
  a.btn .lang { font-weight: 600; }
  a.btn .arrow { color: #57606a; font-size: 13px; }
  footer { margin-top: 28px; font-size: 12px; color: #57606a; text-align: center; }
  footer a { color: inherit; }
</style>
</head>
<body>
  <div class="card">
    <h1>VocalSync Studio 使用說明</h1>
    <p class="sub">Choose your language · 言語を選択してください</p>
    <div class="list">
      <a class="btn" href="user-guide-zh.html"><span class="lang">繁體中文</span><span class="arrow">→</span></a>
      <a class="btn" href="user-guide-en.html"><span class="lang">English</span><span class="arrow">→</span></a>
      <a class="btn" href="user-guide-ja.html"><span class="lang">日本語</span><span class="arrow">→</span></a>
    </div>
    <footer>Offline copy · <a href="https://github.com/himawaril2dev/vocalsync-studio">GitHub</a></footer>
  </div>
</body>
</html>
`;
}

async function ensureHtmls() {
  const missing = [];
  for (const name of HTML_FILES) {
    const p = resolve(ROOT, "dist-docs", name);
    if (!(await exists(p))) missing.push(name);
  }
  if (missing.length > 0) {
    console.log(`  (missing: ${missing.join(", ")})`);
    await runNpmBuildDocs();
  }
}

async function findPortableFolder(version) {
  const expected = resolve(
    ROOT,
    "src-tauri",
    "target",
    "release",
    "bundle",
    "portable",
    `VocalSync Studio Portable ${version}`,
  );
  if (await exists(expected)) return expected;
  throw new Error(
    `Portable folder not found: ${expected}\n` +
      `Run \`npm run tauri build\` first (or pass --portable-dir=<path>).`,
  );
}

async function copyDocsInto(portableDir) {
  console.log(`→ Copying USER_GUIDE HTMLs into: ${portableDir}`);
  for (const name of HTML_FILES) {
    const src = resolve(ROOT, "dist-docs", name);
    const dst = resolve(portableDir, name);
    await copyFile(src, dst);
    console.log(`  ✓ ${name}`);
  }
  const landingPath = resolve(portableDir, "使用說明.html");
  await writeFile(landingPath, landingPageHtml(), "utf8");
  console.log("  ✓ 使用說明.html (landing page)");
}

async function repackZip(portableDir, version) {
  const bundleDir = resolve(portableDir, "..");
  const zipName = `VocalSync.Studio.Portable.${version}.zip`;
  const zipPath = resolve(bundleDir, zipName);

  // Use PowerShell Compress-Archive; it's already part of the Windows toolchain.
  // `-DestinationPath` gets overwritten via `-Force`. The archive root becomes
  // the portable folder, matching Tauri's original layout.
  console.log(`→ Re-packing ${zipName} ...`);

  const psArgs = [
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    `$ErrorActionPreference = 'Stop'; Compress-Archive -Path '${portableDir.replace(/'/g, "''")}' -DestinationPath '${zipPath.replace(/'/g, "''")}' -Force -CompressionLevel Optimal`,
  ];

  await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn("powershell.exe", psArgs, { cwd: ROOT, stdio: "inherit" });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`Compress-Archive exited with code ${code}`));
    });
  });

  const s = await stat(zipPath);
  const sizeMb = (s.size / 1024 / 1024).toFixed(2);
  console.log(`  ✓ ${zipName}  (${sizeMb} MB)`);
  return zipPath;
}

async function main() {
  const pkg = JSON.parse(await readFile(resolve(ROOT, "package.json"), "utf8"));
  const version = pkg.version;
  console.log(`VocalSync Studio portable docs packer — v${version}\n`);

  await ensureHtmls();
  const portableDir = await findPortableFolder(version);
  await copyDocsInto(portableDir);
  const zipPath = await repackZip(portableDir, version);

  console.log(`\nDone. Release this file:\n  ${zipPath}`);
}

main().catch((err) => {
  console.error(`\n✗ ${err.message}`);
  process.exit(1);
});
