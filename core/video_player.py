"""
影片播放模組。
使用 OpenCV 讀取影片幀，透過 PIL + CTkLabel 在 UI 中顯示。
音訊由 AudioRecorder 另行處理，此模組只負責畫面同步。
"""

import time
import tkinter as tk
import threading
from typing import Any, Callable

import numpy as np
from PIL import Image, ImageTk


class VideoPlayer:
    """
    嵌入式影片幀渲染器。
    將影片幀顯示在指定的 tkinter widget（一個含 configure(image=...) 的 Label）。
    音訊同步：透過外部傳入的「已播放秒數」getter 來對齊幀位置。
    """

    def __init__(
        self,
        root: tk.Misc,
        display_widget: Any,
        display_size: tuple[int, int] = (640, 360),
    ) -> None:
        """
        root:           主視窗（用於 after() 排程）
        display_widget: 任何支援 configure(image=...) 的 tkinter widget
        display_size:   (寬, 高) 像素，影片幀會縮放至此尺寸
        """
        self._root = root
        self._widget = display_widget
        self._display_size = display_size
        self._cv2 = None            # 延遲載入
        self._cap = None            # cv2.VideoCapture
        self._fps: float = 30.0
        self._total_frames: int = 0
        self._duration: float = 0.0
        self._playing = False
        self._start_time: float = 0.0
        self._paused_at: float = 0.0
        self._get_elapsed: Callable[[], float] | None = None
        self._photo_ref = None  # 防止 GC

    # ------------------------------------------------------------------ #
    #  載入                                                                #
    # ------------------------------------------------------------------ #

    def load(self, path: str) -> float:
        """
        載入影片檔案。
        回傳影片總時長（秒）。延遲載入 cv2 以加快啟動速度。
        """
        if self._cv2 is None:
            import cv2 as _cv2
            self._cv2 = _cv2

        cv2 = self._cv2
        if self._cap:
            self._cap.release()
        self._cap = cv2.VideoCapture(path)
        if not self._cap.isOpened():
            raise RuntimeError(f"無法開啟影片：{path}")

        self._fps = self._cap.get(cv2.CAP_PROP_FPS) or 30.0
        self._total_frames = int(self._cap.get(cv2.CAP_PROP_FRAME_COUNT))
        self._duration = self._total_frames / self._fps
        self._playing = False

        # 顯示第一幀作為預覽
        self._cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
        ret, frame = self._cap.read()
        if ret:
            self._show_frame(frame)

        return self._duration

    # ------------------------------------------------------------------ #
    #  播放控制                                                            #
    # ------------------------------------------------------------------ #

    def start(self, get_elapsed: Callable[[], float] | None = None) -> None:
        """
        開始播放影片幀。
        get_elapsed: 可選的 callable，回傳目前音訊已播放秒數（用於音訊同步）。
                     若為 None，改用內部計時器。
        """
        if self._cap is None or self._cv2 is None:
            return
        cv2 = self._cv2
        self._playing = True
        self._start_time = time.time()
        self._get_elapsed = get_elapsed
        self._cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
        self._schedule_next_frame()

    def stop(self) -> None:
        """停止播放。"""
        self._playing = False

    def release(self) -> None:
        """釋放資源。"""
        self._playing = False
        if self._cap:
            self._cap.release()
            self._cap = None

    def set_display_size(self, width: int, height: int) -> None:
        """動態調整顯示尺寸（視窗縮放時使用）。"""
        self._display_size = (width, height)

    def seek(self, seconds: float) -> None:
        """跳轉到指定秒數並顯示靜止幀。"""
        if self._cap is None or self._cv2 is None:
            return
        cv2 = self._cv2
        target_frame = int(seconds * self._fps)
        target_frame = max(0, min(target_frame, self._total_frames - 1))
        self._cap.set(cv2.CAP_PROP_POS_FRAMES, target_frame)
        ret, frame = self._cap.read()
        if ret:
            self._show_frame(frame)

    # ------------------------------------------------------------------ #
    #  內部                                                                #
    # ------------------------------------------------------------------ #

    def _schedule_next_frame(self) -> None:
        if not self._playing or self._cap is None:
            return

        # 決定目前應顯示哪一幀（依音訊同步或計時器）
        if self._get_elapsed is not None:
            elapsed = self._get_elapsed()
        else:
            elapsed = time.time() - self._start_time

        target_frame = int(elapsed * self._fps)

        if target_frame >= self._total_frames:
            self._playing = False
            return

        # 跳至目標幀（允許跳幀以避免積壓）
        cv2 = self._cv2
        current_pos = int(self._cap.get(cv2.CAP_PROP_POS_FRAMES))
        if abs(target_frame - current_pos) > 2:
            self._cap.set(cv2.CAP_PROP_POS_FRAMES, target_frame)

        ret, frame = self._cap.read()
        if ret:
            self._show_frame(frame)

        # 排程下一幀
        interval_ms = max(1, int(1000 / self._fps))
        self._root.after(interval_ms, self._schedule_next_frame)

    def _show_frame(self, frame: np.ndarray) -> None:
        """將 OpenCV BGR 幀轉換並顯示在 widget 上（保持原始比例，黑邊填充）。"""
        cv2 = self._cv2
        frame_rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        img = Image.fromarray(frame_rgb)

        orig_w, orig_h = img.size
        disp_w, disp_h = self._display_size

        # 等比縮放，確保完整畫面在顯示區域內
        scale = min(disp_w / orig_w, disp_h / orig_h)
        new_w = max(1, int(orig_w * scale))
        new_h = max(1, int(orig_h * scale))
        img = img.resize((new_w, new_h), Image.Resampling.BILINEAR)

        # 黑色畫布，居中貼上
        canvas = Image.new("RGB", (disp_w, disp_h), (0, 0, 0))
        offset_x = (disp_w - new_w) // 2
        offset_y = (disp_h - new_h) // 2
        canvas.paste(img, (offset_x, offset_y))

        photo = ImageTk.PhotoImage(canvas)
        self._widget.configure(image=photo)
        self._photo_ref = photo  # 防止 GC

    # ------------------------------------------------------------------ #
    #  屬性                                                                #
    # ------------------------------------------------------------------ #

    @property
    def duration(self) -> float:
        return self._duration

    @property
    def fps(self) -> float:
        return self._fps

    @property
    def is_loaded(self) -> bool:
        return self._cap is not None
