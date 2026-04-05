"""
產生程式圖示。
執行：py -3 make_icon.py
"""

import math
from PIL import Image, ImageDraw, ImageFilter

SIZE = 256


def make_icon():
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # ── 圓角方形背景（鵝黃暖色漸層）──────────────────────────────
    r = 52
    _rounded_rect(draw, 0, 0, SIZE, SIZE, r, (253, 246, 227, 255))
    for i in range(SIZE // 2):
        ratio = i / (SIZE // 2)
        c = _lerp_color((253, 246, 227), (245, 234, 208), ratio)
        alpha = int(200 * (1 - ratio * 0.3))
        _rounded_rect(draw, i, i, SIZE - i, SIZE - i, max(r - i, 4),
                      c + (alpha,), outline=False)

    # ── 光暈效果（暖色中心亮點）───────────────────────────────────
    glow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    for rad in range(90, 0, -3):
        alpha = int(50 * (1 - rad / 90))
        gd.ellipse(
            (SIZE // 2 - rad, SIZE // 2 - rad,
             SIZE // 2 + rad, SIZE // 2 + rad),
            fill=(255, 220, 100, alpha))
    glow = glow.filter(ImageFilter.GaussianBlur(14))
    img = Image.alpha_composite(img, glow)
    draw = ImageDraw.Draw(img)

    cx, cy = SIZE // 2, SIZE // 2

    # ── 向日葵花瓣（雙層）────────────────────────────────────────
    num_petals = 16
    # 外層花瓣（金橘色，大而飽滿）
    for i in range(num_petals):
        angle = math.radians(i * 360 / num_petals)
        pw, ph = 36, 70
        petal = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
        pd = ImageDraw.Draw(petal)
        # 花瓣形狀：橢圓，底部在中心，尖端向外
        pd.ellipse((cx - pw // 2, cy - ph, cx + pw // 2, cy + 8),
                    fill=(245, 175, 50, 240))
        rotated = petal.rotate(-math.degrees(angle) + 180,
                               center=(cx, cy), resample=Image.BICUBIC)
        img = Image.alpha_composite(img, rotated)

    # 內層花瓣（淺橘色，較小，交錯排列）
    for i in range(num_petals):
        angle = math.radians(i * 360 / num_petals + 360 / num_petals / 2)
        petal = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
        pd = ImageDraw.Draw(petal)
        pd.ellipse((cx - 24 // 2, cy - 55, cx + 24 // 2, cy + 6),
                    fill=(240, 148, 60, 230))
        rotated = petal.rotate(-math.degrees(angle) + 180,
                               center=(cx, cy), resample=Image.BICUBIC)
        img = Image.alpha_composite(img, rotated)

    draw = ImageDraw.Draw(img)

    # ── 花心（棕色圓盤 + 種子紋理）──────────────────────────────
    core_r = 48
    # 深棕底色
    draw.ellipse((cx - core_r, cy - core_r, cx + core_r, cy + core_r),
                 fill=(110, 65, 25, 255))
    # 內圈漸層
    for ri in range(core_r, 0, -1):
        ratio = ri / core_r
        c = _lerp_color((150, 90, 35), (90, 55, 20), ratio)
        alpha = int(255 * (0.6 + 0.4 * ratio))
        draw.ellipse((cx - ri, cy - ri, cx + ri, cy + ri),
                     fill=c + (alpha,))

    # 種子點（黃金角螺旋排列）
    for n in range(1, 160):
        a = n * 137.508 * math.pi / 180
        dist = math.sqrt(n) * 3.3
        if dist > core_r - 5:
            break
        sx = cx + math.cos(a) * dist
        sy = cy + math.sin(a) * dist
        dot_r = 2.0
        draw.ellipse((sx - dot_r, sy - dot_r, sx + dot_r, sy + dot_r),
                     fill=(70, 40, 10, 200))

    # ── 邊框高光 ──────────────────────────────────────────────────
    _rounded_rect(draw, 1, 1, SIZE - 1, SIZE - 1, r,
                  (255, 255, 255, 0), outline=True,
                  outline_color=(232, 213, 183, 80), outline_width=2)

    return img


def _rounded_rect(draw, x0, y0, x1, y1, r, fill,
                  outline=False, outline_color=None, outline_width=1):
    if not outline:
        draw.rounded_rectangle((x0, y0, x1, y1), radius=r, fill=fill)
    else:
        draw.rounded_rectangle((x0, y0, x1, y1), radius=r,
                                outline=outline_color, width=outline_width)


def _lerp_color(c1, c2, t):
    return tuple(int(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))


if __name__ == "__main__":
    import os
    base = os.path.dirname(os.path.abspath(__file__))
    assets = os.path.join(base, "assets")
    os.makedirs(assets, exist_ok=True)

    icon = make_icon()

    # 存 PNG
    png_path = os.path.join(assets, "icon.png")
    icon.save(png_path)
    print(f"已存：{png_path}")

    # 存 ICO（多尺寸）
    ico_path = os.path.join(assets, "icon.ico")
    sizes = [16, 32, 48, 64, 128, 256]
    icons = [icon.resize((s, s), Image.LANCZOS).convert("RGBA") for s in sizes]
    icon.save(ico_path, format="ICO", append_images=icons[:-1])
    print(f"已存：{ico_path}")
