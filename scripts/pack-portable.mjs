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
import { spawn } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const HTML_FILES = ["user-guide-zh.html", "user-guide-en.html", "user-guide-ja.html"];

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
  const { version, releaseDir, portableRoot, exePath, dllPath, modelsSrc } = common;
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

  for (const name of HTML_FILES) {
    await copyFile(resolve(ROOT, "dist-docs", name), join(portableDir, name));
  }
  await writeFile(join(portableDir, "使用說明.html"), landingPageHtml(), "utf8");

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
