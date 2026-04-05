"""
yt-dlp 下載封裝模組。
提供影片、音訊、字幕的下載函式，以及 URL 類型偵測工具。
所有下載函式均為同步執行，呼叫端應在獨立執行緒中呼叫。
"""

import os
import re
import threading
from collections.abc import Callable
from typing import Any

import yt_dlp

from core.ffmpeg_check import find_ffmpeg
from core.format_helper import (
    build_audio_format,
    build_video_format,
    get_audio_codec,
    get_subtitle_langs,
)


# --------------------------------------------------------------------------- #
#  進度回呼型別定義                                                              #
# --------------------------------------------------------------------------- #

ProgressInfo = dict[str, Any]
ProgressCallback = Callable[[ProgressInfo], None]


# --------------------------------------------------------------------------- #
#  URL 類型偵測                                                                 #
# --------------------------------------------------------------------------- #

def detect_url_type(url: str) -> str:
    """
    偵測 YouTube URL 類型。
    回傳值：'video' | 'playlist' | 'channel'
    """
    if "playlist?list=" in url or "&list=" in url:
        return "playlist"

    parsed = url.lower()
    channel_patterns = [r"/@[\w\-]+", r"/channel/", r"/user/", r"/c/"]
    for pattern in channel_patterns:
        if re.search(pattern, parsed):
            # 如果同時有 watch?v= 則視為單一影片
            if "watch?v=" not in parsed:
                return "channel"

    return "video"


# --------------------------------------------------------------------------- #
#  yt-dlp 選項建構                                                              #
# --------------------------------------------------------------------------- #

def _base_opts(output_dir: str, progress_hook: ProgressCallback) -> dict:
    opts = {
        "outtmpl": os.path.join(output_dir, "%(title)s.%(ext)s"),
        "restrictfilenames": False,
        "ignoreerrors": False,
        "progress_hooks": [progress_hook],
        "quiet": True,
        "no_warnings": False,
    }
    ffmpeg = find_ffmpeg()
    if ffmpeg:
        opts["ffmpeg_location"] = ffmpeg
    return opts


def build_video_opts(
    quality_label: str,
    output_dir: str,
    progress_hook: ProgressCallback,
    subtitle_lang_label: str | None = None,
    is_channel: bool = False,
) -> dict:
    opts = _base_opts(output_dir, progress_hook)
    opts["format"] = build_video_format(quality_label)
    opts["merge_output_format"] = "mp4"

    if is_channel:
        opts["outtmpl"] = os.path.join(output_dir, "%(uploader)s", "%(title)s.%(ext)s")
        opts["download_archive"] = os.path.join(output_dir, "downloaded_archive.txt")

    if subtitle_lang_label:
        langs = get_subtitle_langs(subtitle_lang_label)
        opts["writesubtitles"] = True
        opts["writeautomaticsub"] = True
        opts["subtitlesformat"] = "srt"
        if langs is not None:
            opts["subtitleslangs"] = langs

    return opts


def build_audio_opts(
    fmt_label: str,
    output_dir: str,
    progress_hook: ProgressCallback,
) -> dict:
    opts = _base_opts(output_dir, progress_hook)
    opts["format"] = build_audio_format()
    opts["postprocessors"] = [
        {
            "key": "FFmpegExtractAudio",
            "preferredcodec": get_audio_codec(fmt_label),
            "preferredquality": "192",
        }
    ]
    return opts


# --------------------------------------------------------------------------- #
#  下載執行函式                                                                 #
# --------------------------------------------------------------------------- #

class DownloadError(Exception):
    """下載過程中發生的例外。"""


def download(url: str, ydl_opts: dict) -> None:
    """
    使用指定的 yt-dlp 選項下載 URL。
    同步執行，應在執行緒中呼叫。
    拋出 DownloadError 若發生錯誤。
    """
    try:
        with yt_dlp.YoutubeDL(ydl_opts) as ydl:
            ydl.download([url])
    except yt_dlp.utils.DownloadError as e:
        raise DownloadError(str(e)) from e
    except Exception as e:
        raise DownloadError(f"未預期的錯誤：{e}") from e


def start_download_thread(
    url: str,
    ydl_opts: dict,
    on_error: Callable[[str], None] | None = None,
    on_complete: Callable[[], None] | None = None,
) -> threading.Thread:
    """
    在背景執行緒中啟動下載，回傳執行緒物件。
    """
    def _run():
        try:
            download(url, ydl_opts)
            if on_complete:
                on_complete()
        except DownloadError as e:
            if on_error:
                on_error(str(e))

    thread = threading.Thread(target=_run, daemon=True)
    thread.start()
    return thread
