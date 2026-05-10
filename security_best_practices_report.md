# VocalSync Studio Security Review

Review date: 2026-05-08; v0.3.3/v0.3.6 addenda: 2026-05-10
Reviewed version: v0.2.19; v0.3.3 Whisper downloader and v0.3.6 portable session addenda included below
Scope: `src/`, `src-tauri/`, `scripts/`, Tauri config, npm/Rust dependency advisories.

## 2026-05-10 v0.3.6 Portable Session Addendum

Scope: portable-root storage, `project-session.json`, restored lyrics/backing/vocals/pitch state, AppData cleanup, and current Whisper model list.

No Critical or High severity issue was found in this pass. `npm audit --json` reports 0 vulnerabilities. `cargo audit --json` reports 0 vulnerabilities; informational warnings remain for unmaintained/unsound transitive crates.

Fix status: P1-P4 were fixed on 2026-05-10. Verification passed with `cargo check`, `cargo test`, `npm.cmd run build`, `npm.cmd run tauri build`, and `npm.cmd run pack:portable`.

Portable artifact: `src-tauri/target/release/bundle/portable/VocalSync.Studio.Portable.0.3.6.zip`; SHA-256 `8db6269ecbaa67c4fa078b7a719bec7ec6ed14bbe395eafc41a532973d964113`.

### P1 - Medium - Project session JSON is unbounded and only lightly validated

Status:
Fixed. `project-session.json` is size-capped before read/write, sanitized on the Rust side, and invalid root JSON is quarantined.

Location:
- `src-tauri/src/commands/settings_commands.rs:65`
- `src-tauri/src/commands/settings_commands.rs:75`
- `src-tauri/src/commands/settings_commands.rs:79`
- `src/tabs/SetupTab.svelte:201`
- `src/tabs/SetupTab.svelte:268`

Evidence:
- `load_project_session` reads the whole `project-session.json` into memory.
- `save_project_session` accepts renderer-supplied JSON, verifies only that it parses as JSON, then writes it to the portable root.
- `sanitizeProjectSession` validates only a small subset of fields and casts `melody` / `alignmentResult` into typed objects.

Impact:
A compromised renderer or tampered portable-root `project-session.json` can write or load very large JSON and cause startup/render-time CPU or memory exhaustion. A malformed melody payload can also drive expensive pitch-track reconstruction during restore.

Fix:
- Enforce a backend size limit before parsing and writing `project-session.json`.
- Validate the session as a typed Rust struct before saving.
- Cap lyrics lines, melody notes, raw pitch samples, text length, and numeric ranges.
- Reject invalid session files and keep the app usable by renaming the bad file to a quarantine name.

False positive notes:
Normal UI snapshots are expected to be small enough. This finding is about renderer-compromise and local tampering containment.

### P2 - Medium - Restored media paths are auto-opened from session state without a fresh grant

Status:
Fixed. Restored session media paths are validated as local-drive files, UNC/device paths are rejected, extensions and file sizes are checked, and stale entries are cleared from the sanitized session.

Location:
- `src/tabs/SetupTab.svelte:281`
- `src/tabs/SetupTab.svelte:283`
- `src/tabs/SetupTab.svelte:307`
- `src/tabs/SetupTab.svelte:309`
- `src/tabs/SetupTab.svelte:563`
- `src-tauri/src/security.rs:22`
- `src-tauri/src/security.rs:31`

Evidence:
- Startup restore reads `backingPath` and `guideVocalPath` from `project-session.json`, then calls backend media loaders.
- `validate_path_safe` requires an absolute path and rejects traversal/control characters, but it does not bind paths to a Tauri dialog grant and does not explicitly reject Windows UNC/network paths.

Impact:
A tampered session file can make the app attempt to open arbitrary absolute media paths at startup. On Windows, UNC paths can trigger network access and may leak Windows authentication metadata or hang the app while probing media.

