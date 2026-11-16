#!/usr/bin/env python3
"""Render the installer bitmaps from the official KeyReed icon (assets/keyreed-icon.png).

  python3 scripts/installer/gen_art.py   ->  scripts/installer/welcome.bmp (164x314), header.bmp (150x57)

The logo is only resampled and composited, never redrawn or recoloured. The waveform bars are
decoration in the app's accent colour (settings theme `accent_fill`). Needs ffmpeg for resampling.
"""
import math, struct, subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
ICON = HERE.parent.parent / "assets" / "keyreed-icon.png"
SIDEBAR, BORDER, WHITE = (0xEE, 0xF3, 0xF0), (0xDD, 0xE5, 0xE1), (255, 255, 255)
ACCENT, FADE = (0x04, 0x78, 0x57), (0xB4, 0xD3, 0xC5)


def icon_rgba(n):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(ICON), "-vf", f"scale={n}:{n}:flags=lanczos",
                          "-f", "rawvideo", "-pix_fmt", "rgba", "-"], check=True, capture_output=True).stdout
    assert len(raw) == n * n * 4
    return raw


def canvas(w, h, bg):
    return [[bg] * w for _ in range(h)]


def blend(px, col, a):
    return tuple(round(p * (1 - a) + c * a) for p, c in zip(px, col))


def paste(img, raw, n, x0, y0):
    for y in range(n):
        for x in range(n):
            r, g, b, a = raw[(y * n + x) * 4:(y * n + x) * 4 + 4]
            if a:
                img[y0 + y][x0 + x] = blend(img[y0 + y][x0 + x], (r, g, b), a / 255)


def capsule(img, cx, cy, w, h, col):
    """Anti-aliased vertical capsule (signed-distance coverage)."""
    r = w / 2
    for y in range(int(cy - h / 2) - 1, int(cy + h / 2) + 2):
        for x in range(int(cx - r) - 1, int(cx + r) + 2):
            dx, dy = abs(x + 0.5 - cx), abs(y + 0.5 - cy)
            sd = math.hypot(dx, max(0, dy - (h / 2 - r))) - r
            a = min(1.0, max(0.0, 0.5 - sd))
            if a and 0 <= y < len(img) and 0 <= x < len(img[0]):
                img[y][x] = blend(img[y][x], col, a)


def write_bmp(path, img):
    h, w = len(img), len(img[0])
    row = (w * 3 + 3) & ~3
    data = bytearray()
    for line in reversed(img):
        b = bytearray()
        for r, g, bl in line:
            b += bytes((bl, g, r))
        data += b + b"\0" * (row - len(b))
    path.write_bytes(b"BM" + struct.pack("<IHHI", 54 + len(data), 0, 0, 54) +
                     struct.pack("<IiiHHIIiiII", 40, w, h, 1, 24, 0, len(data), 2835, 2835, 0, 0) + bytes(data))


def welcome():
    w, h = 164, 314
    img = canvas(w, h, SIDEBAR)
    for y in range(h):
        img[y][w - 1] = BORDER
    n = 104
    paste(img, icon_rgba(n), n, (w - 1 - n) // 2, 52)
    heights = [10, 18, 30, 22, 42, 58, 38, 66, 36, 54, 28, 40, 20, 12]
    pitch, bw = 9, 5
    x0 = (w - 1 - (len(heights) * pitch - (pitch - bw))) / 2
    for i, bh in enumerate(heights):
        d = abs(i - (len(heights) - 1) / 2) / ((len(heights) - 1) / 2)
        col = blend(ACCENT, FADE, d * 0.85)
        capsule(img, x0 + i * pitch + bw / 2, 214, bw, bh, col)
    write_bmp(HERE / "welcome.bmp", img)


def header():
    w, h = 150, 57
    img = canvas(w, h, WHITE)
    n = 40
    paste(img, icon_rgba(n), n, w - n - 12, (h - n) // 2)
    write_bmp(HERE / "header.bmp", img)


if __name__ == "__main__":
    welcome(); header()
    print("wrote welcome.bmp, header.bmp")
