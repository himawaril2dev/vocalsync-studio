"""
VU 音量條元件。
使用 tkinter Canvas 繪製彩色分格音量條，含峰值保持指示線。
"""

import tkinter as tk


# 顏色設定
_GREEN  = "#00c853"
_YELLOW = "#f9a825"
_RED    = "#d50000"
_OFF_G  = "#0d2b14"
_OFF_Y  = "#2b2200"
_OFF_R  = "#2b0000"
_PEAK   = "#ffffff"
_BG     = "#0d0d0d"

_SEGMENTS    = 32
_GREEN_END   = int(_SEGMENTS * 0.68)   # 0–68% 綠
_YELLOW_END  = int(_SEGMENTS * 0.88)   # 68–88% 黃
# 88–100% 紅


class VUMeter(tk.Canvas):
    """
    水平 VU 音量條。
    set_level(rms) 接受 0.0–1.0 的 RMS 值；
    內部會做適度放大以讓小音量也清晰可見。
    """

    def __init__(self, parent, boost: float = 3.0, **kwargs):
        """
        boost: RMS 放大倍數（預設 3x，讓 -10dBFS 左右的訊號填滿約 70%）
        """
        kwargs.setdefault("height", 18)
        kwargs.setdefault("bg", _BG)
        kwargs.setdefault("highlightthickness", 0)
        super().__init__(parent, **kwargs)
        self._level: float = 0.0
        self._peak:  float = 0.0
        self._boost = boost
        self.bind("<Configure>", lambda e: self._draw())

    # ------------------------------------------------------------------ #

    def set_level(self, rms: float):
        """傳入 0.0–1.0 的 RMS 值，更新顯示。"""
        level = min(rms * self._boost, 1.0)
        self._level = level
        if level > self._peak:
            self._peak = level
        self._draw()

    def decay_peak(self, rate: float = 0.025):
        """峰值緩慢下降，建議每 50ms 呼叫一次。"""
        if self._peak > 0:
            self._peak = max(0.0, self._peak - rate)
            self._draw()

    def reset(self):
        self._level = 0.0
        self._peak  = 0.0
        self._draw()

    # ------------------------------------------------------------------ #

    def _draw(self):
        w = self.winfo_width()
        h = self.winfo_height()
        if w < 10 or h < 4:
            return

        self.delete("all")
        self.create_rectangle(0, 0, w, h, fill=_BG, outline="")

        gap = 2
        seg_w = (w - gap * (_SEGMENTS - 1)) / _SEGMENTS
        filled = int(self._level * _SEGMENTS)

        for i in range(_SEGMENTS):
            x0 = int(i * (seg_w + gap))
            x1 = int(x0 + seg_w)
            if i < _GREEN_END:
                on, off = _GREEN, _OFF_G
            elif i < _YELLOW_END:
                on, off = _YELLOW, _OFF_Y
            else:
                on, off = _RED, _OFF_R
            color = on if i < filled else off
            self.create_rectangle(x0, 2, x1, h - 2, fill=color, outline="")

        # 峰值指示線
        if self._peak > 0:
            px = int(self._peak * w)
            px = min(px, w - 2)
            self.create_line(px, 0, px, h, fill=_PEAK, width=2)