Fix:
- Store a backend-issued restore token or canonical path record when the user selects the file through a dialog.
- On restore, allow only canonical local drive paths and reject UNC/network paths by default.
- Check extension, existence, file size, and parent directory consistency before loading restored paths.
- Clear or quarantine stale session entries that fail validation.

False positive notes:
Normal app use writes session paths only after a user dialog selection. The risk appears when the portable root is tampered with or when a renderer compromise invokes commands directly.

### P3 - Medium - Official Whisper model integrity still uses SHA-1 pins

Status:
Fixed. The two official Whisper model options now use SHA-256 pins and exact file-size checks before activation.

Location:
- `src-tauri/src/core/whisper_engine.rs:119`
- `src-tauri/src/core/whisper_engine.rs:131`
- `src-tauri/src/core/whisper_engine.rs:141`
- `src-tauri/src/core/whisper_engine.rs:533`
- `src-tauri/src/core/whisper_engine.rs:994`

Evidence:
- The two remaining official model specs store SHA-1 hashes.
- Model install verifies downloaded model content with `verify_sha1`.

Impact:
The app downloads model files that are parsed by native Whisper code. Model integrity should use modern SHA-256 pins, matching the runner download policy.

Fix:
- Replace `sha1` fields with pinned SHA-256 values.
- Verify downloaded temporary models with SHA-256 before rename.
- Keep manifest SHA-256 revalidation before transcription.

### P4 - Medium - Managed Whisper runner install still trusts pre-existing runner files

Status:
Fixed. Existing managed runners are reused only when the manifest already trusts the same path and SHA-256; fresh installs extract into a temporary tree, validate the pinned executable, then replace the install directory.

Location:
- `src-tauri/src/core/whisper_engine.rs:823`
- `src-tauri/src/core/whisper_engine.rs:824`
- `src-tauri/src/core/whisper_engine.rs:825`
- `src-tauri/src/core/whisper_engine.rs:867`

Evidence:
- `install_whisper_runner` first searches the managed install directory for an existing `whisper-cli.exe`.
- If found, it computes the hash and writes trust to the manifest before downloading or verifying the pinned official zip.

Impact:
A pre-seeded executable under the managed portable root can become trusted without being proven to come from the pinned official runner zip. This is a local tampering hardening gap.

Fix:
- Reuse an existing managed runner only when the existing manifest already trusts that exact SHA-256.
- For a fresh install, always download the pinned zip into a temporary directory, verify SHA-256, extract into a fresh temporary tree, then atomically replace the install directory.
- Optionally pin the extracted `whisper-cli.exe` SHA-256.

### Positive controls confirmed in v0.3.6

- Frontend scan found no `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval`, `new Function`, `postMessage`, unsafe navigation sink, or `javascript:` URL usage under `src/`.
- `localStorage` is used only for UI section state and locale, not secrets.
- Tauri capabilities expose core/dialog APIs only; shell, fs, and http plugins are not enabled.
- CSP is present, `object-src 'none'`, `base-uri 'self'`, `frame-ancestors 'none'`; inline style remains the existing accepted Svelte exception.
- `assetProtocol.scope` is empty.
- Subprocess execution uses `Command::new(...).arg(...)`; no shell interpolation was found in the reviewed paths.
- Old Studio AppData folders were removed; app-owned state now targets the portable root.

## 2026-05-10 v0.3.3 Whisper Downloader Addendum

Scope: new Whisper runner download, model download, local trust, and transcription execution paths.

No Critical or High severity issue was found in this pass. `npm audit --json` reports 0 vulnerabilities. `cargo audit --json` reports 0 vulnerabilities; the existing informational warnings for unmaintained/unsound transitive crates remain.

### W1 - Medium - Whisper runner trust can be persisted from renderer-supplied paths

Location:
- `src-tauri/src/commands/whisper_commands.rs:20`
- `src-tauri/src/commands/whisper_commands.rs:27`
- `src-tauri/src/core/whisper_engine.rs:524`
- `src-tauri/src/core/whisper_engine.rs:530`
- `src-tauri/src/core/whisper_engine.rs:877`

