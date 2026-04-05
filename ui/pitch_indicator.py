"""
即時音高指示器元件。
顯示當前偵測到的音符名稱、cent 偏移條、頻率值。
"""

import tkinter as tk

import customtkinter as ctk

from core.pitch_data import PitchSample
from ui import theme as T


class PitchIndicator(ctk.CTkFrame):
    """
    即時音高指示器，嵌入 VU 表區域右側。
    呼叫 update(sample) 更新顯示，sample 為 None 時顯示 "--"。
    """

    def __init__(self, parent, **kwargs):
        kwargs.setdefault("fg_color", "transparent")
        super().__init__(parent, **kwargs)

        self.grid_columnconfigure(0, weight=1)

        # 音符名稱（大字）
        self._note_label = ctk.CTkLabel(
            self, text="--",
            font=ctk.CTkFont(family=T.FONT_DISPLAY, size=20, weight="bold"),
            text_color=T.TEXT_PRIMARY,
            width=60,
        )
        self._note_label.grid(row=0, column=0, padx=(0, 4), pady=(2, 0))

        # cent 偏移條
        self._cent_canvas = tk.Canvas(
            self, width=60, height=10,
            bg=self._apply_appearance_mode(T.SURFACE),
            highlightthickness=0,
        )
        self._cent_canvas.grid(row=1, column=0, padx=(0, 4), pady=0)

        # 頻率值（小字）
        self._freq_label = ctk.CTkLabel(
            self, text="--- Hz",
            font=ctk.CTkFont(*T.TINY),
            text_color=T.TEXT_MUTED,
            width=60,
        )
        self._freq_label.grid(row=2, column=0, padx=(0, 4), pady=(0, 2))

        self._draw_cent_bar(0, active=False)

    def update_pitch(self, sample: PitchSample | None) -> None:
        """更新音高顯示。"""
        if sample is None:
            self._note_label.configure(text="--", text_color=T.TEXT_MUTED)
            self._freq_label.configure(text="--- Hz")
            self._draw_cent_bar(0, active=False)
            return

        note_text = f"{sample.note_name}{sample.octave}"
        self._note_label.configure(text=note_text, text_color=T.TEXT_PRIMARY)
        self._freq_label.configure(text=f"{sample.frequency_hz:.0f} Hz")
        self._draw_cent_bar(sample.cent_offset, active=True)

    def reset(self) -> None:
        self.update_pitch(None)

    def _draw_cent_bar(self, cent: int, active: bool) -> None:
        """繪製 cent 偏移條。中央為準，左偏低右偏高。"""
        c = self._cent_canvas
        w = c.winfo_width() if c.winfo_width() > 10 else 60
        h = c.winfo_height() if c.winfo_height() > 4 else 10

        c.delete("all")

        bg = self._apply_appearance_mode(T.SURFACE)
        c.configure(bg=bg)

        # 底色條
        c.create_rectangle(0, 0, w, h, fill=bg, outline="")

        # 刻度線（中央）
        mid = w // 2
        c.create_line(mid, 0, mid, h, fill=self._apply_appearance_mode(T.BORDER), width=1)

        if not active:
            return

        # 偏移指示
        cent_clamped = max(-50, min(50, cent))
        offset_px = int((cent_clamped / 50.0) * (mid - 2))

        if abs(cent_clamped) <= 10:
            color = self._apply_appearance_mode(T.ACCENT_GREEN)
        elif abs(cent_clamped) <= 30:
            color = T.WARN_YELLOW
        else:
            color = self._apply_appearance_mode(T.ACCENT_RED)

        x = mid + offset_px
        bar_w = max(3, abs(offset_px))
        x0 = min(mid, x)
        x1 = max(mid, x)
        c.create_rectangle(x0, 1, x1, h - 1, fill=color, outline="")
