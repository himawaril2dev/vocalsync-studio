# VocalSync Studio

練唱輔助工具，讓每一次練習都聽得見進步。

VocalSync Studio 是一款桌面應用程式，結合伴奏播放、即時錄音、AI 音高偵測與歌詞同步顯示，幫助歌唱練習者視覺化自己的演唱表現。

> 100% AI-Crafted — 從架構設計、前後端程式碼到 UI，全程由 AI 生成。

## 功能特色

- **YouTube 下載** — 直接輸入 URL 下載伴奏（自動安裝 yt-dlp + FFmpeg）
- **即時錄音** — 邊聽伴奏邊錄音，支援延遲校準
- **AI 音高偵測** — 使用 CREPE 神經網路模型分析演唱音高（離線運作，不需網路）
- **音高曲線對比** — 將你的演唱與目標旋律並排顯示
- **歌詞同步** — 支援 LRC / SRT / VTT 格式，含雙語自動偵測
- **MIDI 旋律載入** — 匯入 MIDI 作為音高參考線
- **調性偵測** — 自動分析伴奏調性
- **A-B 循環** — 重複練習特定段落
- **變速播放** — WSOLA 時間拉伸，不改變音高
- **快速消人聲** — 立體聲 center-cancel 去除原唱

## 技術架構

| 層級 | 技術 |
|------|------|
| 前端 | Svelte 5 + TypeScript + Vite |
| 後端 | Rust + Tauri v2 |
| 音訊 | cpal (錄放音) + symphonia (解碼) + biquad (濾波) |
| 音高偵測 | CREPE tiny (ONNX Runtime) + PYIN (傳統演算法) |
| 訊號處理 | rustfft (FFT) + WSOLA (時間拉伸) |
| 下載 | yt-dlp CLI wrapper + SHA-256 供應鏈驗證 |

## 安裝

### 從 Release 下載（推薦）

1. 前往 [Releases](https://github.com/himawaril2dev/vocalsync-studio/releases) 頁面，下載最新的 `VocalSync.Studio.Portable.x.y.z.zip`
2. 解壓縮到任意位置（例如桌面或 `D:\Tools\`）
3. 進入資料夾，雙擊 **`vocalsync-studio.exe`** 即可啟動

資料夾內其他檔案是依賴，請保留不要移動：

| 檔案 | 說明 |
|---|---|
| **`vocalsync-studio.exe`** | 主程式 ← 點這個啟動 |
| `DirectML.dll` | ONNX Runtime 的 DirectX ML 加速 DLL（CREPE 音高偵測需要）|
| `yt-dlp.exe` | YouTube 伴奏下載 CLI |
| `models/crepe-tiny.onnx` | CREPE 音高偵測模型 |

> **第一次執行的 Windows SmartScreen 警告**
> 因為目前還沒有做 code-signing 數位簽章，Windows SmartScreen 可能會跳出「已防止 Windows 保護您的電腦」的警告。
> 點警告視窗左上的 **「其他資訊」**，再按下方出現的 **「仍要執行」** 按鈕即可啟動。
> 之後同一個 exe 不會再跳此警告。
> 若對來源仍有疑慮，可以在 Release 頁下載 zip 後用 `certutil -hashfile "檔名.zip" SHA256` 比對 GitHub 上顯示的 digest。

### 從原始碼建置

**前置需求：**
- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://rustup.rs/) >= 1.77
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

```bash
# 1. 安裝前端依賴
npm install

# 2. 開發模式
npm run tauri dev

# 3. 建置 Release
npm run tauri build
```

建置產物在 `src-tauri/target/release/bundle/` 中。

## 專案結構

```
vocalsync-studio-tauri/
├── src/                    # Svelte 前端
│   ├── components/         # UI 元件（音高曲線、歌詞面板、校準器...）
│   ├── stores/             # 狀態管理（播放、錄音、歌詞、設定...）
│   └── tabs/               # 頁面（準備、錄音、音高、關於）
├── src-tauri/
│   └── src/
│       ├── commands/       # Tauri IPC 指令
│       ├── core/           # 核心引擎
│       │   ├── audio_engine.rs      # 錄放音引擎
│       │   ├── crepe_engine.rs      # CREPE AI 音高偵測
│       │   ├── pyin_engine.rs       # PYIN 傳統音高偵測
│       │   ├── lyrics_parser.rs     # LRC/SRT/VTT 解析
│       │   ├── midi_parser.rs       # MIDI 解析
│       │   ├── wsola.rs             # 時間拉伸
│       │   ├── key_detector.rs      # 調性偵測
│       │   ├── ytdlp_engine.rs      # YouTube 下載
│       │   └── ...
│       └── lib.rs          # Tauri 入口
└── package.json
```

## 快捷鍵

| 按鍵 | 功能 |
|------|------|
| `Space` | 播放 / 暫停 |
| `R` | 開始錄音 |
| `Esc` | 停止 |
| `A` | 設定循環 A 點 |
| `B` | 設定循環 B 點 |
| `+` | 升半音 |
| `-` | 降半音 |

## 問題回報

使用遇到問題、想回報 bug 或有功能建議都很歡迎，可以透過以下兩種方式聯絡：

- **GitHub Issues**：[vocalsync-studio/issues](https://github.com/himawaril2dev/vocalsync-studio/issues)
- **電子信箱**：`himawaril2dev@gmail.com`

回報時若能附上：作業系統版本、VocalSync Studio 版本、操作步驟與錯誤訊息截圖，會更容易定位問題。

## 版本更新

### v0.2.0（2026-04-19）

**新增功能**
- 錄音「暫停 / 繼續」按鈕：試聽與錄音都可暫停，暫停時可拖動進度條從目標位置續播 / 續錄（向前 seek 會跳確認對話框，避免誤觸）
- 停止按鈕固定回到開頭，4 顆 Transport 按鈕統一 36×36 尺寸
- 匯出自動平衡：以 RMS 比例讓人聲主導（係數 1.5，無上限鉗制），避免伴奏蓋過人聲
- 「標準化」選項旁加上 tooltip 說明（滑鼠移到驚嘆號上顯示）
- 伴奏 / 人聲 / 速度 / 移調皆提供獨立的 ↺ 重設按鈕

**安全性強化**
- 新增 `security` 模組：所有接收路徑的 Tauri command 都會做 path traversal / NUL / dash-prefix / 絕對路徑 / `..` 檢查
- FFmpeg / FFprobe 參數注入防線，yt-dlp 擋空白字元 URL
- CSP 收緊：新增 `object-src 'none'`、`base-uri 'self'`、`frame-ancestors 'none'`、`form-action 'self'`，並移除未使用的 `api.github.com` connect-src
- Asset 協定 scope 拔掉 `$HOME/**`，只保留 Downloads / Desktop / Documents / Videos / Music / AppData / Temp
- 移除前端未使用的 `shell:allow-open` 權限

**錯誤修復與體驗調整**
- 載入新伴奏時自動清空上一首的錄音緩衝與即時音高樣本，避免跨歌殘影
- 關閉 `USE_PRODUCER_PATH` 停用中的 producer thread，減少多餘 CPU 與潛在競態
- 修掉 RecordingTab 的 a11y warning：tooltip 觸發元素改用 `<button>`

## 授權

本專案以 [MIT License](LICENSE) 開源發佈。

CREPE 音高偵測模型由 NYU 開發，同樣以 MIT License 授權。
yt-dlp 與 FFmpeg 為獨立的第三方工具，各有其授權條款。

## 支持開發

如果這個工具對你的練唱有幫助，歡迎請我喝杯咖啡：

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/himawari168)
