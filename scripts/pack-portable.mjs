#!/usr/bin/env node
/**
 * Assemble VocalSync Studio portable bundles.
 *
 * Default:
 *   npm run pack:portable
 *
 * Source separation is handled by external UVR5 in the current product flow,
 * so the default portable bundle does not include audio-separator workers.
 */

import {
  access,
  copyFile,
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { createWriteStream } from "node:fs";
import { spawn } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import https from "node:https";
import os from "node:os";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const HTML_FILES = ["user-guide-zh.html", "user-guide-en.html", "user-guide-ja.html"];
const WHISPER_RUNNER_URL =
  "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.4/whisper-bin-x64.zip";
const WHISPER_RUNNER_ZIP_SHA256 =
  "74f973345cb52ef5ba3ec9e7e7af8e48cc8c71722d1528603b80588a11f82e3e";
const WHISPER_RUNNER_EXE_SHA256 =
  "d4c598cf97de103f888d1a53b8abddc85bf27ab752f785ca69318cedc8a2cf64";

function argValue(name, fallback) {
  const prefix = `--${name}=`;
  const match = process.argv.slice(2).find((arg) => arg.startsWith(prefix));
  return match ? match.slice(prefix.length) : fallback;
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function copyDir(src, dst) {
  await mkdir(dst, { recursive: true });
  const entries = await readdir(src, { withFileTypes: true });
  for (const entry of entries) {
    const s = join(src, entry.name);
    const d = join(dst, entry.name);
    if (entry.isDirectory()) {
      await copyDir(s, d);
    } else {
      await copyFile(s, d);
    }
  }
}

async function listFiles(root) {
  const files = [];
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFiles(path)));
    } else if (entry.isFile()) {
      files.push(path);
    }
  }
  return files;
}

function localLeakNeedles() {
  const username = os.userInfo().username;
  const home = resolve(os.homedir());
  const values = [
    home,
    ROOT,
    process.env.USERPROFILE,
    process.env.HOME,
    username && username.length >= 3 ? username : null,
  ].filter(Boolean);
  return [...new Set(values)].flatMap((value) => [
    { label: value, bytes: Buffer.from(value, "utf8") },
    { label: value, bytes: Buffer.from(value, "utf16le") },
  ]);
}

async function assertNoLocalPathLeaks(root) {
  const needles = localLeakNeedles();
  const matches = [];
  for (const file of await listFiles(root)) {
    const data = await readFile(file);
    for (const needle of needles) {
      if (needle.bytes.length > 0 && data.includes(needle.bytes)) {
        matches.push(`${file}: ${needle.label}`);
      }
    }
  }
  if (matches.length > 0) {
    throw new Error(
      [
        "Refusing to package portable zip because local user or workspace data was found:",
        ...matches.slice(0, 20).map((match) => `  ${match}`),
        matches.length > 20 ? `  ... ${matches.length - 20} more` : "",
        "Build with npm run tauri:build:release before packaging.",
      ]
        .filter(Boolean)
        .join("\n"),
    );
  }
}

async function copyWhisperRuntime(src, dst) {
  await mkdir(dst, { recursive: true });
  const entries = await readdir(src, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const lower = entry.name.toLowerCase();
    const shouldCopy =
      lower === "whisper-cli.exe" ||
      lower === "whisper-cli" ||
      lower === "whisper_cpp_source.txt" ||
      lower.endsWith(".dll");
    if (shouldCopy) {
      await copyFile(join(src, entry.name), join(dst, entry.name));
    }
  }
}

async function runNpm(script) {
  const isWin = process.platform === "win32";
  await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(isWin ? "npm.cmd" : "npm", ["run", script], {
      cwd: ROOT,
      stdio: "inherit",
      shell: isWin,
    });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`npm run ${script} exited with code ${code}`));
    });
  });
}

async function compressDir(portableDir, zipPath) {
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
}

async function downloadFile(url, targetPath, redirectsLeft = 5) {
  await mkdir(dirname(targetPath), { recursive: true });
  await new Promise((resolvePromise, rejectPromise) => {
    const req = https.get(url, (res) => {
      const status = res.statusCode ?? 0;
      if ([301, 302, 303, 307, 308].includes(status)) {
        const location = res.headers.location;
        res.resume();
        if (!location || redirectsLeft <= 0) {
          rejectPromise(new Error(`Could not follow download redirect for ${url}`));
          return;
        }
        const nextUrl = new URL(location, url).toString();
        downloadFile(nextUrl, targetPath, redirectsLeft - 1).then(resolvePromise, rejectPromise);
        return;
      }
      if (status < 200 || status >= 300) {
        res.resume();
        rejectPromise(new Error(`Download failed with HTTP ${status}: ${url}`));
        return;
      }
      const out = createWriteStream(targetPath);
      res.pipe(out);
      out.on("finish", () => out.close(resolvePromise));
      out.on("error", rejectPromise);
    });
    req.on("error", rejectPromise);
  });
}