Evidence:
- `inspect_local_whisper_runner_path(path)` accepts a renderer-supplied path and returns the canonical path plus SHA-256.
- `trust_local_whisper_runner(candidate)` accepts the renderer-supplied candidate and persists it into the Whisper manifest after recomputing the hash.
- Transcription later executes the trusted runner through `Command::new(&runner)`.

Impact:
If the renderer is compromised, it can attempt to trust an existing local executable named `whisper-cli.exe`, `main.exe`, `whisper.exe`, or `whisper-cpp.exe`, then trigger transcription to execute it. This requires a suitable executable already present at a known path, so the risk is renderer-compromise containment rather than the normal UI flow.

Fix:
- Move manual runner selection into a backend-owned dialog command and persist trust only for a path returned by that dialog.
- Alternatively, issue a short-lived backend candidate token after inspection and require that token for trust.
- Keep managed Whisper download as the default path for normal users.

### W2 - Medium - Managed runner install trusts a pre-existing executable before proving it came from the pinned zip

Location:
- `src-tauri/src/core/whisper_engine.rs:620`
- `src-tauri/src/core/whisper_engine.rs:621`
- `src-tauri/src/core/whisper_engine.rs:623`
- `src-tauri/src/core/whisper_engine.rs:664`

Evidence:
- `install_whisper_runner` first searches the managed install directory for an allowed runner filename.
- When found, it computes that executable's SHA-256 and saves it to the manifest.
- The official zip SHA-256 verification runs only on the download path.

Impact:
A pre-seeded executable under the managed install directory can become trusted without being tied back to the pinned official zip. This is mainly a local tampering hardening gap because the directory is under the user's app data.

Fix:
- Reuse an existing managed runner only when the existing manifest already trusts it.
- For a fresh managed install, download the pinned zip, verify `WHISPER_RUNNER_ZIP_SHA256`, extract into a fresh temporary directory, and atomically replace the install directory.
- Optionally pin the extracted `whisper-cli.exe` SHA-256 as an additional check. The v1.8.4 Windows `Release/whisper-cli.exe` from the verified zip hashes to `D4C598CF97DE103F888D1A53B8ABDDC85BF27AB752F785CA69318CEDC8A2CF64`.

### W3 - Medium - Whisper model download integrity is pinned with SHA-1

Location:
- `src-tauri/src/core/whisper_engine.rs:112`
- `src-tauri/src/core/whisper_engine.rs:121`
- `src-tauri/src/core/whisper_engine.rs:130`
- `src-tauri/src/core/whisper_engine.rs:403`
- `src-tauri/src/core/whisper_engine.rs:794`

Evidence:
- The managed model specs store SHA-1 hashes.
- Downloads verify SHA-1 before activation, then store a computed SHA-256 in the local manifest.
- There is no pinned SHA-256 constant for the remote model content.

Impact:
SHA-1 is collision-broken and is weaker than the runner's SHA-256 pin. Model files are parsed by native Whisper code, so model integrity should use the same modern checksum level as executable/tool downloads.

Fix:
- Replace model `sha1` pins with SHA-256 pins.
- Verify the downloaded temporary model with SHA-256 before rename.
- Keep the manifest SHA-256 recheck before every transcription run.

### W4 - Low - Runner zip extraction writes into an existing install tree

Location:
- `src-tauri/src/core/whisper_engine.rs:618`
- `src-tauri/src/core/whisper_engine.rs:698`
- `src-tauri/src/core/whisper_engine.rs:700`
- `src-tauri/src/core/whisper_engine.rs:703`
- `src-tauri/src/core/whisper_engine.rs:705`

Evidence:
- Zip entries are protected with `enclosed_name` and `safe_zip_entry_path`, which blocks zip traversal.
- Extraction still writes into an existing app-data install tree using `create_dir_all`, `remove_file`, and `File::create`.

Impact:
A malicious pre-existing directory layout, such as a Windows junction/reparse point under the install tree, could redirect extraction writes. This requires local filesystem tampering and is a defense-in-depth issue.

