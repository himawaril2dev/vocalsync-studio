# VocalSync Studio

**繁體中文** | [English](README.en.md) | [日本語](README.ja.md)

一間桌面練唱室，為想練習唱歌、聽見自己聲音的你打造，每次練習都聽得見進步。

VocalSync Studio 是一款桌面應用程式，結合伴奏播放、即時錄音、AI 音高偵測與歌詞同步顯示，幫助歌唱練習者視覺化自己的演唱表現。

📖 **使用說明**：[USER_GUIDE.md](docs/USER_GUIDE.md)｜[English](docs/USER_GUIDE.en.md)｜[日本語](docs/USER_GUIDE.ja.md)（portable zip 內另附離線 HTML 版）

> 📢 **透明度聲明**
> 作者沒有程式開發背景，本專案由 AI（Claude / Codex）協作完成——從架構、程式碼到 UI 全部由 AI 生成。所有功能皆經實測與跨模型 code review（Claude 實作 + Codex 獨立審查）。請依你的使用情境評估風險後採用。

## 功能特色

- **YouTube 下載** — 直接輸入 URL 下載伴奏（自動安裝 yt-dlp + FFmpeg）
- **即時錄音** — 邊聽伴奏邊錄音，支援延遲校準
- **AI 音高偵測** — 使用 CREPE 神經網路模型分析演唱音高（離線運作，不需網路）
- **音高曲線對比** — 將你的演唱與目標旋律並排顯示
- **導唱監聽** — 匯入人聲軌後可低音量跟唱參考，匯出時不混入成品
- **歌詞同步** — 支援 LRC / SRT / VTT 格式，含雙語自動偵測
- **A-B 循環** — 重複練習特定段落
- **變速播放** — WSOLA 時間拉伸，不改變音高

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

> ⚠️ **資料夾結構請勿變動**
> `DirectML.dll`、`yt-dlp.exe`、`models/` 都是 `vocalsync-studio.exe` 執行期會載入的依賴，**不可單獨搬移**到其他位置。若要換目錄，請整個資料夾一起搬。

| 檔案 | 說明 |
|---|---|
| **`vocalsync-studio.exe`** | 主程式 ← 點這個啟動 |
| `DirectML.dll` | ONNX Runtime 的 DirectX ML 加速 DLL（CREPE 音高偵測需要）|
| `yt-dlp.exe` | YouTube 伴奏下載 CLI |
| `models/crepe-tiny.onnx` | CREPE 音高偵測模型 |

### 已測試環境

| 項目 | 版本 |
|---|---|
| OS | Windows 11 Pro 23H2 / 24H2 |
| 架構 | x86_64 |
| WebView2 | 120+（Windows 10/11 預裝）|

macOS / Linux 的 Tauri build 理論上可行（原始碼多平台），但**尚未實測**，也未提供 portable release。若你想在這些平台建置，請走「從原始碼建置」流程，並歡迎回報成果到 Issues。

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

# 4.（可選）產生離線 USER_GUIDE HTML 並打包進 portable zip
npm run build:docs          # 輸出到 dist-docs/*.html
npm run pack:portable-docs  # 複製 HTML 進 portable 資料夾並重新壓縮 zip
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
│       │   ├── wsola.rs             # 時間拉伸
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

## 授權

本專案以 [MIT License](LICENSE) 開源發佈，可自由使用、修改與散佈，無任何擔保。

### 第三方元件授權

| 元件 | 授權 | 在本專案中的用途 |
|---|---|---|
| [CREPE](https://github.com/marl/crepe) / [onnxcrepe v1.1.0](https://github.com/yqzhishen/onnxcrepe) | MIT | AI 音高偵測模型（NYU MARL 開發；ONNX 轉換版本來自 onnxcrepe v1.1.0；[BibTeX 引用](#crepe-論文引用)）|
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) | MIT | 執行 CREPE 模型的推論引擎 |
| [DirectML](https://github.com/microsoft/DirectML) | MIT | Windows 下的 ML 加速層（`DirectML.dll`）|
| [Tauri](https://github.com/tauri-apps/tauri) | MIT / Apache-2.0 | 桌面應用框架 |
| [Svelte](https://github.com/sveltejs/svelte) | MIT | 前端 UI 框架 |
| [Symphonia](https://github.com/pdeljanov/Symphonia) / [cpal](https://github.com/RustAudio/cpal) / [rustfft](https://github.com/ejmahler/RustFFT) / [biquad](https://github.com/korken89/biquad-rs) 等 Rust crate | MIT / Apache-2.0 | 音訊解碼、錄放音、訊號處理 |

### yt-dlp（Unlicense / 公有領域）

[yt-dlp](https://github.com/yt-dlp/yt-dlp) 是 youtube-dl 的社群維護 fork，以 **[The Unlicense](https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE)** 釋出（等同公有領域，CC0-like）。

- **使用方式**：本專案以 **subprocess / CLI 方式**呼叫 `yt-dlp.exe`，不做靜態連結、不修改其原始碼
- **來源**：`yt-dlp.exe` 為從 [yt-dlp 官方 Releases](https://github.com/yt-dlp/yt-dlp/releases) 下載的未修改二進位檔
- **使用者責任**：透過 yt-dlp 下載的內容是否合乎當地法律（著作權、YouTube 服務條款等），由使用者自行負責
- 官方完整授權文字：<https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE>

### FFmpeg（LGPL 2.1+ / 可能為 GPL）

[FFmpeg](https://ffmpeg.org/) 預設以 **[LGPL-2.1-or-later](https://ffmpeg.org/legal.html)** 授權釋出；若啟用特定編碼器（如 libx264、libx265）則需改為 **GPL-2.0-or-later**。

- **使用方式**：本專案以 **subprocess / CLI 方式**呼叫 `ffmpeg` / `ffprobe` 執行檔，不靜態連結 libavcodec / libavformat 等函式庫；因此本專案本體不受 LGPL / GPL 傳染
- **來源**：預設從 [gyan.dev FFmpeg Windows builds](https://www.gyan.dev/ffmpeg/builds/) 或系統既有安裝載入。請使用者自行留意所下載 build 的具體授權版本（essentials build 通常為 LGPL；full build 含 GPL 組件）
- **修改 / 再散佈**：若你要把 FFmpeg binaries 連同本專案一起散佈，需遵守 FFmpeg 的授權條款（附上授權文字、提供原始碼取得方式等）。本倉庫內**未**附 ffmpeg binaries，避免授權衝突
- 官方完整授權文字：<https://ffmpeg.org/legal.html>

### CREPE 論文引用

本專案使用的 CREPE 音高偵測模型源自以下論文：

```bibtex
@inproceedings{kim2018crepe,
  title={{CREPE}: A Convolutional Representation for Pitch Estimation},
  author={Kim, Jong Wook and Salamon, Justin and Li, Peter and Bello, Juan Pablo},
  booktitle={2018 IEEE International Conference on Acoustics, Speech and Signal Processing (ICASSP)},
  pages={161--165},
  year={2018},
  organization={IEEE}
}
```

如果你在學術或商業研究中使用了 VocalSync 的音高偵測成果，請一併引用上述論文與 [onnxcrepe](https://github.com/yqzhishen/onnxcrepe) 的 ONNX 轉換工作。

### 免責聲明

VocalSync Studio 僅為本地練唱輔助工具。使用者透過本工具下載、處理、轉錄的任何音訊內容，其著作權與合法使用責任均歸使用者本人。開發者不對使用者的任何不當使用負責。

## 支持開發

如果這個工具對你的練唱有幫助，歡迎請我喝杯咖啡：

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/himawari168)
