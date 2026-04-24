#!/usr/bin/env node
/**
 * Assemble the VocalSync Studio portable bundle for the current
 * package.json version and zip it for GitHub Release.
 *
 * Pipeline (all versions are read from package.json):
 *   1. Require `src-tauri/target/release/vocalsync-studio.exe` to exist
 *      (run `npm run tauri build` first).
 *   2. Ensure `dist-docs/*.html` exist — auto-run `npm run build:docs` if not.
 *   3. Wipe and recreate `src-tauri/target/release/bundle/portable/
 *      VocalSync Studio Portable <ver>/`.
 *   4. Copy into it:
 *        vocalsync-studio.exe   ← src-tauri/target/release/
 *        DirectML.dll           ← src-tauri/target/release/
 *        yt-dlp.exe             ← reused from prior portable folder
 *        ffmpeg.exe             ← trusted local FFmpeg asset
 *        ffprobe.exe            ← trusted local FFmpeg asset
 *        tool-manifest.json     ← SHA-256 hashes for ffmpeg / ffprobe
 *        models/                ← src-tauri/models/
 *        user-guide-{zh,en,ja}.html + 使用說明.html ← dist-docs/
 *   5. Re-zip to VocalSync.Studio.Portable.<ver>.zip via PowerShell
 *      Compress-Archive.
 *
 * Design notes
 * ------------
 * - tauri.conf.json has bundle.targets = [] by policy (commit ab3f24e),
 *   so the official build output is just the .exe. This script fills in
 *   the runtime deps the way the v0.1.0–v0.2.6 releases have done manually.
 * - yt-dlp.exe is sourced from the most recent existing portable folder so
 *   the published SHA-256 stays stable between patch releases. If none
 *   exists (fresh checkout), the script explains where to put one.
 * - FFmpeg is copied from a known local asset folder or a prior portable
 *   folder, then recorded in tool-manifest.json for runtime verification.
 * - No external network calls.
 */

import { createReadStream } from "node:fs";
import { readFile, writeFile, mkdir, copyFile, rm, readdir, stat, access } from "node:fs/promises";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");

const HTML_FILES = ["user-guide-zh.html", "user-guide-en.html", "user-guide-ja.html"];
const FFMPEG_TOOL_NAMES = ["ffmpeg.exe", "ffprobe.exe"];
const LOCAL_FFMPEG_CANDIDATE_DIRS = [
  resolve(ROOT, "src-tauri", "tools", "ffmpeg"),
  resolve(ROOT, "..", "vocalsync-studio", "assets", "ffmpeg"),
];

async function exists(p) {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

async function runNpm(script) {
  const isWin = process.platform === "win32";
  await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(isWin ? "npm.cmd" : "npm", ["run", script], {
      cwd: ROOT,
      stdio: "inherit",
      shell: false,
    });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`npm run ${script} exited with code ${code}`));
    });
  });
}

/** Recursively copy a directory (or fail early with a readable error). */
async function copyDir(src, dst) {
  await mkdir(dst, { recursive: true });
  const entries = await readdir(src, { withFileTypes: true });
  for (const entry of entries) {
    const s = join(src, entry.name);
    const d = join(dst, entry.name);
    if (entry.isDirectory()) await copyDir(s, d);
    else await copyFile(s, d);
  }
}

