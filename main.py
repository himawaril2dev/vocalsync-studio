"""
VocalSync 錄音器 — 獨立啟動入口。
只包含錄音頁面，不含下載管理介面。
"""

import os
import sys

import customtkinter as ctk

from ui import theme as T
from ui.recording_page import RecordingPage

_DEFAULT_SETTINGS: dict = {
    "theme": "dark",
    "download_folder": os.path.join(os.path.expanduser("~"), "Downloads", "YouTube"),
}


class RecorderApp(ctk.CTk):
    def __init__(self):
        ctk.set_appearance_mode("light")
        ctk.set_default_color_theme("blue")
        super().__init__()
        self.configure(fg_color=T.BG)
        # 設定全域預設字體，讓所有未指定 font 的元件都使用粉圓
        ctk.ThemeManager.theme["CTkFont"] = {
            "family": T.FONT_BODY,
            "size": 13,
            "weight": "normal",
        }

        self.title("VocalSync 錄音器")
        self.geometry("820x720")
        self.minsize(680, 580)
        self.resizable(True, True)
        self._set_icon()

        self.grid_rowconfigure(0, weight=1)
        self.grid_columnconfigure(0, weight=1)

        page = RecordingPage(self, _DEFAULT_SETTINGS)
        page.grid(row=0, column=0, sticky="nsew", padx=0, pady=0)

    def _set_icon(self):
        if getattr(sys, "frozen", False):
            base = sys._MEIPASS
        else:
            base = os.path.dirname(os.path.abspath(__file__))
        ico_path = os.path.join(base, "assets", "icon.ico")
        if os.path.isfile(ico_path):
            try:
                self.iconbitmap(ico_path)
            except Exception:
                pass


if __name__ == "__main__":
    app = RecorderApp()
    app.mainloop()