Fix:
- Extract into a newly created temporary directory.
- Verify each canonical output parent remains under the temporary root before writing.
- Rename the finished temporary directory into place after verification.

Positive controls confirmed in v0.3.3:
- Whisper runner URL is fixed and the downloaded zip is pinned with SHA-256.
- Download byte limits exist for runner and model downloads.
- Transcription uses `Command::new(...).arg(...)`; no shell interpolation was found.
- Runner/model manifest entries are rehashed before use.
- `normalize_language` maps UI input to a fixed Whisper language set.
- Tauri capabilities expose core/dialog APIs only; shell, fs, and http plugins are not enabled.
- Frontend scan found no `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `{@html}`, `document.write`, `eval`, or `new Function` usage under `src/`.
- CSP still has the previously accepted inline-style exception.

## Executive Summary

No Critical or High severity issue was found in this pass.

The app is in a good baseline state for a local Tauri tool: Dependabot open alerts are 0, `npm audit` reports 0 vulnerabilities, `cargo audit` reports 0 vulnerabilities, CSP is present, asset protocol scope is empty, no shell plugin / fs plugin / http plugin is exposed, tool downloads are pinned by SHA-256, and yt-dlp / FFmpeg subprocess calls use argument arrays instead of shell interpolation.

Patch status: `save_lyrics_as_lrc` now opens the save dialog in the backend, enforces `.lrc`, and writes only the selected dialog path. The offline docs builder now strips raw Markdown HTML and rejects unsafe link/image URL schemes. CSP inline styles are recorded as an accepted exception for the current Svelte UI.

The remaining security work is defense-in-depth against a compromised renderer in the broader export/download flows. Several backend commands still validate paths only as absolute, non-traversal paths, so an XSS or renderer compromise could invoke those file-write operations against arbitrary user-writable locations. I found no current DOM XSS sink that makes this directly exploitable.

## Findings

### S1 - Medium - File-write commands are not bound to a dialog-granted path

Status:
Fixed for `save_lyrics_as_lrc` in this patch. Broader backend path-grant hardening remains for audio export and download output directories.

Location:
- `src-tauri/src/commands/lyrics_commands.rs:30`
- `src-tauri/src/commands/lyrics_commands.rs:56`
- `src-tauri/src/commands/lyrics_commands.rs:88`
- `src-tauri/src/commands/audio_commands.rs:175`
- `src-tauri/src/commands/audio_commands.rs:182`
- `src-tauri/src/core/ytdlp_engine.rs:1505`
- `src-tauri/src/core/ytdlp_engine.rs:1507`
- `src-tauri/src/security.rs:22`
- `src-tauri/src/security.rs:31`

Evidence:
- Before this patch, `save_lyrics_as_lrc(lines, output_path)` accepted `output_path` from the renderer and wrote it directly after `validate_path_safe`.
- After this patch, `save_lyrics_as_lrc` opens the Tauri save dialog in the backend, converts the dialog result into a path, canonicalizes the parent directory, enforces `.lrc`, and writes only that selected path.
- `export_audio(dir, prefix, ...)` accepts `dir` from the renderer and writes exported WAV files there.
- `run_download` accepts `req.output_dir`, creates the directory, then passes it into the yt-dlp output template.
- `validate_path_safe` checks empty/control chars, absolute path, `..`, and leading `-`; it does not verify that the remaining audio/download paths came from a save/open dialog or an allowlisted output root.

Impact:
If the renderer is compromised, the remaining audio export and download output-directory commands could still be invoked against arbitrary user-writable locations. The clearest LRC write case is fixed in this patch.

Fix:
- `save_lyrics_as_lrc` now routes save selection through Rust, uses the Tauri dialog backend grant, canonicalizes the parent directory, and enforces `.lrc` output.
- Add backend-side path grants for the remaining save/open results. A simple approach is to route save/open dialogs through Rust commands, store granted canonical paths in app state, and require a matching grant before write/read commands execute.
- For `export_audio`, require a granted directory and keep generated filenames backend-controlled.
- For `start_download`, require a granted output directory and keep the batch limit in backend enforcement.

False positive notes:
The current frontend uses Tauri dialog flows before invoking these commands. This finding is about renderer-compromise containment, not normal user flow.

### S2 - Low - Offline docs builder renders Markdown HTML without sanitization or URL scheme allowlist

Status:
Fixed in this patch.

Location:
- `scripts/build-user-guide.mjs:78`
- `scripts/build-user-guide.mjs:119`
- `scripts/build-user-guide.mjs:127`
- `scripts/build-user-guide.mjs:134`

Evidence:
- The script still uses `marked.parse(md)` and writes the result into static HTML.
- `safeHref` now rejects empty values, protocol-relative URLs, and schemes outside `http:`, `https:`, and `mailto:`.
- Custom link and image renderers now emit URLs only after `safeHref`.
- The custom HTML renderer now returns an empty string, stripping raw Markdown HTML.

Impact:
Current docs are repo-controlled, and this patch adds containment before future generated or external Markdown can produce script-capable HTML or `javascript:` links.

Fix:
- The builder now strips raw Markdown HTML.
- The builder now accepts only `http:`, `https:`, `mailto:`, relative paths, and `#anchors` for links/images.
- Keep docs input repo-controlled, or add a sanitizer such as DOMPurify with a strict allowlist if future docs need a safe HTML subset.

