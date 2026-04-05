"""
VocalSync Studio 設計系統。
集中定義色碼、字體、按鈕樣式，供所有 UI 模組引用。
"""

# ------------------------------------------------------------------ #
#  色碼（格式：(淺色, 深色)）                                          #
# ------------------------------------------------------------------ #

# 背景層 — 鵝黃色為基底的明亮主題
BG = ("#fdf6e3", "#fdf6e3")
SURFACE = ("#fffdf5", "#fffdf5")
SURFACE_VAR = ("#f5ead0", "#f5ead0")

# 品牌色 — 淺橘色
PRIMARY = ("#f0946c", "#f0946c")
PRIMARY_HOVER = ("#e87d50", "#e87d50")

# 次要 — 奶茶色
SECONDARY = ("#e8d5b7", "#e8d5b7")
SECONDARY_HOVER = ("#ddc49e", "#ddc49e")

# 強調色
ACCENT_RED = ("#e07060", "#e07060")
ACCENT_RED_HOVER = ("#c85a4a", "#c85a4a")
ACCENT_GREEN = ("#6dba7a", "#6dba7a")

# 文字
TEXT_PRIMARY = ("#4a3728", "#4a3728")
TEXT_SECONDARY = ("#8c7560", "#8c7560")
TEXT_MUTED = ("#b5a48e", "#b5a48e")

# 邊線
BORDER = ("#e0d2b8", "#e0d2b8")

# 導覽列
NAV_BG = ("#f5ead0", "#f5ead0")
NAV_ACTIVE = ("#ece0c5", "#ece0c5")

# 影片區底色
VIDEO_BG = ("#ede2c8", "#ede2c8")

# ------------------------------------------------------------------ #
#  字體                                                                #
# ------------------------------------------------------------------ #

FONT_DISPLAY = "jf open 粉圓 2.1"
FONT_BODY = "jf open 粉圓 2.1"
FONT_MONO = "Consolas"

# 字體層級 (family, size, weight)
H1 = (FONT_DISPLAY, 22, "bold")
H2 = (FONT_DISPLAY, 16, "bold")
H3 = (FONT_DISPLAY, 14, "bold")
BODY1 = (FONT_BODY, 13, "normal")
BODY2 = (FONT_BODY, 12, "normal")
CAPTION = (FONT_BODY, 11, "normal")
TINY = (FONT_BODY, 10, "normal")
MONO_S = (FONT_MONO, 11, "normal")

# ------------------------------------------------------------------ #
#  間距常數                                                            #
# ------------------------------------------------------------------ #

PAD_PAGE = 24       # 下載器頁面側邊距
PAD_PAGE_REC = 16   # 錄音器頁面側邊距（較窄）
PAD_CARD_Y = 12     # 卡片垂直間距
PAD_CARD_Y_REC = 6  # 錄音器卡片垂直間距
PAD_INNER = 16      # 卡片內部 padding
CARD_RADIUS = 10    # 卡片圓角