async function sha256File(path) {
  const hash = createHash("sha256");
  await new Promise((resolvePromise, rejectPromise) => {
    const stream = createReadStream(path);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("error", rejectPromise);
    stream.on("end", resolvePromise);
  });
  return hash.digest("hex");
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

async function pickPriorYtdlp(portableRoot, currentVersion) {
  const entries = await readdir(portableRoot, { withFileTypes: true });
  const candidates = entries
    .filter((e) => e.isDirectory() && e.name.startsWith("VocalSync Studio Portable "))
    .map((e) => e.name)
    .filter((n) => !n.endsWith(` ${currentVersion}`)) // skip the folder we're about to (re)create
    .sort()
    .reverse();
  for (const name of candidates) {
    const candidate = join(portableRoot, name, "yt-dlp.exe");
    if (await exists(candidate)) return candidate;
  }
  return null;
}

async function hasFfmpegPair(dir) {
  for (const name of FFMPEG_TOOL_NAMES) {
    if (!(await exists(join(dir, name)))) return false;
  }
  return true;
}

async function pickPriorFfmpegDir(portableRoot, currentVersion) {
  const entries = await readdir(portableRoot, { withFileTypes: true });
  const candidates = entries
    .filter((e) => e.isDirectory() && e.name.startsWith("VocalSync Studio Portable "))
    .map((e) => e.name)
    .filter((n) => !n.endsWith(` ${currentVersion}`))
    .sort()
    .reverse();
  for (const name of candidates) {
    const candidate = join(portableRoot, name);
    if (await hasFfmpegPair(candidate)) return candidate;
  }
  return null;
}

async function pickFfmpegDir(portableRoot, currentVersion) {
  const prior = await pickPriorFfmpegDir(portableRoot, currentVersion);
  if (prior) return prior;

  for (const dir of LOCAL_FFMPEG_CANDIDATE_DIRS) {
    if (await hasFfmpegPair(dir)) return dir;
  }

  return null;
}

async function main() {
  const pkg = JSON.parse(await readFile(resolve(ROOT, "package.json"), "utf8"));
  const version = pkg.version;
  console.log(`VocalSync Studio portable builder — v${version}\n`);

  const releaseDir = resolve(ROOT, "src-tauri", "target", "release");
  const exePath = join(releaseDir, "vocalsync-studio.exe");
  const dllPath = join(releaseDir, "DirectML.dll");
  const modelsSrc = resolve(ROOT, "src-tauri", "models");

  if (!(await exists(exePath))) {
    throw new Error(
      `Missing ${exePath}\nRun \`npm run tauri build\` first to produce the release binary.`,
    );
  }
  if (!(await exists(dllPath))) {
    throw new Error(
      `Missing ${dllPath}\nDirectML.dll should be copied into target/release/ by the ort crate during build.`,
    );
  }
  if (!(await exists(modelsSrc))) {
    throw new Error(`Missing models folder: ${modelsSrc}`);
  }

  // Regenerate docs if needed.
  const docsMissing = [];
  for (const name of HTML_FILES) {
    if (!(await exists(resolve(ROOT, "dist-docs", name)))) docsMissing.push(name);
  }
  if (docsMissing.length > 0) {
    console.log(`→ Generating USER_GUIDE HTMLs (missing: ${docsMissing.join(", ")}) ...`);
    await runNpm("build:docs");
  }

  // Resolve yt-dlp.exe source (reuse from the most recent prior portable folder).
  const portableRoot = resolve(releaseDir, "bundle", "portable");
  await mkdir(portableRoot, { recursive: true });
  const ytdlpSrc = await pickPriorYtdlp(portableRoot, version);
  if (!ytdlpSrc) {
    throw new Error(
      `No prior yt-dlp.exe found.\nExpected to find it in one of the existing\n  ${portableRoot}\\VocalSync Studio Portable X.Y.Z/\nfolders. Copy a known-good yt-dlp.exe into one of them and retry.`,
    );
  }
  console.log(`→ Reusing yt-dlp.exe from: ${ytdlpSrc.replace(ROOT + "\\", "")}`);

  const ffmpegSrcDir = await pickFfmpegDir(portableRoot, version);
  if (!ffmpegSrcDir) {
    throw new Error(
      `No ffmpeg.exe / ffprobe.exe pair found.\nExpected to find them in:\n  ${LOCAL_FFMPEG_CANDIDATE_DIRS.join("\n  ")}\nor an existing prior portable folder under:\n  ${portableRoot}`,
    );
  }
  console.log(`→ Using FFmpeg tools from: ${ffmpegSrcDir.replace(ROOT + "\\", "")}`);

  // Wipe and recreate target folder.
  const portableDir = join(portableRoot, `VocalSync Studio Portable ${version}`);
  if (await exists(portableDir)) {
    console.log(`→ Removing existing ${portableDir}`);
    await rm(portableDir, { recursive: true, force: true });
  }
  await mkdir(portableDir, { recursive: true });

  // Copy binaries + models.
  console.log(`→ Assembling ${portableDir}`);
  await copyFile(exePath, join(portableDir, "vocalsync-studio.exe"));
  console.log("  ✓ vocalsync-studio.exe");
  await copyFile(dllPath, join(portableDir, "DirectML.dll"));
  console.log("  ✓ DirectML.dll");
  await copyFile(ytdlpSrc, join(portableDir, "yt-dlp.exe"));
  console.log("  ✓ yt-dlp.exe");
  await copyFile(join(ffmpegSrcDir, "ffmpeg.exe"), join(portableDir, "ffmpeg.exe"));
  console.log("  ✓ ffmpeg.exe");
  await copyFile(join(ffmpegSrcDir, "ffprobe.exe"), join(portableDir, "ffprobe.exe"));
  console.log("  ✓ ffprobe.exe");
  const toolManifest = {
    ffmpeg_sha256: await sha256File(join(portableDir, "ffmpeg.exe")),
    ffprobe_sha256: await sha256File(join(portableDir, "ffprobe.exe")),
  };
  await writeFile(join(portableDir, "tool-manifest.json"), `${JSON.stringify(toolManifest, null, 2)}\n`, "utf8");
  console.log("  ✓ tool-manifest.json");
  await copyDir(modelsSrc, join(portableDir, "models"));
  console.log("  ✓ models/");

  // Docs.
  for (const name of HTML_FILES) {
    await copyFile(resolve(ROOT, "dist-docs", name), join(portableDir, name));
    console.log(`  ✓ ${name}`);
  }
  await writeFile(join(portableDir, "使用說明.html"), landingPageHtml(), "utf8");
  console.log("  ✓ 使用說明.html (landing page)");

  // Zip.
  const zipName = `VocalSync.Studio.Portable.${version}.zip`;
  const zipPath = join(portableRoot, zipName);
  console.log(`→ Compressing ${zipName} ...`);
  await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        `$ErrorActionPreference = 'Stop'; Compress-Archive -Path '${portableDir.replace(/'/g, "''")}' -DestinationPath '${zipPath.replace(/'/g, "''")}' -Force -CompressionLevel Optimal`,
      ],
      { cwd: ROOT, stdio: "inherit" },
    );
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`Compress-Archive exited with code ${code}`));
    });
  });

  const s = await stat(zipPath);
  const sizeMb = (s.size / 1024 / 1024).toFixed(2);
  console.log(`  ✓ ${zipName}  (${sizeMb} MB)`);

  // SHA-256 for release notes.
  const sha = await new Promise((resolvePromise, rejectPromise) => {
    let out = "";
    const child = spawn(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `(Get-FileHash -Algorithm SHA256 '${zipPath.replace(/'/g, "''")}').Hash`,
      ],
      { cwd: ROOT },
    );
    child.stdout.on("data", (d) => (out += d.toString()));
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise(out.trim());
      else rejectPromise(new Error(`Get-FileHash exited with code ${code}`));
    });
  });

  console.log(`\nSHA-256: ${sha.toLowerCase()}`);
  console.log(`\nRelease artifact ready:\n  ${zipPath}`);
}

main().catch((err) => {
  console.error(`\n✗ ${err.message}`);
  process.exit(1);
});
