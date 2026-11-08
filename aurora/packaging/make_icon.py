"""Draws Aurora's app icon (icon.png, 1024 px): a rounded square with a night gradient, a soft
accent glow and a white "A". Run once; the PNG is checked in and package.sh turns it into .icns.

    python3 aurora/packaging/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

S = 1024
here = Path(__file__).parent


def gradient():
    img = Image.new("RGB", (S, S))
    top, bottom = (16, 20, 44), (6, 7, 14)
    px = img.load()
    for y in range(S):
        t = y / (S - 1)
        c = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        for x in range(S):
            px[x, y] = c
    return img


def main():
    base = gradient().convert("RGBA")
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    g = ImageDraw.Draw(glow)
    g.ellipse((180, 120, 900, 760), fill=(122, 162, 255, 150))
    g.ellipse((420, 420, 1000, 980), fill=(196, 155, 255, 110))
    base = Image.alpha_composite(base, glow.filter(ImageFilter.GaussianBlur(140)))
    d = ImageDraw.Draw(base)
    font = None
    for name in ("DejaVuSans-Bold.ttf", "Arial Bold.ttf", "Helvetica.ttc"):
        try:
            font = ImageFont.truetype(name, 600)
            break
        except OSError:
            pass
    font = font or ImageFont.load_default()
    box = d.textbbox((0, 0), "A", font=font)
    w, h = box[2] - box[0], box[3] - box[1]
    d.text(((S - w) / 2 - box[0], (S - h) / 2 - box[1] + 10), "A", font=font, fill=(240, 244, 255, 255))
    # macOS icon grid: the art sits in an 824 px rounded square with a 100 px margin.
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((100, 100, 924, 924), radius=185, fill=255)
    art = base.resize((824, 824), Image.LANCZOS)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(art, (100, 100))
    out.putalpha(mask)
    out.save(here / "icon.png")


if __name__ == "__main__":
    main()
