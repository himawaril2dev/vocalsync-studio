"""
yt-dlp 格式字串建構工具。
根據使用者在 UI 選擇的格式與畫質，產生對應的 yt-dlp format selector。
"""

# 畫質標籤 → 高度限制對應
_QUALITY_HEIGHT: dict[str, int | None] = {
    "最佳畫質": None,
    "1080p": 1080,
    "720p": 720,
    "480p": 480,
    "360p": 360,
}

# 音訊格式標籤
AUDIO_FORMATS = ("MP3", "M4A")

# 字幕語言選項
SUBTITLE_LANG_OPTIONS = {
    "繁體中文": ["zh-TW", "zh-Hant"],
    "簡體中文": ["zh-Hans", "zh-CN"],
    "英文": ["en"],
    "全部": None,  # None 代表下載所有可用語言
}


def build_video_format(quality_label: str) -> str:
    """
    根據畫質標籤產生 yt-dlp video format selector。
    優先下載 mp4+m4a 組合，確保最高相容性。
    """
    height = _QUALITY_HEIGHT.get(quality_label)

    if height is None:
        # 最佳畫質：優先 mp4，fallback 到任意最佳
        return "bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best"

    # 指定高度：height <= N，優先 mp4
    return (
        f"bestvideo[height<={height}][ext=mp4]+bestaudio[ext=m4a]"
        f"/bestvideo[height<={height}]+bestaudio"
        f"/best[height<={height}]"
        f"/best"
    )


def build_audio_format() -> str:
    """純音訊下載的 format selector。"""
    return "bestaudio/best"


def get_audio_codec(fmt_label: str) -> str:
    """
    UI 選擇的音訊格式標籤 → FFmpegExtractAudio preferredcodec。
    """
    mapping = {
        "MP3": "mp3",
        "M4A": "m4a",
    }
    return mapping.get(fmt_label, "mp3")


def get_subtitle_langs(lang_label: str) -> list[str] | None:
    """
    UI 選擇的字幕語言標籤 → yt-dlp subtitleslangs 清單。
    回傳 None 代表下載所有語言。
    """
    return SUBTITLE_LANG_OPTIONS.get(lang_label)


def quality_labels() -> list[str]:
    """回傳所有畫質標籤（供 UI 下拉選單使用）。"""
    return list(_QUALITY_HEIGHT.keys())
