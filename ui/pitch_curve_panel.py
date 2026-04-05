"""
音高曲線面板。
使用 tkinter Canvas 繪製音高隨時間變化的曲線，支援即時追加和回放游標。
"""

import tkinter as tk

import customtkinter as ctk

from core.pitch_data import PitchSample, PitchTrack, freq_to_midi
from ui import theme as T

# Y 軸範圍：MIDI note 48 (C3) ~ 84 (C6)
_MIDI_MIN = 48
_MIDI_MAX = 84
_NOTE_LABELS = {48: "C3", 60: "C4", 72: "C5", 84: "C6"}

# 曲線顏色（引用 theme 常數，取淺色模式值）
_CURVE_COLOR = T.PRIMARY[0]       # 品牌橘
_CURSOR_COLOR = T.ACCENT_RED[0]   # 強調紅
_GRID_COLOR = T.BORDER[0]         # 邊線色
_NOTE_LINE_COLOR = T.VIDEO_BG[0]  # 淡底色


class PitchCurvePanel(ctk.CTkFrame):
    """
    音高曲線面板。

    使用方式：
    - set_duration(sec): 設定時間軸總長度
    - append_sample(sample): 錄音中即時追加
    - set_data(track): 設定完整音高軌跡（回放用）
    - set_cursor(sec): 更新播放游標位置
    - clear(): 清空所有資料
    """

    def __init__(self, parent, height: int = 100, **kwargs):
        kwargs.setdefault("fg_color", T.SURFACE)
        kwargs.setdefault("corner_radius", T.CARD_RADIUS)
        super().__init__(parent, **kwargs)

        self.grid_columnconfigure(0, weight=1)
        self.grid_rowconfigure(0, weight=1)

        # 標題列
        header = ctk.CTkFrame(self, fg_color="transparent", height=20)
        header.grid(row=0, column=0, padx=12, pady=(6, 0), sticky="ew")
        ctk.CTkLabel(
            header, text="音高曲線",
            font=ctk.CTkFont(*T.CAPTION), text_color=T.TEXT_SECONDARY,
        ).pack(side="left")

        bg_color = self._apply_appearance_mode(T.SURFACE)

        self._canvas = tk.Canvas(
            self, height=height, bg=bg_color, highlightthickness=0,
        )
        self._canvas.grid(row=1, column=0, padx=12, pady=(2, 8), sticky="nsew")
        self.grid_rowconfigure(1, weight=1)

        self._samples: list[PitchSample] = []
        self._duration: float = 0.0
        self._cursor_sec: float = -1.0
        self._placeholder_shown = True

        self._canvas.bind("<Configure>", lambda e: self._redraw())
        self._draw_placeholder()

    def set_duration(self, seconds: float) -> None:
        self._duration = seconds

    def append_sample(self, sample: PitchSample) -> None:
        """錄音中即時追加資料點。"""
        self._placeholder_shown = False
        self._samples.append(sample)
        # 增量繪製：只畫最後一個點（避免每次全部重繪）
        if len(self._samples) >= 2:
            self._draw_segment(self._samples[-2], self._samples[-1])
        elif len(self._samples) == 1:
            self._draw_point(self._samples[0])

    def set_data(self, track: PitchTrack) -> None:
        """設定完整音高軌跡（回放時使用）。"""
        self._samples = list(track.samples)
        self._placeholder_shown = False
        self._redraw()

    def set_cursor(self, seconds: float) -> None:
        """更新回放游標位置。"""
        self._cursor_sec = seconds
        self._draw_cursor()

    def clear(self) -> None:
        self._samples.clear()
        self._cursor_sec = -1.0
        self._duration = 0.0
        self._placeholder_shown = True
        self._canvas.delete("all")
        self._draw_placeholder()

    # ------------------------------------------------------------------ #
    #  內部繪製邏輯                                                        #
    # ------------------------------------------------------------------ #

    def _draw_placeholder(self) -> None:
        w = self._canvas.winfo_width()
        h = self._canvas.winfo_height()
        if w < 10 or h < 10:
            return
        self._canvas.delete("all")
        self._canvas.create_text(
            w // 2, h // 2,
            text="錄音後將顯示音高曲線",
            fill=self._apply_appearance_mode(T.TEXT_MUTED),
            font=(T.FONT_BODY, 11),
        )

    def _redraw(self) -> None:
        """完整重繪。"""
        self._canvas.delete("all")

        if self._placeholder_shown or not self._samples:
            self._draw_placeholder()
            return

        self._draw_grid()
        self._draw_curve()
        self._draw_cursor()

    def _draw_grid(self) -> None:
        """繪製 Y 軸音符參考線。"""
        w = self._canvas.winfo_width()
        h = self._canvas.winfo_height()
        if w < 10 or h < 10:
            return

        for midi, label in _NOTE_LABELS.items():
            y = self._midi_to_y(midi, h)
            self._canvas.create_line(
                30, y, w, y,
                fill=_NOTE_LINE_COLOR, dash=(2, 4), tags="grid",
            )
            self._canvas.create_text(
                2, y, text=label, anchor="w",
                fill=self._apply_appearance_mode(T.TEXT_MUTED),
                font=(T.FONT_MONO, 8), tags="grid",
            )

    def _draw_curve(self) -> None:
        """繪製完整曲線。"""
        w = self._canvas.winfo_width()
        h = self._canvas.winfo_height()
        if w < 30 or h < 10 or not self._samples:
            return

        draw_w = w - 30  # 左邊留給刻度

        # 降採樣：若資料點超過畫布像素，每 N 個取一個
        max_points = draw_w
        samples = self._samples
        if len(samples) > max_points:
            step = len(samples) / max_points
            samples = [samples[int(i * step)] for i in range(int(max_points))]

        points = []
        for s in samples:
            x = self._time_to_x(s.timestamp_sec, draw_w) + 30
            midi = freq_to_midi(s.frequency_hz)
            y = self._midi_to_y(midi, h)
            points.append(x)
            points.append(y)

        if len(points) >= 4:
            self._canvas.create_line(
                *points, fill=_CURVE_COLOR, width=2,
                smooth=True, tags="curve",
            )

    def _draw_segment(self, s1: PitchSample, s2: PitchSample) -> None:
        """增量繪製最後一段線段。"""
        w = self._canvas.winfo_width()
        h = self._canvas.winfo_height()
        if w < 30 or h < 10:
            return

        draw_w = w - 30
        x1 = self._time_to_x(s1.timestamp_sec, draw_w) + 30
        y1 = self._midi_to_y(freq_to_midi(s1.frequency_hz), h)
        x2 = self._time_to_x(s2.timestamp_sec, draw_w) + 30
        y2 = self._midi_to_y(freq_to_midi(s2.frequency_hz), h)

        self._canvas.create_line(
            x1, y1, x2, y2,
            fill=_CURVE_COLOR, width=2, tags="curve",
        )

    def _draw_point(self, s: PitchSample) -> None:
        """繪製單一資料點。"""
        w = self._canvas.winfo_width()
        h = self._canvas.winfo_height()
        if w < 30 or h < 10:
            return

        draw_w = w - 30
        x = self._time_to_x(s.timestamp_sec, draw_w) + 30
        y = self._midi_to_y(freq_to_midi(s.frequency_hz), h)
        self._canvas.create_oval(
            x - 2, y - 2, x + 2, y + 2,
            fill=_CURVE_COLOR, outline="", tags="curve",
        )

    def _draw_cursor(self) -> None:
        """繪製播放游標。"""
        self._canvas.delete("cursor")
        if self._cursor_sec < 0 or self._duration <= 0:
            return

        w = self._canvas.winfo_width()
        h = self._canvas.winfo_height()
        if w < 30 or h < 10:
            return

        draw_w = w - 30
        x = self._time_to_x(self._cursor_sec, draw_w) + 30
        self._canvas.create_line(
            x, 0, x, h,
            fill=_CURSOR_COLOR, width=2, tags="cursor",
        )

    # ------------------------------------------------------------------ #
    #  座標轉換                                                            #
    # ------------------------------------------------------------------ #

    def _time_to_x(self, sec: float, draw_w: int) -> int:
        if self._duration <= 0:
            return 0
        return int((sec / self._duration) * draw_w)

    def _midi_to_y(self, midi: float, canvas_h: int) -> int:
        """MIDI note → Y 座標（高音在上，低音在下）。"""
        midi_clamped = max(_MIDI_MIN, min(_MIDI_MAX, midi))
        ratio = (midi_clamped - _MIDI_MIN) / (_MIDI_MAX - _MIDI_MIN)
        return int(canvas_h * (1.0 - ratio))