False positive notes:
This is a build-time/docs pipeline issue, not a runtime app issue.

### S3 - Low - CSP still allows inline styles

Status:
Accepted exception documented in this report.

Location:
- `src-tauri/tauri.conf.json:40`

Evidence:
- CSP includes `style-src 'self' 'unsafe-inline'`.

Impact:
The app currently has no direct DOM XSS sink in `src/`, and the app does not load remote UI. Inline styles still reduce CSP's containment value if a future markup injection bug appears.

Fix:
- Keep this as an explicit accepted exception while Svelte components use inline dynamic styles.
- Longer term, move frequently changing visual state into classes or controlled CSS variables generated from backend/frontend numeric clamps, then tighten CSP when practical.

False positive notes:
This is a hardening item. It is not blocking the current release.

## Positive Controls Confirmed

- `gh api /dependabot/alerts?state=open`: 0 open alerts.
- `npm.cmd audit --json`: 0 vulnerabilities.
- `cargo audit --json`: 0 vulnerabilities.
- `cargo audit` informational warnings remain for transitive unmaintained/unsound crates, mainly GTK3 Linux stack and transitive crates; no RustSec vulnerability is currently active.
- Tauri capabilities expose dialog/core APIs only; shell, fs, and http plugins are not enabled.
- `assetProtocol.scope` is empty in `src-tauri/tauri.conf.json`.
- Frontend scan found no `innerHTML`, `outerHTML`, `insertAdjacentHTML`, Svelte `{@html}`, `document.write`, `eval`, or `new Function` usage under `src/`.
- External links using `target="_blank"` include `rel="noopener"` or `rel="noopener noreferrer"`.
- Managed yt-dlp and FFmpeg downloads use fixed URLs, byte limits, temporary files, and SHA-256 verification before activation.
- Local yt-dlp / FFmpeg trust flow hashes the selected binary and rejects a changed file before trust is saved.
- Subprocess execution uses `Command::new(...).args(...)`; no app runtime shell interpolation was found.
- YouTube URL handling restricts scheme/host and normalizes `http` YouTube URLs to `https`.
- Batch playlist/channel downloads are limited to 25 items in backend args.

## Verification Commands

- `gh api --method GET /repos/himawaril2dev/vocalsync-studio/dependabot/alerts -f state=open --jq 'length'`
- `npm.cmd audit --json`
- `cargo audit --json`
- `rg` scans for DOM XSS sinks, storage, external links, subprocess usage, file I/O, and Tauri commands.

## Recommended Priority

1. Add backend path grants for the remaining audio export and download output-directory commands.
2. Track CSP inline-style cleanup as a low-priority hardening task.
