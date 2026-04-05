<p align="center">
  <img src="assets/icon.png" width="120" alt="VocalSync Logo">
</p>

<h1 align="center">VocalSync Studio</h1>

<p align="center">
  卡拉OK 錄音工作室 — 載入伴奏、同步影片、錄製人聲
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Python-3.10%2B-blue">
  <img src="https://img.shields.io/badge/Platform-Windows-lightgrey">
  <img src="https://img.shields.io/badge/License-MIT-green">
</p>

---

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

## 安裝

### 前置需求

- Python 3.10 以上
- [FFmpeg](https://ffmpeg.org/download.html)（需加入系統 PATH）

### 安裝步驟

```bash
git clone https://github.com/himawaril2dev/vocalsync-studio.git
cd vocalsync-studio
pip install -r requirements.txt
python main.py
```

## 打包為 EXE

```bash
pip install pyinstaller
pyinstaller --name "VocalSync Studio" --windowed --icon=assets/icon.ico --add-data "assets;assets" --collect-all cv2 --collect-all sounddevice --collect-all soundfile main.py
```

## 專案結構

```
vocalsync-studio/
├── main.py                 # 應用程式入口
├── requirements.txt
├── make_icon.py            # 圖示產生器
├── assets/
│   ├── icon.png
│   └── icon.ico
├── core/
│   ├── audio_recorder.py   # 錄音核心（播放 / 錄音 / 匯出）
│   ├── video_player.py     # OpenCV 影片播放器
│   ├── downloader.py       # yt-dlp 封裝（伴奏下載）
│   ├── ffmpeg_check.py     # FFmpeg 偵測
│   └── format_helper.py    # 格式選項定義
└── ui/
    ├── theme.py            # 設計系統（色碼 / 字體 / 間距）
    ├── recording_page.py   # 錄音頁面
    └── vu_meter.py         # VU 表元件
```

## 技術棧

| 元件 | 套件 |
|------|------|
| GUI | customtkinter |
| 錄音 / 播放 | sounddevice + soundfile |
| 影片播放 | OpenCV |
| 影像渲染 | Pillow + NumPy |
| 伴奏下載 | yt-dlp |
| 音訊處理 | FFmpeg |

## 相關專案

- [VocalSync Downloader](https://github.com/himawaril2dev/vocalsync-downloader) — YouTube 影片下載器

## 授權

MIT License
