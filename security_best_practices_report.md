# VocalSync Studio 安全性檢查報告

檢查日期：2026-04-28  
專案版本：0.2.13  
範圍：`src/`、`src-tauri/`、`scripts/`、Tauri capability、npm/Rust 依賴

## 結論

本輪未發現 P0/P1 等級的使用者安全問題。外部工具信任、yt-dlp / FFmpeg 下載驗證、YouTube URL allowlist、Tauri 權限與前端 XSS sink 目前都有明確防線。

2026-04-28 追加修復：4 個 P2/P3 findings 已處理。下載任務已加入後端單工鎖，playlist/channel 已加入使用者確認與後端項目上限，Tauri asset protocol 已收窄，未使用的完整設定寫入 command 已移除。HTTP YouTube URL 會在後端自動升級成 HTTPS 後再交給 yt-dlp。

## Findings

### P2：播放清單與頻道下載缺少後端數量上限（已修復）

位置：
- `src-tauri/src/core/ytdlp_engine.rs:1412`
- `src-tauri/src/core/ytdlp_engine.rs:1477`
- `src/tabs/DownloadTab.svelte:134`

證據：
- `detect_url_type` 會辨識 playlist / channel。
- `build_args` 對非單支影片只加上 `--ignore-errors`。
- 前端開始下載時直接呼叫 `start_download`，沒有 metadata preview、項目數量上限或二次確認。

影響：使用者貼到頻道或大型播放清單時，可能長時間下載、大量占用磁碟與網路。若 renderer 未來出現 XSS，也會放大成資源消耗攻擊面。

建議修復：
- 預設加入 `--playlist-end` 或 `--max-downloads`。
- 對 playlist / channel 先做 metadata preview，顯示項目數與輸出位置後再確認。
- UI 允許使用者設定最大下載數，後端也要 enforce。

修復狀態：
- 後端對 playlist/channel 加入 `--playlist-end 25`。
- 前端在 playlist/channel 下載前顯示確認訊息。
- `ToolStatus` 回傳 `batch_download_limit`，避免 UI 文案與後端限制漂移。

### P2：下載任務缺少後端單工鎖（已修復）

位置：
- `src-tauri/src/commands/download_commands.rs:68`
- `src-tauri/src/core/ytdlp_engine.rs:1487`
- `src/stores/download.ts:74`

證據：
- 前端用 `isDownloading` 控制按鈕狀態。
- 後端 `start_download` 每次呼叫都會 `spawn_blocking` 啟動一個 yt-dlp subprocess。
- `DownloadCancelFlag` 是全域旗標，沒有 per-task id 或 running mutex。

影響：IPC 重送、未來新增第二個入口、或 renderer compromise 時，可同時啟動多個下載。結果會造成 CPU / 網路 / 磁碟壓力，取消按鈕也會變成全域取消，狀態容易互相覆蓋。

建議修復：
- 後端新增 `DownloadRunLock` 或 `AtomicBool`。
- `start_download` 進入時用 `compare_exchange(false, true)`，已有任務時直接回傳「下載已在進行中」。
- 任務完成、錯誤或取消時用 guard 自動釋放。

修復狀態：
- 新增 `DownloadRunFlag` 與 `DownloadRunGuard`。
- `start_download` 在後端以 `AtomicBool::compare_exchange` 保證同時間只跑一個 yt-dlp 任務。
- 已有任務時直接回傳錯誤，不會重設目前任務的取消旗標。

### P3：assetProtocol scope 偏寬（已修復）

位置：
- `src-tauri/tauri.conf.json:48`
- `src-tauri/tauri.conf.json:50`

證據：
- 修復前 asset protocol 允許 `$DOWNLOAD/**`、`$DESKTOP/**`、`$DOCUMENT/**`、`$VIDEO/**`、`$AUDIO/**`、`$APPDATA/**`、`$APPLOCALDATA/**`、`$TEMP/**`。
- 目前前端只在 `src/tabs/SetupTab.svelte:253` 對載入的影片路徑呼叫 `convertFileSrc`。

影響：目前沒有看到前端 XSS sink，所以實際風險低。未來若出現 renderer compromise，較寬的 asset scope 會增加本機檔案被 WebView 讀取或展示的範圍。

建議修復：
- 移除目前不需要的 `$APPDATA/**`、`$APPLOCALDATA/**`、`$TEMP/**`。
- 優先改成只服務使用者選過的媒體檔案，或建立後端 allowlist token。

修復狀態：
- 已移除 `$APPDATA/**`、`$APPLOCALDATA/**`、`$TEMP/**`。
- 保留常用媒體來源：Downloads、Desktop、Documents、Video、Audio。

### P3：未使用的 `save_settings` command 增加攻擊面（已修復）

位置：
- `src-tauri/src/commands/settings_commands.rs:17`
- `src-tauri/src/lib.rs:66`

證據：
- 前端沒有呼叫 `save_settings`。
- 修復前後端仍將 `save_settings` 暴露在 invoke handler。
- 這個 command 接收完整 `AppSettings` 並直接覆蓋設定檔。

影響：目前下游重要路徑仍有額外驗證，實際風險低。保留未使用的完整設定寫入入口會增加 renderer compromise 後可持久化的狀態範圍。

建議修復：
- 從 invoke handler 移除 `save_settings`。
- 保留目前較窄的 `update_pitch_engine`、`update_calibrated_latency`。
- 若未來要恢復完整設定儲存，先做欄位 allowlist、範圍 clamp 與路徑驗證。

修復狀態：
- 已從 Tauri invoke handler 移除 `save_settings`。
- 已刪除後端完整設定覆蓋 command。

## 已確認的安全控制

- 前端未找到 `innerHTML`、`outerHTML`、`insertAdjacentHTML`、`document.write`、`eval`、`new Function`、Svelte `{@html}`。
- 外部連結使用 `target="_blank"` 時都有 `rel="noopener"`。
- Tauri capability 沒有開 shell plugin、fs plugin、http plugin。
- CSP 包含 `object-src 'none'`、`base-uri 'self'`、`frame-ancestors 'none'`、`form-action 'self'`。
- yt-dlp / FFmpeg managed download 使用固定 URL、SHA-256 驗證與下載大小上限。
- 本機 yt-dlp / FFmpeg 偵測階段只計算 SHA-256，不直接執行；信任後會綁定路徑與 hash。
- YouTube URL 後端驗證包含長度限制、NUL/空白字元阻擋、scheme 限制與 host allowlist。
- `http://` YouTube URL 會 normalize 成 `https://`，避免下載流程使用明文 HTTP。
- subprocess 呼叫使用 `Command::new(...).args(...)`，沒有 shell 字串拼接執行下載命令。

## 驗證結果

- `npm run build`：通過。
- `cargo test --quiet`：170 tests 通過。
- `git diff --check`：通過。
- `npm audit --omit=dev`：0 vulnerabilities。
- `cargo audit`：無直接 vulnerability；有 Tauri/Linux GTK 相關 transitive unmaintained warnings，以及 `glib`、`rand` transitive unsound warnings。Windows portable release 主要風險較低，建議持續追 Tauri / wry 更新。
- `cargo clippy --quiet -- -D warnings`：未通過，失敗點集中在既有 `audio_engine.rs`、`crepe_engine.rs`、`melody_extractor.rs`、`pyin_engine.rs`、`settings.rs`、`wsola.rs` warning，與本次安全修補檔案無關。

## Release 判定

狀態：READY

0.2.13 的外部工具安全修復仍有效。本報告列出的 4 個 findings 已完成修復，前端 build 與 Rust tests 已通過。
