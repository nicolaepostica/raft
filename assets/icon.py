#!/usr/bin/env python3
"""Draws Raft's icon from theme.toml: books on a wooden shelf.

Usage: python3 assets/icon.py                 -> assets/icon.png (256x256)
       python3 assets/icon.py --preview DIR   -> also DIR/icon-preview.png (16, 32, 64, 256 px)
"""
import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
S = 4  # supersampling


def theme():
    t = {}
    for line in (HERE / "theme.toml").read_text(encoding="utf-8").splitlines():
        m = re.match(r'\s*([a-z_0-9]+)\s*=\s*"([^"]*)"', line)
        if m:
            t[m.group(1)] = m.group(2)
    return t


def rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i : i + 2], 16) for i in (0, 2, 4))


def draw(size=256):
    t = theme()
    n = size * S
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = n / 256  # one unit of the 256 grid
    # Tile: rounded square in the accent colour.
    d.rounded_rectangle([8 * u, 8 * u, 248 * u, 248 * u], radius=52 * u, fill=rgb(t["accent"]))
    shelf_top = 188 * u
    # Books: (left, width, height, colour); the last one leans on its neighbour.
    books = [
        (52, 34, 116, "book_1"),
        (90, 28, 98, "book_2"),
        (122, 38, 126, "book_3"),
    ]
    for left, w, h, key in books:
        x0, x1 = left * u, (left + w) * u
        y0 = shelf_top - h * u
        d.rounded_rectangle([x0, y0, x1, shelf_top], radius=5 * u, fill=rgb(t[key]))
        # Spine band.
        band = y0 + h * u * 0.22
        d.rectangle([x0, band, x1, band + 9 * u], fill=rgb(t["accent_press"]))
    # Leaning book: a rotated rectangle drawn on its own layer.
    lean = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    ld = ImageDraw.Draw(lean)
    w, h = 32 * u, 112 * u
    cx, base = 166 * u, shelf_top
    ld.rounded_rectangle([cx, base - h, cx + w, base], radius=5 * u, fill=rgb(t["book_4"]))
    ld.rectangle([cx, base - h * 0.78, cx + w, base - h * 0.78 + 9 * u], fill=rgb(t["accent_press"]))
    lean = lean.rotate(-18, center=(cx, base), resample=Image.BICUBIC)
    img.alpha_composite(lean)
    # Shelf board.
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([36 * u, shelf_top, 220 * u, shelf_top + 18 * u], radius=4 * u, fill=rgb(t["wood"]))
    return img.resize((size, size), Image.LANCZOS)


def main():
    draw(256).save(HERE / "icon.png", optimize=True)
    if "--preview" in sys.argv:
        out = Path(sys.argv[sys.argv.index("--preview") + 1]) if len(sys.argv) > sys.argv.index("--preview") + 1 else HERE
        sheet = Image.new("RGBA", (16 + 32 + 64 + 256 + 50, 256), (255, 255, 255, 255))
        x = 0
        for s in (16, 32, 64, 256):
            sheet.alpha_composite(draw(s), (x, 0))
            x += s + 10
        sheet.save(out / "icon-preview.png")
        print(out / "icon-preview.png")


if __name__ == "__main__":
    main()