async function expandZip(zipPath, destDir) {
  if (await exists(destDir)) {
    await rm(destDir, { recursive: true, force: true });
  }
  await mkdir(destDir, { recursive: true });
  await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        `$ErrorActionPreference = 'Stop'; Expand-Archive -Path '${zipPath.replace(/'/g, "''")}' -DestinationPath '${destDir.replace(/'/g, "''")}' -Force`,
      ],
      { cwd: ROOT, stdio: "inherit" },
    );
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`Expand-Archive exited with code ${code}`));
    });
  });
}

async function sha256(path) {
  return await new Promise((resolvePromise, rejectPromise) => {
    let out = "";
    const child = spawn(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `(Get-FileHash -Algorithm SHA256 '${path.replace(/'/g, "''")}').Hash`,
      ],
      { cwd: ROOT },
    );
    child.stdout.on("data", (data) => {
      out += data.toString();
    });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise(out.trim().toLowerCase());
      else rejectPromise(new Error(`Get-FileHash exited with code ${code}`));
    });
  });
}

async function ensureWhisperRunner(releaseDir) {
  const runnerDir = join(releaseDir, "whisper");
  const runnerExe = join(runnerDir, "whisper-cli.exe");
  if ((await exists(runnerExe)) && (await sha256(runnerExe)) === WHISPER_RUNNER_EXE_SHA256) {
    return runnerDir;
  }

  const cacheDir = resolve(ROOT, "src-tauri", "target", "whisper-cache");
  const zipPath = join(cacheDir, "whisper-bin-x64-v1.8.4.zip");
  const extractedDir = join(cacheDir, "whisper-bin-x64-v1.8.4");

  if (!(await exists(zipPath)) || (await sha256(zipPath)) !== WHISPER_RUNNER_ZIP_SHA256) {
    console.log("Downloading bundled whisper.cpp runner...");
    await downloadFile(WHISPER_RUNNER_URL, zipPath);
  }
  const zipHash = await sha256(zipPath);
  if (zipHash !== WHISPER_RUNNER_ZIP_SHA256) {
    throw new Error(`Whisper runner zip SHA-256 mismatch: ${zipHash}`);
  }

  await expandZip(zipPath, extractedDir);
  const sourceDir = join(extractedDir, "Release");
  const sourceExe = join(sourceDir, "whisper-cli.exe");
  if (!(await exists(sourceExe))) {
    throw new Error(`Missing whisper-cli.exe after extraction: ${sourceExe}`);
  }
  const exeHash = await sha256(sourceExe);
  if (exeHash !== WHISPER_RUNNER_EXE_SHA256) {
    throw new Error(`whisper-cli.exe SHA-256 mismatch: ${exeHash}`);
  }

  await rm(runnerDir, { recursive: true, force: true });
  await mkdir(runnerDir, { recursive: true });
  const files = await readdir(sourceDir, { withFileTypes: true });
  for (const file of files) {
    const lower = file.name.toLowerCase();
    if (file.isFile() && (lower === "whisper-cli.exe" || lower.endsWith(".dll"))) {
      await copyFile(join(sourceDir, file.name), join(runnerDir, file.name));
    }
  }
  await writeFile(
    join(runnerDir, "WHISPER_CPP_SOURCE.txt"),
    [
      "whisper.cpp Windows runner",
      "Version: v1.8.4",
      `Source: ${WHISPER_RUNNER_URL}`,
      `Zip SHA-256: ${WHISPER_RUNNER_ZIP_SHA256}`,
      `whisper-cli.exe SHA-256: ${WHISPER_RUNNER_EXE_SHA256}`,
      "License: MIT",
      "",
    ].join("\n"),
    "utf8",
  );
  return runnerDir;
}

function landingPageHtml() {
  return `<!DOCTYPE html>
<html lang="zh-Hant">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>VocalSync Studio User Guide</title>
<style>
  body { margin: 0; font-family: -apple-system, "Segoe UI", "Noto Sans CJK TC", "Microsoft JhengHei", sans-serif; background: #faf8f4; color: #2f2924; display: grid; place-items: center; min-height: 100vh; }
  .card { background: #fff; border: 1px solid #e8e2d8; border-radius: 18px; padding: 40px 48px; max-width: 520px; width: calc(100% - 32px); box-shadow: 0 18px 50px rgba(62, 45, 17, 0.08); }
  h1 { margin: 0 0 8px; font-size: 24px; }
  p.sub { margin: 0 0 28px; color: #7a7268; font-size: 14px; }
  .list { display: grid; gap: 10px; }
  a.btn { display: flex; justify-content: space-between; align-items: center; padding: 14px 18px; border-radius: 12px; background: #f4efe7; color: #2f2924; text-decoration: none; font-size: 15px; border: 1px solid transparent; }
  a.btn:hover { background: #ebe2d5; border-color: #d8c9ad; }
  a.btn .lang { font-weight: 700; }
  a.btn .arrow { color: #8a6500; font-size: 13px; }
  footer { margin-top: 28px; font-size: 12px; color: #7a7268; text-align: center; }
  footer a { color: inherit; }
</style>
</head>
<body>
  <div class="card">
    <h1>VocalSync Studio 使用說明</h1>
    <p class="sub">Choose your language</p>
    <div class="list">
      <a class="btn" href="user-guide-zh.html"><span class="lang">繁體中文</span><span class="arrow">open</span></a>
      <a class="btn" href="user-guide-en.html"><span class="lang">English</span><span class="arrow">open</span></a>
      <a class="btn" href="user-guide-ja.html"><span class="lang">日本語</span><span class="arrow">open</span></a>
    </div>
    <footer>Offline copy - <a href="https://github.com/himawaril2dev/vocalsync-studio">GitHub</a></footer>
  </div>
</body>
</html>
`;
}

