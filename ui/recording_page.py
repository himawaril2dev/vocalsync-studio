"""
錄音頁面 UI。
功能：YouTube 下載 / 本機匯入伴奏、嵌入影片、音量調整、裝置選擇、VU 表、可拖動進度條、導出三檔案。
"""

import os
import subprocess
import tkinter as tk
from datetime import datetime
from tkinter import filedialog, messagebox

import customtkinter as ctk
import sounddevice as sd

from core.audio_recorder import AudioRecorder
from core.downloader import start_download_thread
from core.ffmpeg_check import find_ffmpeg, is_ffmpeg_available
from core.video_player import VideoPlayer
from ui import theme as T
from ui.vu_meter import VUMeter

_VIDEO_W, _VIDEO_H = 640, 360
_AUDIO_EXTS = {".wav", ".mp3", ".m4a", ".flac", ".ogg", ".aac"}
_VIDEO_EXTS = {".mp4", ".mkv", ".avi", ".mov", ".webm"}


def _fmt_time(sec: float) -> str:
    sec = max(0.0, sec)
    return f"{int(sec)//60:02d}:{int(sec)%60:02d}"


def _get_input_devices() -> list[tuple[int, str]]:
    """回傳所有輸入裝置的 (index, 名稱) 清單。"""
    result = []
    for i, d in enumerate(sd.query_devices()):
        if d["max_input_channels"] > 0:
            result.append((i, d["name"]))
    return result


def _get_output_devices() -> list[tuple[int, str]]:
    """回傳所有輸出裝置的 (index, 名稱) 清單。"""
    result = []
    for i, d in enumerate(sd.query_devices()):
        if d["max_output_channels"] > 0:
            result.append((i, d["name"]))
    return result


