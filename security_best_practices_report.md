# VocalSync Studio Security Review

Review date: 2026-05-08
Reviewed version: v0.2.19
Scope: `src/`, `src-tauri/`, `scripts/`, Tauri config, npm/Rust dependency advisories.

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