function flavorPlan(flavor, version) {
  const plans = {
    base: {
      flavor: "base",
      folderName: `VocalSync Studio Portable ${version}`,
      zipName: `VocalSync.Studio.Portable.${version}.zip`,
    },
    full: {
      flavor: "full",
      folderName: `VocalSync Studio Portable ${version}`,
      zipName: `VocalSync.Studio.Portable.${version}.zip`,
    },
  };
  return plans[flavor];
}

function selectedPlans(flavor, version) {
  const plan = flavorPlan(flavor, version);
  if (!plan) {
    throw new Error("Unknown flavor. Use base or full.");
  }
  return [plan];
}

async function ensureDocs() {
  const docsMissing = [];
  for (const name of HTML_FILES) {
    if (!(await exists(resolve(ROOT, "dist-docs", name)))) {
      docsMissing.push(name);
    }
  }
  if (docsMissing.length > 0) {
    console.log(`Generating USER_GUIDE HTMLs (${docsMissing.join(", ")}) ...`);
    await runNpm("build:docs");
  }
}

async function removeStaleReleaseWorkers(releaseDir) {
  const workerRoot = join(releaseDir, "separation-worker");
  if (await exists(workerRoot)) {
    await rm(workerRoot, { recursive: true, force: true });
  }
}

async function assemble(plan, common) {
  const {
    version,
    releaseDir,
    portableRoot,
    exePath,
    dllPath,
    modelsSrc,
    whisperRunnerDir,
  } = common;
  const portableDir = join(portableRoot, plan.folderName);
  const zipPath = join(portableRoot, plan.zipName);

  if (await exists(portableDir)) {
    console.log(`Removing existing ${portableDir}`);
    await rm(portableDir, { recursive: true, force: true });
  }
  await mkdir(portableDir, { recursive: true });

  console.log(`Assembling ${plan.folderName}`);
  await copyFile(exePath, join(portableDir, "vocalsync-studio.exe"));
  await copyFile(dllPath, join(portableDir, "DirectML.dll"));
  await copyDir(modelsSrc, join(portableDir, "models"));
  await copyWhisperRuntime(whisperRunnerDir, join(portableDir, "whisper"));

  for (const name of HTML_FILES) {
    await copyFile(resolve(ROOT, "dist-docs", name), join(portableDir, name));
  }
  await writeFile(join(portableDir, "使用說明.html"), landingPageHtml(), "utf8");

  await assertNoLocalPathLeaks(portableDir);
  await compressDir(portableDir, zipPath);
  const size = await stat(zipPath);
  const hash = await sha256(zipPath);

  console.log(`Ready: ${plan.zipName}`);
  console.log(`Size: ${(size.size / 1024 / 1024).toFixed(2)} MB`);
  console.log(`SHA-256: ${hash}`);
  return { zipPath, hash, version };
}

async function main() {
  const flavor = argValue("flavor", "base").toLowerCase();
  const pkg = JSON.parse(await readFile(resolve(ROOT, "package.json"), "utf8"));
  const version = pkg.version;
  const releaseDir = resolve(ROOT, "src-tauri", "target", "release");
  const exePath = join(releaseDir, "vocalsync-studio.exe");
  const dllPath = join(releaseDir, "DirectML.dll");
  const modelsSrc = resolve(ROOT, "src-tauri", "models");

  if (!(await exists(exePath))) {
    throw new Error(`Missing ${exePath}. Run npm run tauri build first.`);
  }
  if (!(await exists(dllPath))) {
    throw new Error(`Missing ${dllPath}. Run npm run tauri build first.`);
  }
  if (!(await exists(modelsSrc))) {
    throw new Error(`Missing models folder: ${modelsSrc}`);
  }

  const plans = selectedPlans(flavor, version);
  await ensureDocs();
  await removeStaleReleaseWorkers(releaseDir);
  const whisperRunnerDir = await ensureWhisperRunner(releaseDir);

  const portableRoot = resolve(releaseDir, "bundle", "portable");
  await mkdir(portableRoot, { recursive: true });

  console.log(`VocalSync Studio portable builder - v${version} - ${flavor}`);
  const results = [];
  for (const plan of plans) {
    results.push(
      await assemble(plan, {
        version,
        releaseDir,
        portableRoot,
        exePath,
        dllPath,
        modelsSrc,
        whisperRunnerDir,
      }),
    );
  }

  console.log("\nRelease artifacts ready:");
  for (const result of results) {
    console.log(`  ${result.zipPath}`);
    console.log(`  SHA-256: ${result.hash}`);
  }
}

main().catch((err) => {
  console.error(`\n${err.message}`);
  process.exit(1);
});