class RecordingPage(ctk.CTkFrame):

    def __init__(self, parent, settings: dict):
        super().__init__(parent, fg_color="transparent")
        self._settings = settings
        self._recorder = AudioRecorder()
        self._recorder.volume = 0.05   # 預設 5%
        self._video_player: VideoPlayer | None = None
        self._video_path: str | None = None
        self._is_downloading = False
        self._seek_dragging = False   # 使用者正在拖動進度條
        self._preview_state: str = "idle"   # "idle" | "playing" | "paused"
        self._preview_paused_frame: int = 0
        self._resize_after_id: str | None = None
        self._export_folder = settings.get(
            "download_folder", os.path.expanduser("~/Downloads/YouTube"))

        # 裝置清單
        self._input_devices = _get_input_devices()
        self._output_devices = _get_output_devices()
        self._selected_input_idx: int | None = None   # None = 系統預設
        self._selected_output_idx: int | None = None  # None = 系統預設

        self._build_ui()
        self.bind("<Configure>", self._on_resize)
        self._start_level_poll()

    # ------------------------------------------------------------------ #
    #  UI 建構                                                             #
    # ------------------------------------------------------------------ #

    def _build_ui(self):
        p = T.PAD_PAGE_REC
        cy = T.PAD_CARD_Y_REC
        self.grid_columnconfigure(0, weight=1)
        self.grid_rowconfigure(2, weight=1)  # 影片區可拉伸

        compact = dict(
            fg_color=T.SECONDARY, hover_color=T.SECONDARY_HOVER,
            text_color=T.TEXT_PRIMARY, corner_radius=6, height=32,
            font=ctk.CTkFont(family=T.FONT_DISPLAY, size=12),
        )

        # ── 頂部：伴奏來源 ───────────────────────────────────────────
        src_card = ctk.CTkFrame(self, corner_radius=T.CARD_RADIUS, fg_color=T.SURFACE)
        src_card.grid(row=0, column=0, padx=p, pady=(12, cy), sticky="ew")
        src_card.grid_columnconfigure(0, weight=1)

        title_row = ctk.CTkFrame(src_card, fg_color="transparent")
        title_row.grid(row=0, column=0, padx=14, pady=(12, 6), sticky="ew")
        title_row.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(title_row, text="伴奏來源",
                     font=ctk.CTkFont(*T.H3), text_color=T.TEXT_PRIMARY).grid(
            row=0, column=0, sticky="w")

        # YouTube 下載列
        yt_row = ctk.CTkFrame(src_card, fg_color="transparent")
        yt_row.grid(row=1, column=0, padx=14, pady=(0, 4), sticky="ew")
        yt_row.grid_columnconfigure(1, weight=1)

        ctk.CTkLabel(yt_row, text="YouTube",
                     font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY,
                     width=68).grid(row=0, column=0, sticky="w")

        self._url_entry = ctk.CTkEntry(
            yt_row, placeholder_text="貼上 YouTube 網址...", height=32,
            font=ctk.CTkFont(*T.BODY2))
        self._url_entry.grid(row=0, column=1, sticky="ew", padx=(0, 6))
        self._url_entry.bind("<Button-3>", self._show_ctx_menu)

        ctk.CTkButton(yt_row, text="貼上", width=48, command=self._paste_url,
                      **compact).grid(row=0, column=2, padx=(0, 6))
        self._dl_btn = ctk.CTkButton(
            yt_row, text="下載", width=72, command=self._on_download, **compact)
        self._dl_btn.grid(row=0, column=3)

        # 本機匯入列
        local_row = ctk.CTkFrame(src_card, fg_color="transparent")
        local_row.grid(row=2, column=0, padx=14, pady=(0, 10), sticky="ew")
        local_row.grid_columnconfigure(1, weight=1)

        ctk.CTkLabel(local_row, text="本機檔案",
                     font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY,
                     width=68).grid(row=0, column=0, sticky="w")

        self._local_label = ctk.CTkLabel(
            local_row, text="尚未選擇",
            text_color=T.TEXT_MUTED, anchor="w",
            font=ctk.CTkFont(*T.CAPTION))
        self._local_label.grid(row=0, column=1, sticky="ew", padx=(0, 6))

        ctk.CTkButton(
            local_row, text="瀏覽…", width=72,
            command=self._on_browse_local, **compact).grid(row=0, column=2)

        # 狀態列
        self._src_label = ctk.CTkLabel(
            src_card, text="", font=ctk.CTkFont(*T.CAPTION),
            text_color=T.TEXT_SECONDARY)
        self._src_label.grid(row=3, column=0, padx=14, pady=(0, 10), sticky="w")

        # ── 裝置選擇列 ───────────────────────────────────────────────
        dev_card = ctk.CTkFrame(self, corner_radius=T.CARD_RADIUS, fg_color=T.SURFACE)
        dev_card.grid(row=1, column=0, padx=p, pady=(0, cy), sticky="ew")
        dev_card.grid_columnconfigure((1, 3), weight=1)

        ctk.CTkLabel(dev_card, text="裝置",
                     font=ctk.CTkFont(*T.H3), text_color=T.TEXT_PRIMARY).grid(
            row=0, column=0, columnspan=4, padx=14, pady=(10, 6), sticky="w")

        ctk.CTkLabel(dev_card, text="🎙 錄音來源",
                     font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY).grid(
            row=1, column=0, padx=(14, 6), pady=(0, 10), sticky="w")

        in_names = ["系統預設"] + [d[1][:40] for d in self._input_devices]
        self._input_var = tk.StringVar(value="系統預設")
        self._input_menu = ctk.CTkOptionMenu(
            dev_card, values=in_names, variable=self._input_var,
            height=30, command=self._on_input_change,
            font=ctk.CTkFont(*T.CAPTION), dropdown_font=ctk.CTkFont(*T.CAPTION))
        self._input_menu.grid(row=1, column=1, padx=(0, 16), pady=(0, 10), sticky="ew")

        ctk.CTkLabel(dev_card, text="🔊 伴奏輸出",
                     font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY).grid(
            row=1, column=2, padx=(0, 6), pady=(0, 10), sticky="w")

        out_names = ["系統預設"] + [d[1][:40] for d in self._output_devices]
        self._output_var = tk.StringVar(value="系統預設")
        self._output_menu = ctk.CTkOptionMenu(
            dev_card, values=out_names, variable=self._output_var,
            height=30, command=self._on_output_change,
            font=ctk.CTkFont(*T.CAPTION), dropdown_font=ctk.CTkFont(*T.CAPTION))
        self._output_menu.grid(row=1, column=3, padx=(0, 14), pady=(0, 10), sticky="ew")

        # ── 影片顯示區 ───────────────────────────────────────────────
        self._video_frame = ctk.CTkFrame(
            self, fg_color=T.VIDEO_BG, corner_radius=8)
        self._video_frame.grid(row=2, column=0, padx=p, pady=(0, cy), sticky="nsew")
        self._video_frame.grid_rowconfigure(0, weight=1)
        self._video_frame.grid_columnconfigure(0, weight=1)

        self._video_label = ctk.CTkLabel(
            self._video_frame,
            text="📽  下載或匯入伴奏後，畫面會顯示在此",
            font=ctk.CTkFont(*T.BODY1), text_color=T.TEXT_MUTED,
            width=_VIDEO_W, height=_VIDEO_H, image=None)
        self._video_label.grid(row=0, column=0)

        # ── VU 表區 ──────────────────────────────────────────────────
        vu_card = ctk.CTkFrame(self, corner_radius=T.CARD_RADIUS, fg_color=T.SURFACE)
        vu_card.grid(row=3, column=0, padx=p, pady=(0, 4), sticky="ew")
        vu_card.grid_columnconfigure(1, weight=1)
        vu_card.grid_columnconfigure(3, weight=1)

        ctk.CTkLabel(vu_card, text="伴奏",
                     font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY,
                     width=38).grid(row=0, column=0, padx=(12, 6), pady=8)
        self._vu_backing = VUMeter(vu_card, boost=2.0, height=16)
        self._vu_backing.grid(row=0, column=1, padx=(0, 14), pady=8, sticky="ew")

        ctk.CTkLabel(vu_card, text="麥克風",
                     font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY,
                     width=46).grid(row=0, column=2, padx=(0, 6), pady=8)
        self._vu_mic = VUMeter(vu_card, boost=4.0, height=16)
        self._vu_mic.grid(row=0, column=3, padx=(0, 12), pady=8, sticky="ew")

        # ── 音量 + 錄音控制 ──────────────────────────────────────────
        ctrl_card = ctk.CTkFrame(self, corner_radius=T.CARD_RADIUS, fg_color=T.SURFACE)
        ctrl_card.grid(row=4, column=0, padx=p, pady=(0, cy), sticky="ew")
        ctrl_card.grid_columnconfigure(4, weight=1)

        # 音量滑桿
        ctk.CTkLabel(ctrl_card, text="🔊",
                     font=ctk.CTkFont(family=T.FONT_DISPLAY, size=14)).grid(
            row=0, column=0, padx=(12, 4), pady=10)
        self._vol_slider = ctk.CTkSlider(
            ctrl_card, from_=0, to=200, number_of_steps=200,
            width=140, command=self._on_volume_change)
        self._vol_slider.set(5)
        self._vol_slider.grid(row=0, column=1, padx=(0, 4), pady=10)
        self._vol_label = ctk.CTkLabel(
            ctrl_card, text="5%", font=ctk.CTkFont(*T.CAPTION), width=38)
        self._vol_label.grid(row=0, column=2, padx=(0, 14))

        # 分隔線
        ctk.CTkFrame(ctrl_card, width=1, height=26,
                     fg_color=T.BORDER).grid(row=0, column=3, padx=4)

        # 控制按鈕
        ctrl_kw = dict(
            height=36, corner_radius=6,
            fg_color=T.SECONDARY, hover_color=T.SECONDARY_HOVER,
            text_color=T.TEXT_PRIMARY,
            font=ctk.CTkFont(family=T.FONT_DISPLAY, size=12),
        )

        self._preview_btn = ctk.CTkButton(
            ctrl_card, text="▶ 試聽", width=80,
            state="disabled", command=self._on_preview, **ctrl_kw)
        self._preview_btn.grid(row=0, column=4, padx=(6, 4), pady=8, sticky="e")

        self._rec_btn = ctk.CTkButton(
            ctrl_card, text="● 錄音", width=90,
            fg_color=T.ACCENT_RED, hover_color=T.ACCENT_RED_HOVER,
            text_color=("#ffffff", "#ffffff"), corner_radius=6, height=36,
            font=ctk.CTkFont(family=T.FONT_DISPLAY, size=12, weight="bold"),
            state="disabled", command=self._on_start_record)
        self._rec_btn.grid(row=0, column=5, padx=4, pady=8)

        self._stop_btn = ctk.CTkButton(
            ctrl_card, text="■ 停止", width=80,
            state="disabled", command=self._on_stop, **ctrl_kw)
        self._stop_btn.grid(row=0, column=6, padx=4, pady=8)

        self._play_btn = ctk.CTkButton(
            ctrl_card, text="▶ 回放", width=80,
            state="disabled", command=self._on_playback, **ctrl_kw)
        self._play_btn.grid(row=0, column=7, padx=4, pady=8)

        self._export_btn = ctk.CTkButton(
            ctrl_card, text="📦 導出", width=80,
            fg_color=T.PRIMARY, hover_color=T.PRIMARY_HOVER,
            text_color=("#ffffff", "#ffffff"), corner_radius=6, height=36,
            font=ctk.CTkFont(family=T.FONT_DISPLAY, size=12),
            state="disabled", command=self._on_export)
        self._export_btn.grid(row=0, column=8, padx=(4, 12), pady=8)

        # ── 進度條（可拖動）+ 導出路徑 ──────────────────────────────
        bottom = ctk.CTkFrame(self, fg_color="transparent")
        bottom.grid(row=5, column=0, padx=p, pady=(0, 10), sticky="ew")
        bottom.grid_columnconfigure(0, weight=1)

        prog_row = ctk.CTkFrame(bottom, fg_color="transparent")
        prog_row.grid(row=0, column=0, sticky="ew")
        prog_row.grid_columnconfigure(0, weight=1)

        self._seek_slider = ctk.CTkSlider(
            prog_row, from_=0, to=1, number_of_steps=1000,
            height=14, button_length=10)
        self._seek_slider.set(0)
        self._seek_slider.grid(row=0, column=0, sticky="ew", padx=(0, 8))
        self._seek_slider.bind("<ButtonPress-1>", self._on_seek_press)
        self._seek_slider.bind("<ButtonRelease-1>", self._on_seek_release)

        self._time_label = ctk.CTkLabel(
            prog_row, text="00:00 / 00:00",
            font=ctk.CTkFont(*T.MONO_S), text_color=T.TEXT_SECONDARY, width=90)
        self._time_label.grid(row=0, column=1)

        folder_row = ctk.CTkFrame(bottom, fg_color="transparent")
        folder_row.grid(row=1, column=0, pady=(4, 0), sticky="ew")
        folder_row.grid_columnconfigure(0, weight=1)

        self._folder_label = ctk.CTkLabel(
            folder_row, text=self._export_folder,
            text_color=T.TEXT_MUTED, anchor="w",
            font=ctk.CTkFont(*T.TINY))
        self._folder_label.grid(row=0, column=0, sticky="ew", padx=(0, 6))

        ctk.CTkButton(
            folder_row, text="選擇導出位置", width=110, height=26,
            fg_color=T.SECONDARY, hover_color=T.SECONDARY_HOVER,
            text_color=T.TEXT_PRIMARY, corner_radius=6,
            font=ctk.CTkFont(*T.CAPTION),
            command=self._choose_folder).grid(row=0, column=1)

    # ------------------------------------------------------------------ #
    #  視窗縮放                                                            #
    # ------------------------------------------------------------------ #

    def _on_resize(self, event):
        if not self._video_player:
            return
        # debounce：150ms 內若再觸發就取消重排程，避免拖動時大量重繪
        if self._resize_after_id:
            self.after_cancel(self._resize_after_id)
        self._resize_after_id = self.after(150, self._do_resize)

    def _do_resize(self):
        self._resize_after_id = None
        if not self._video_player:
            return
        fw = self._video_frame.winfo_width() - 4
        fh = self._video_frame.winfo_height() - 4
        if fw > 100 and fh > 60:
            self._video_label.configure(width=fw, height=fh)
            self._video_player.set_display_size(fw, fh)

    # ------------------------------------------------------------------ #
    #  裝置選擇                                                            #
    # ------------------------------------------------------------------ #

    def _on_input_change(self, name: str):
        if name == "系統預設":
            self._selected_input_idx = None
        else:
            for idx, n in self._input_devices:
                if n[:40] == name:
                    self._selected_input_idx = idx
                    break

    def _on_output_change(self, name: str):
        if name == "系統預設":
            self._selected_output_idx = None
        else:
            for idx, n in self._output_devices:
                if n[:40] == name:
                    self._selected_output_idx = idx
                    break

    # ------------------------------------------------------------------ #
    #  YouTube 下載                                                        #
    # ------------------------------------------------------------------ #

    def _on_download(self):
        if self._is_downloading:
            return
        url = self._url_entry.get().strip()
        if not url:
            messagebox.showwarning("提示", "請先輸入 YouTube 網址")
            return
        if not is_ffmpeg_available():
            messagebox.showerror("缺少 FFmpeg", "需要 FFmpeg，請先在設定頁面確認。")
            return

        self._is_downloading = True
        self._dl_btn.configure(state="disabled")
        self._src_label.configure(text="下載中，請稍候...", text_color="gray")

        # 伴奏存到 backing_tracks 子資料夾
        save_dir = os.path.join(self._export_folder, "backing_tracks")
        os.makedirs(save_dir, exist_ok=True)

        ydl_opts = {
            "format": "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best",
            "outtmpl": os.path.join(save_dir, "%(title)s.%(ext)s"),
            "merge_output_format": "mp4",
            "quiet": True,
            "ignoreerrors": True,
            "progress_hooks": [],
        }

        def on_complete():
            mp4s = [os.path.join(save_dir, f)
                    for f in os.listdir(save_dir)
                    if f.lower().endswith(".mp4")]
            if not mp4s:
                self.after(0, lambda: self._src_fail("找不到下載的影片"))
                return
            # 取最新檔案
            latest = max(mp4s, key=os.path.getmtime)
            self.after(0, lambda: self._load_video_file(latest))

        def on_error(msg):
            self.after(0, lambda: self._src_fail(msg))

        start_download_thread(url, ydl_opts, on_error=on_error, on_complete=on_complete)

    # ------------------------------------------------------------------ #
    #  本機匯入                                                            #
    # ------------------------------------------------------------------ #

    def _on_browse_local(self):
        exts = " ".join(f"*{e}" for e in sorted(_AUDIO_EXTS | _VIDEO_EXTS))
        path = filedialog.askopenfilename(
            title="選擇伴奏檔案",
            filetypes=[("影音檔案", exts),
                       ("影片", "*.mp4 *.mkv *.avi *.mov *.webm"),
                       ("音訊", "*.wav *.mp3 *.m4a *.flac *.ogg *.aac"),
                       ("所有檔案", "*.*")])
        if not path:
            return
        short = os.path.basename(path)[:50]
        self._local_label.configure(text=short, text_color=("gray30", "gray70"))
        self._src_label.configure(text="載入中...", text_color="gray")
        self.after(50, lambda: self._load_local_file(path))

    def _load_local_file(self, path: str):
        ext = os.path.splitext(path)[1].lower()
        if ext in _VIDEO_EXTS:
            self._load_video_file(path)
        elif ext in _AUDIO_EXTS:
            self._load_audio_file(path)
        else:
            self._src_fail(f"不支援的格式：{ext}")

    def _load_audio_file(self, path: str):
        """直接載入音訊檔（無影片）。"""
        # 若非 WAV，先用 ffmpeg 轉換
        if not path.lower().endswith(".wav"):
            wav_path = path + "_converted.wav"
            ffmpeg = find_ffmpeg()
            if ffmpeg:
                try:
                    subprocess.run(
                        [ffmpeg, "-y", "-i", path,
                         "-acodec", "pcm_s16le", "-ar", "44100", "-ac", "2",
                         wav_path],
                        check=True, capture_output=True)
                    path = wav_path
                except subprocess.CalledProcessError:
                    pass  # 嘗試直接載入
        try:
            duration = self._recorder.load_backing(path)
            self._video_path = None
            name = os.path.basename(path)
            self._src_label.configure(
                text=f"✅  {name[:55]}  （{_fmt_time(duration)}，僅音訊）",
                text_color="green")
            self._time_label.configure(text=f"00:00 / {_fmt_time(duration)}")
            self._seek_slider.configure(to=duration if duration > 0 else 1)
            self._seek_slider.set(0)
            self._rec_btn.configure(state="normal")
            self._preview_btn.configure(state="normal")
            # 清除影片播放器
            if self._video_player:
                self._video_player.release()
                self._video_player = None
            self._video_label.configure(
                image=None,
                text="")
        except Exception as e:
            self._src_fail(str(e))

    def _load_video_file(self, video_path: str):
        """從影片提取音訊並載入（背景執行緒），同時初始化影片播放器。"""
        ffmpeg = find_ffmpeg()
        if not ffmpeg:
            self._src_fail("找不到 FFmpeg，無法提取音訊")
            return

        self._src_label.configure(text="⏳  提取音訊中，請稍候…", text_color="gray")

        import threading

        def extract_and_load():
            wav_path = video_path + "_audio.wav"
            try:
                subprocess.run(
                    [ffmpeg, "-y", "-i", video_path,
                     "-vn", "-acodec", "pcm_s16le", "-ar", "44100", "-ac", "2",
                     wav_path],
                    check=True, capture_output=True)
            except subprocess.CalledProcessError:
                self.after(0, lambda: self._src_fail("音訊提取失敗，請確認 FFmpeg 是否正常安裝"))
                return

            try:
                duration = self._recorder.load_backing(wav_path)
            except Exception as e:
                self.after(0, lambda: self._src_fail(str(e)))
                return

            self.after(0, lambda: self._finish_video_load(video_path, duration))

        threading.Thread(target=extract_and_load, daemon=True).start()

    def _finish_video_load(self, video_path: str, duration: float):
        """在主執行緒完成影片載入後的 UI 更新。"""
        name = os.path.basename(video_path)
        self._src_label.configure(
            text=f"✅  {name[:55]}  （{_fmt_time(duration)}）",
            text_color="green")
        self._time_label.configure(text=f"00:00 / {_fmt_time(duration)}")
        self._seek_slider.configure(to=duration if duration > 0 else 1)
        self._seek_slider.set(0)
        self._rec_btn.configure(state="normal")
        self._preview_btn.configure(state="normal")
        self._is_downloading = False
        self._dl_btn.configure(state="normal")
        self.update_idletasks()

        # 初始化影片播放器
        fw = max(self._video_frame.winfo_width() - 4, _VIDEO_W)
        fh = max(self._video_frame.winfo_height() - 4, _VIDEO_H)
        if self._video_player:
            self._video_player.release()
        self._video_player = VideoPlayer(
            self.winfo_toplevel(), self._video_label, (fw, fh))
        self._video_label.configure(text="")
        try:
            self._video_player.load(video_path)
        except Exception as e:
            self._src_label.configure(
                text=f"⚠  影片載入失敗（{e}），僅音訊模式",
                text_color="#e0a800")

    def _src_fail(self, msg: str):
        self._is_downloading = False
        self._dl_btn.configure(state="normal")
        self._src_label.configure(
            text=f"❌  失敗：{msg[:65]}", text_color="#e05252")

    # ------------------------------------------------------------------ #
    #  音量                                                                #
    # ------------------------------------------------------------------ #

    def _on_volume_change(self, value: float):
        pct = int(value)
        self._recorder.volume = pct / 100.0
        self._vol_label.configure(text=f"{pct}%")

    # ------------------------------------------------------------------ #
    #  進度條拖動                                                          #
    # ------------------------------------------------------------------ #

    def _on_seek_press(self, event):
        self._seek_dragging = True

    def _on_seek_release(self, event):
        self._seek_dragging = False
        if not self._recorder.is_recording:
            target = self._seek_slider.get()
            self._recorder.seek(target)
            # 同步暫停幀位置，讓「繼續」從新位置開始
            new_frame = int(target * self._recorder.samplerate)
            self._preview_paused_frame = new_frame
            if hasattr(self, "_playback_paused_frame"):
                self._playback_paused_frame = new_frame
            # 同步影片
            if self._video_player and self._video_player.is_loaded:
                self._video_player.seek(target)

    # ------------------------------------------------------------------ #
    #  VU 表 + 進度條 50ms 輪詢                                           #
    # ------------------------------------------------------------------ #

    def _start_level_poll(self):
        """每 50ms 更新一次 VU 表和進度條。"""
        self._poll_levels()

    def _poll_levels(self):
        # VU 表更新（錄音或回放中）
        if self._recorder.is_recording or self._recorder.is_playing_back:
            self._vu_backing.set_level(self._recorder.backing_rms)
            self._vu_mic.set_level(self._recorder.mic_rms)
        else:
            # 不在播放：將 level 歸零並讓 peak 緩慢衰減
            self._vu_backing.set_level(0.0)
            self._vu_mic.set_level(0.0)
            self._vu_backing.decay_peak()
            self._vu_mic.decay_peak()

        # 進度條同步（非拖動狀態下）
        if not self._seek_dragging:
            duration = self._recorder.duration
            elapsed = self._recorder.elapsed
            if duration > 0:
                self._seek_slider.set(elapsed)
                self._time_label.configure(
                    text=f"{_fmt_time(elapsed)} / {_fmt_time(duration)}")

        # 繼續排程
        self.after(50, self._poll_levels)

    # ------------------------------------------------------------------ #
    #  試聽（純伴奏，不錄音）— 三態：idle / playing / paused              #
    # ------------------------------------------------------------------ #

    def _on_preview(self):
        if self._preview_state == "idle":
            # 從目前 seek 位置開始
            start_sec = self._seek_slider.get()
            start_frame = int(start_sec * self._recorder.samplerate)
            self._start_preview(start_frame=start_frame)
        elif self._preview_state == "playing":
            self._pause_preview()
        elif self._preview_state == "paused":
            self._resume_preview()

    def _start_preview(self, start_frame: int = 0):
        self._preview_state = "playing"
        self._preview_btn.configure(text="⏸ 暫停")
        self._rec_btn.configure(state="disabled")
        self._stop_btn.configure(state="normal")
        self._vu_backing.reset()
        self._vu_mic.reset()

        if self._video_player and self._video_player.is_loaded:
            if start_frame > 0:
                self._video_player.seek(start_frame / self._recorder.samplerate)
            self._video_player.start(get_elapsed=lambda: self._recorder.elapsed)

        def on_finished():
            self.after(0, self._on_preview_done)

        def on_error(msg):
            self.after(0, lambda: messagebox.showerror("播放錯誤", msg))
            self.after(0, self._on_preview_done)

        self._recorder.start_playback(
            on_finished=on_finished,
            on_error=on_error,
            output_device=self._selected_output_idx,
            start_frame=start_frame,
        )

    def _pause_preview(self):
        self._preview_paused_frame = self._recorder.pause_playback()
        if self._video_player:
            self._video_player.stop()
        self._preview_state = "paused"
        self._preview_btn.configure(text="▶ 繼續")

    def _resume_preview(self):
        self._start_preview(start_frame=self._preview_paused_frame)

    def _on_preview_done(self):
        # 若是暫停觸發的 on_finished，不重置狀態
        if self._preview_state == "paused":
            return
        self._preview_state = "idle"
        self._preview_paused_frame = 0
        self._stop_btn.configure(state="disabled")
        self._rec_btn.configure(state="normal")
        self._preview_btn.configure(text="▶ 試聽", state="normal")
        if self._video_player:
            self._video_player.stop()

    # ------------------------------------------------------------------ #
    #  錄音                                                                #
    # ------------------------------------------------------------------ #

    def _on_start_record(self):
        self._rec_btn.configure(state="disabled")
        self._stop_btn.configure(state="normal")
        self._play_btn.configure(state="disabled")
        self._export_btn.configure(state="disabled")
        self._preview_btn.configure(state="disabled")
        self._seek_slider.set(0)
        self._seek_slider.configure(state="disabled")
        self._vu_backing.reset()
        self._vu_mic.reset()

        if self._video_player and self._video_player.is_loaded:
            self._video_player.start(
                get_elapsed=lambda: self._recorder.elapsed)

        def on_finished():
            self.after(0, self._on_record_done)

        def on_error(msg):
            self.after(0, lambda: messagebox.showerror("錄音錯誤", msg))
            self.after(0, self._reset_ctrl)

        self._recorder.start_recording(
            on_finished=on_finished,
            on_error=on_error,
            input_device=self._selected_input_idx,
            output_device=self._selected_output_idx,
        )

    def _on_stop(self):
        self._recorder.stop()
        self._recorder.stop_playback()
        self._preview_state = "idle"
        self._preview_paused_frame = 0
        if hasattr(self, "_playback_state"):
            self._playback_state = "idle"
            self._playback_paused_frame = 0
        if self._video_player:
            self._video_player.stop()
        self._on_record_done()

    def _on_record_done(self):
        self._stop_btn.configure(state="disabled")
        self._rec_btn.configure(state="normal")
        self._seek_slider.configure(state="normal")
        self._preview_btn.configure(text="▶ 試聽")
        if self._recorder.has_backing:
            self._preview_btn.configure(state="normal")
        if self._recorder.has_recording:
            self._play_btn.configure(text="▶ 回放", state="normal")
            self._export_btn.configure(state="normal")

    def _reset_ctrl(self):
        self._rec_btn.configure(state="normal")
        self._stop_btn.configure(state="disabled")
        if self._recorder.has_backing:
            self._preview_btn.configure(state="normal")

    # ------------------------------------------------------------------ #
    #  回放                                                                #
    # ------------------------------------------------------------------ #

    def _on_playback(self):
        """回放（含人聲混合）— 也支援暫停/繼續。"""
        if not hasattr(self, "_playback_state"):
            self._playback_state = "idle"
            self._playback_paused_frame = 0

        if self._playback_state == "idle":
            start_sec = self._seek_slider.get()
            start_frame = int(start_sec * self._recorder.samplerate)
            self._start_playback(start_frame=start_frame)
        elif self._playback_state == "playing":
            self._pause_playback()
        elif self._playback_state == "paused":
            self._start_playback(start_frame=self._playback_paused_frame)

    def _start_playback(self, start_frame: int = 0):
        self._playback_state = "playing"
        self._play_btn.configure(text="⏸ 暫停")
        self._stop_btn.configure(state="normal")
        self._vu_backing.reset()
        self._vu_mic.reset()

        if self._video_player and self._video_player.is_loaded:
            if start_frame > 0:
                self._video_player.seek(start_frame / self._recorder.samplerate)
            self._video_player.start(get_elapsed=lambda: self._recorder.elapsed)

        def on_finished():
            self.after(0, self._on_playback_done)

        def on_error(msg):
            self.after(0, lambda: messagebox.showerror("播放錯誤", msg))
            self.after(0, self._on_playback_done)

        self._recorder.start_playback(
            on_finished=on_finished,
            on_error=on_error,
            output_device=self._selected_output_idx,
            start_frame=start_frame,
        )

    def _pause_playback(self):
        self._playback_paused_frame = self._recorder.pause_playback()
        if self._video_player:
            self._video_player.stop()
        self._playback_state = "paused"
        self._play_btn.configure(text="▶ 繼續")

    def _on_playback_done(self):
        # 若是暫停觸發的 on_finished，不重置狀態
        if hasattr(self, "_playback_state") and self._playback_state == "paused":
            return
        self._playback_state = "idle"
        self._playback_paused_frame = 0
        self._play_btn.configure(text="▶ 回放", state="normal")
        self._stop_btn.configure(state="disabled")
        if self._video_player:
            self._video_player.stop()

    # ------------------------------------------------------------------ #
    #  導出                                                                #
    # ------------------------------------------------------------------ #

    def _on_export(self):
        try:
            ts = datetime.now().strftime("%Y%m%d_%H%M%S")
            prefix = f"session_{ts}"
            self._recorder.export(self._export_folder, prefix=prefix)
            messagebox.showinfo(
                "導出完成",
                f"已儲存至：{self._export_folder}\n\n"
                f"  {prefix}_vocal.wav       — 純人聲\n"
                f"  {prefix}_backing.wav     — 純伴奏\n"
                f"  {prefix}_multitrack.wav  — 3聲道（可匯入 DAW）")
        except Exception as e:
            messagebox.showerror("導出失敗", str(e))

    def _choose_folder(self):
        folder = filedialog.askdirectory(
            initialdir=self._export_folder, title="選擇導出資料夾")
        if folder:
            self._export_folder = folder
            self._folder_label.configure(text=folder)

    # ------------------------------------------------------------------ #
    #  工具                                                                #
    # ------------------------------------------------------------------ #

    def _paste_url(self):
        try:
            text = self.clipboard_get()
            self._url_entry.delete(0, tk.END)
            self._url_entry.insert(0, text.strip())
        except tk.TclError:
            pass

    def _show_ctx_menu(self, event):
        menu = tk.Menu(self, tearoff=0)
        menu.add_command(label="貼上", command=self._paste_url)
        menu.add_command(label="全選",
                         command=lambda: self._url_entry.select_range(0, tk.END))
        menu.tk_popup(event.x_root, event.y_root)
