import os
import shutil
import subprocess
import sys


def find_ffmpeg() -> str | None:
    """
    嘗試找到系統上的 FFmpeg 可執行檔路徑。
    搜尋順序：系統 PATH → 應用程式目錄 → 常見安裝路徑
    回傳找到的路徑，若找不到則回傳 None。
    """
    # 1. 系統 PATH
    path = shutil.which("ffmpeg")
    if path:
        return path

    # 2. 應用程式所在目錄（打包後的 exe 旁邊）
    base_dir = _get_base_dir()
    candidate = os.path.join(base_dir, "ffmpeg.exe" if sys.platform == "win32" else "ffmpeg")
    if os.path.isfile(candidate):
        return candidate

    # 3. Windows 常見安裝路徑
    if sys.platform == "win32":
        common_paths = [
            r"C:\ffmpeg\bin\ffmpeg.exe",
            r"C:\Program Files\ffmpeg\bin\ffmpeg.exe",
            r"C:\Program Files (x86)\ffmpeg\bin\ffmpeg.exe",
        ]
        for p in common_paths:
            if os.path.isfile(p):
                return p

    return None


def is_ffmpeg_available() -> bool:
    """回傳 FFmpeg 是否可用。"""
    return find_ffmpeg() is not None


def get_ffmpeg_path() -> str:
    """
    取得 FFmpeg 路徑。若找不到，拋出 RuntimeError。
    """
    path = find_ffmpeg()
    if path is None:
        raise RuntimeError(
            "找不到 FFmpeg。請先安裝 FFmpeg 並確認已加入系統 PATH。\n"
            "Windows 安裝說明：https://ffmpeg.org/download.html"
        )
    return path


def get_ffmpeg_version() -> str | None:
    """回傳 FFmpeg 版本字串，若找不到則回傳 None。"""
    path = find_ffmpeg()
    if path is None:
        return None
    try:
        result = subprocess.run(
            [path, "-version"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        first_line = result.stdout.splitlines()[0] if result.stdout else ""
        return first_line
    except Exception:
        return None


def _get_base_dir() -> str:
    """取得應用程式的基底目錄（相容 PyInstaller 打包環境）。"""
    if getattr(sys, "frozen", False):
        return os.path.dirname(sys.executable)
    return os.path.dirname(os.path.abspath(__file__))
