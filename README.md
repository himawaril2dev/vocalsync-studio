<p align="center">
  <img src="assets/icon.png" width="120" alt="VocalSync Logo">
</p>

<h1 align="center">VocalSync Studio</h1>

<p align="center">
  卡拉OK 錄音工作室 — 載入伴奏、同步影片、錄製人聲
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows-lightgrey">
  <img src="https://img.shields.io/badge/License-MIT-green">
  <a href="https://github.com/himawaril2dev/vocalsync-studio/releases/latest">
    <img src="https://img.shields.io/github/v/release/himawaril2dev/vocalsync-studio?label=%E4%B8%8B%E8%BC%89">
  </a>
</p>

---

## 下載

**不需要安裝任何環境，解壓縮後即可使用。**

1. 前往 [Releases](https://github.com/himawaril2dev/vocalsync-studio/releases/latest) 頁面
2. 下載 `VocalSync-Studio-v1.0-win64.zip`
3. 解壓縮到任意資料夾
4. 執行 `VocalSync Studio.exe`

已內建 FFmpeg 與粉圓字體，不需額外安裝。

## 功能

- 支援 YouTube 下載或本機匯入伴奏（音訊 / 影片）
- 影片同步播放，自動保持原始比例
- 即時 VU 表顯示（伴奏 + 麥克風）
- 伴奏音量調整（0% ~ 200%）
- 試聽 / 錄音 / 回放，支援暫停 / 繼續 / 任意位置跳轉
- 自由選擇錄音裝置與輸出裝置
- 導出三軌檔案：
  - `*_vocal.wav` — 純人聲
  - `*_backing.wav` — 純伴奏
  - `*_multitrack.wav` — 3 聲道（可匯入 DAW 混音）

## 截圖

> *（待補充）*

## 從原始碼執行

如果你想自行修改或開發，可以從原始碼執行：

### 前置需求

- Python 3.10 以上
- [FFmpeg](https://ffmpeg.org/download.html)（需加入系統 PATH）

### 步驟

```bash
git clone https://github.com/himawaril2dev/vocalsync-studio.git
cd vocalsync-studio
pip install -r requirements.txt
python main.py
```

### 自行打包為 EXE

```bash
pip install pyinstaller
pyinstaller "VocalSync Studio.spec"
```

產出位於 `dist/VocalSync Studio/`。

> 注意：自行打包前需先將 FFmpeg 執行檔放入 `assets/ffmpeg/` 目錄。

## 專案結構

```
vocalsync-studio/
├── main.py                     # 應用程式入口
├── requirements.txt
├── VocalSync Studio.spec       # PyInstaller 打包設定
├── make_icon.py                # 圖示產生器
├── assets/
│   ├── icon.png / icon.ico     # 向日葵圖示
│   ├── jf-openhuninn-2.1.ttf   # 粉圓字體（免安裝載入）
│   └── ffmpeg/                 # FFmpeg 執行檔（不納入 git）
├── core/
│   ├── audio_recorder.py       # 錄音核心（播放 / 錄音 / 匯出）
│   ├── video_player.py         # OpenCV 影片播放器
│   ├── downloader.py           # yt-dlp 封裝（伴奏下載）
│   ├── ffmpeg_check.py         # FFmpeg 路徑偵測
│   └── format_helper.py        # 格式選項定義
└── ui/
    ├── theme.py                # 設計系統（色碼 / 字體 / 間距）
    ├── recording_page.py       # 錄音頁面
    └── vu_meter.py             # VU 表元件
```

## 技術棧

| 元件 | 套件 |
|------|------|
| GUI | customtkinter |
| 錄音 / 播放 | sounddevice + soundfile |
| 影片播放 | OpenCV |
| 影像渲染 | Pillow + NumPy |
| 伴奏下載 | yt-dlp |
| 音訊處理 | FFmpeg（內建） |
| 字體 | jf open 粉圓 2.1（內建） |

## 相關專案

- [VocalSync Downloader](https://github.com/himawaril2dev/vocalsync-downloader) — YouTube 影片下載器

## 授權

MIT License
