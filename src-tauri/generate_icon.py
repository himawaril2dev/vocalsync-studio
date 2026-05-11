"""一次性 helper：產生最小可用的 icon.ico（16x16 透明），讓 tauri-build 能順利通過。
產出後可手動替換為正式 logo。"""
import os
import struct

OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "icons")
OUT_FILE = os.path.join(OUT_DIR, "icon.ico")

WIDTH = 16
HEIGHT = 16
BPP = 32  # ARGB
PIXEL_BYTES = WIDTH * HEIGHT * 4
BMP_HEADER_SIZE = 40
IMAGE_SIZE = BMP_HEADER_SIZE + PIXEL_BYTES + (WIDTH * HEIGHT // 8)  # + AND mask

# ICONDIR (6 bytes)
icondir = struct.pack("<HHH", 0, 1, 1)

# ICONDIRENTRY (16 bytes)
icondirentry = struct.pack(
    "<BBBBHHII",
    WIDTH,
    HEIGHT,
    0,        # color count (0 = >256)
    0,        # reserved
    1,        # color planes
    BPP,      # bits per pixel
    IMAGE_SIZE,
    22,       # offset to bitmap data (6 + 16)
)

# BITMAPINFOHEADER (40 bytes)
bmp_header = struct.pack(
    "<IiiHHIIiiII",
    BMP_HEADER_SIZE,
    WIDTH,
    HEIGHT * 2,  # ICO BMP 高度為 image*2 (XOR + AND mask)
    1,           # planes
    BPP,
    0,           # compression (BI_RGB)
    PIXEL_BYTES,
    0,
    0,
    0,
    0,
)

# 透明像素：BGRA (0,0,0,0)
pixels = b"\x00" * PIXEL_BYTES

# AND mask：每行 row 16 px = 2 bytes，全 0
and_mask = b"\x00" * (WIDTH * HEIGHT // 8)

data = icondir + icondirentry + bmp_header + pixels + and_mask

os.makedirs(OUT_DIR, exist_ok=True)
with open(OUT_FILE, "wb") as f:
    f.write(data)

print(f"Wrote {OUT_FILE} ({len(data)} bytes)")
