"""DevWhisp app icon — "Voice Code" (family-matched).

Violet rounded-square tile + angle brackets < > framing three parallel
horizontal smoky bars. Canonical vector: design/devwhisp-icon.svg.

Taskbar paths (16–32): simplified brackets + fewer bars + heavy supersampling
so the mark stays legible after downsample.
"""

from __future__ import annotations

import io
import math
import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageEnhance, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
ICONS_DIR = ROOT / "src-tauri" / "icons"
PUBLIC_ICON = ROOT / "public" / "icon.png"
DIST_ICON = ROOT / "dist" / "icon.png"
DESIGN_PNG = ROOT / "design" / "devwhisp-icon-1024.png"

CANVAS = 1024
TILE_INSET = 96
TILE_RADIUS = 196
TOP_COLOR = (237, 233, 254)  # #ede9fe
MID_COLOR = (167, 139, 250)  # #a78bfa
BOTTOM_COLOR = (76, 29, 149)  # #4c1d95

# Bracket geometry: open chevrons
BRACKET_STROKE = 76
BRACKET_TIP_X = 260
BRACKET_MID_X = 420
BRACKET_TOP_Y = 280
BRACKET_BOT_Y = 744
BRACKET_MID_Y = 512

# Horizontal smoky bars: (left_x, y_center, right_x, height, opacity)
# Centered between the brackets.
BARS = [
    (394, 386, 630, 54, 0.52),
    (364, 508, 660, 62, 0.80),
    (394, 630, 630, 58, 1.00),
]

BARS_SMALL = [
    (390, 390, 634, 66, 0.55),
    (360, 512, 664, 74, 0.82),
    (390, 634, 634, 70, 1.00),
]

BARS_TINY = [
    (400, 415, 624, 78, 0.62),
    (375, 525, 649, 86, 0.86),
    (400, 635, 624, 82, 1.00),
]


def supersample_for(size: int) -> int:
    if size <= 20:
        return 24
    if size <= 32:
        return 20
    if size <= 48:
        return 14
    if size <= 64:
        return 10
    if size <= 128:
        return 8
    if size <= 256:
        return 6
    return 4


def draw_tile(internal: int) -> Image.Image:
    s = internal / CANVAS
    inset = TILE_INSET * s
    end = internal - inset
    radius = TILE_RADIUS * s

    import numpy as np

    y, x = np.mgrid[0:internal, 0:internal].astype(np.float64)
    span = 2 * (end - inset)
    if span > 0:
        t = ((x - inset) + (y - inset)) / span
        t = np.clip(t, 0.0, 1.0)
    else:
        t = np.zeros((internal, internal), dtype=np.float64)

    top = np.array(TOP_COLOR, dtype=np.float64)
    mid = np.array(MID_COLOR, dtype=np.float64)
    bot = np.array(BOTTOM_COLOR, dtype=np.float64)
    t2 = t[..., None]
    grad_arr = np.where(
        t2 <= 0.40,
        top * (1 - t2 / 0.40) + mid * (t2 / 0.40),
        mid * (1 - (t2 - 0.40) / 0.60) + bot * ((t2 - 0.40) / 0.60),
    ).astype(np.uint8)
    grad = Image.fromarray(grad_arr, mode="RGB")

    mask = Image.new("L", (internal, internal), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (inset, inset, end - 1, end - 1), radius=radius, fill=255
    )
    tile = Image.new("RGBA", (internal, internal), (0, 0, 0, 0))
    tile.paste(grad, (0, 0), mask)

    # Top sheen
    sheen = Image.new("RGBA", (internal, internal), (0, 0, 0, 0))
    ImageDraw.Draw(sheen).rounded_rectangle(
        (inset, inset, end - 1, inset + (end - inset) * 0.42),
        radius=radius,
        fill=(255, 255, 255, 32),
    )
    fade = Image.new("L", (internal, internal), 0)
    fd = ImageDraw.Draw(fade)
    h = int((end - inset) * 0.5)
    for i in range(h):
        a = int(45 * (1 - i / max(1, h)))
        fd.line([(inset, inset + i), (end, inset + i)], fill=a)
    sheen.putalpha(Image.composite(fade, Image.new("L", (internal, internal), 0), mask))
    tile.alpha_composite(sheen)

    rim = ImageDraw.Draw(tile, "RGBA")
    rim.rounded_rectangle(
        (inset + 3 * s, inset + 3 * s, end - 1 - 3 * s, end - 1 - 3 * s),
        radius=max(2, radius - 3 * s),
        outline=(255, 255, 255, 56),
        width=max(1, int(6 * s)),
    )
    return tile


def thick_segment_polygon(p0, p1, width):
    """Return a filled-polygon outline for a thick line segment with round caps."""
    x0, y0 = p0
    x1, y1 = p1
    dx = x1 - x0
    dy = y1 - y0
    length = math.hypot(dx, dy)
    if length == 0:
        return []
    angle = math.atan2(dy, dx)
    cos_a = math.cos(angle)
    sin_a = math.sin(angle)
    w = width / 2
    res = max(16, int(math.pi * width / 3))

    def lg(lx, ly):
        """Local-to-global coordinate transform."""
        return (x0 + lx * cos_a - ly * sin_a, y0 + lx * sin_a + ly * cos_a)

    pts = []
    # Top side
    pts.append(lg(0, w))
    pts.append(lg(length, w))
    # End cap (top to bottom around the outer side)
    for i in range(1, res):
        theta = math.pi / 2 - math.pi * i / res
        pts.append(lg(length + w * math.cos(theta), w * math.sin(theta)))
    # Bottom side
    pts.append(lg(length, -w))
    pts.append(lg(0, -w))
    # Start cap (bottom to top around the outer side)
    for i in range(1, res):
        theta = -math.pi / 2 + math.pi * i / res
        pts.append(lg(w * math.cos(theta), w * math.sin(theta)))
    return pts


def draw_bracket(draw, s, left: bool):
    """Draw one < or > bracket with clean round caps and joins."""
    if left:
        top = (BRACKET_MID_X * s, BRACKET_TOP_Y * s)
        tip = (BRACKET_TIP_X * s, BRACKET_MID_Y * s)
        bot = (BRACKET_MID_X * s, BRACKET_BOT_Y * s)
    else:
        top = ((CANVAS - BRACKET_MID_X) * s, BRACKET_TOP_Y * s)
        tip = ((CANVAS - BRACKET_TIP_X) * s, BRACKET_MID_Y * s)
        bot = ((CANVAS - BRACKET_MID_X) * s, BRACKET_BOT_Y * s)

    poly1 = thick_segment_polygon(top, tip, BRACKET_STROKE * s)
    poly2 = thick_segment_polygon(tip, bot, BRACKET_STROKE * s)
    if poly1:
        draw.polygon(poly1, fill=(255, 255, 255, 255))
    if poly2:
        draw.polygon(poly2, fill=(255, 255, 255, 255))


def draw_smoke_bar(draw, x0, y, x1, h, s):
    """Draw one horizontal pill-shaped bar with smoky rounded ends."""
    half = h / 2
    draw.rounded_rectangle(
        (x0 * s, (y - half) * s, x1 * s, (y + half) * s),
        radius=half * s,
        fill=(255, 255, 255, 255),
    )


def draw_glyph(tile: Image.Image, internal: int, detail: str):
    s = internal / CANVAS

    if detail == "tiny":
        bars = BARS_TINY
    elif detail == "minimal":
        bars = BARS_SMALL
    elif detail == "standard":
        bars = BARS[:2]
    else:
        bars = BARS

    # Brackets
    bracket_layer = Image.new("RGBA", tile.size, (0, 0, 0, 0))
    bd = ImageDraw.Draw(bracket_layer)
    draw_bracket(bd, s, left=True)
    draw_bracket(bd, s, left=False)
    tile.alpha_composite(bracket_layer)

    # Bars
    for x0, y, x1, h, opacity in bars:
        layer = Image.new("RGBA", tile.size, (0, 0, 0, 0))
        d = ImageDraw.Draw(layer)
        draw_smoke_bar(d, x0, y, x1, h, s)
        if opacity < 1.0:
            a = layer.split()[3].point(lambda v, op=opacity: int(v * op))
            layer.putalpha(a)
        tile.alpha_composite(layer)


def detail_level(size: int) -> str:
    if size <= 20:
        return "tiny"
    if size <= 32:
        return "minimal"
    if size <= 48:
        return "standard"
    return "full"


def _crisp_alpha(img: Image.Image, size: int) -> Image.Image:
    """Snap soft alpha edges so taskbar glyphs don't look muddy."""
    import numpy as np

    arr = np.array(img, dtype=np.float32)
    a = arr[:, :, 3]
    if size <= 20:
        lo, hi = 28.0, 200.0
    elif size <= 32:
        lo, hi = 22.0, 210.0
    else:
        lo, hi = 18.0, 220.0
    a2 = np.clip((a - lo) / (hi - lo), 0.0, 1.0)
    a2 = a2 * a2 * (3.0 - 2.0 * a2)
    arr[:, :, 3] = a2 * 255.0
    return Image.fromarray(arr.astype(np.uint8), "RGBA")


def _lift_glyph_white(img: Image.Image, size: int) -> Image.Image:
    """Push near-white glyph pixels toward pure white so the mark stays crisp on purple."""
    import numpy as np

    arr = np.array(img, dtype=np.float32)
    rgb = arr[:, :, :3]
    a = arr[:, :, 3]
    lum = rgb.mean(axis=2)
    if size <= 20:
        t0, t1 = 130.0, 210.0
        strength = 0.72
    elif size <= 32:
        t0, t1 = 140.0, 220.0
        strength = 0.55
    else:
        t0, t1 = 150.0, 230.0
        strength = 0.35
    mask = (a > 90) & (lum > t0)
    if not np.any(mask):
        return img
    w = np.clip((lum - t0) / (t1 - t0), 0.0, 1.0)
    boost = (strength * w)[..., None]
    rgb2 = rgb.copy()
    rgb2[mask] = rgb[mask] * (1.0 - boost[mask]) + 255.0 * boost[mask]
    arr[:, :, :3] = rgb2
    return Image.fromarray(arr.astype(np.uint8), "RGBA")


def _sharpen_small(img: Image.Image, size: int) -> Image.Image:
    """Mild contrast snap so taskbar glyphs don't look soft/muddy."""
    radius = 0.4 if size <= 20 else 0.55
    percent = 90 if size <= 20 else 110
    sharp = img.filter(
        ImageFilter.UnsharpMask(radius=radius, percent=percent, threshold=2)
    )
    enhancer = ImageEnhance.Contrast(sharp)
    return enhancer.enhance(1.04 if size <= 32 else 1.02)


def build_icon(size: int) -> Image.Image:
    ss = supersample_for(size)
    internal = size * ss
    tile = draw_tile(internal)
    draw_glyph(tile, internal, detail_level(size))
    if internal != size:
        tile = tile.resize((size, size), Image.Resampling.LANCZOS)
    if size <= 64:
        tile = _crisp_alpha(tile, size)
        tile = _lift_glyph_white(tile, size)
    if size <= 48:
        tile = _sharpen_small(tile, size)
    return tile


TAURI_BUNDLE_PNGS = [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("256x256.png", 256),
    ("1024x1024.png", 1024),
    ("icon.png", 1024),
]
WINDOWS_SQUARE_PNGS = [
    ("Square30x30Logo.png", 30),
    ("Square44x44Logo.png", 44),
    ("Square71x71Logo.png", 71),
    ("Square89x89Logo.png", 89),
    ("Square107x107Logo.png", 107),
    ("Square142x142Logo.png", 142),
    ("Square150x150Logo.png", 150),
    ("Square284x284Logo.png", 284),
    ("Square310x310Logo.png", 310),
    ("StoreLogo.png", 50),
]
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def export_png(cache, path, size):
    cache.setdefault(size, build_icon(size)).save(path, format="PNG", optimize=True)
    print(f"  -> {path.name}  ({size}px, {path.stat().st_size // 1024} KB)")


def export_ico(cache, path):
    sizes = sorted(ICO_SIZES)
    blobs = []
    for s in sizes:
        layer = cache.setdefault(s, build_icon(s))
        buf = io.BytesIO()
        layer.save(buf, format="PNG", optimize=True)
        blobs.append(buf.getvalue())
    offset = 6 + 16 * len(sizes)
    out = io.BytesIO()
    out.write(struct.pack("<HHH", 0, 1, len(sizes)))
    for s, blob in zip(sizes, blobs):
        w = s if s < 256 else 0
        out.write(struct.pack("<BBBBHHII", w, w, 0, 0, 1, 0, len(blob), offset))
        offset += len(blob)
    for blob in blobs:
        out.write(blob)
    path.write_bytes(out.getvalue())
    print(f"  -> {path.name}  (sizes {sizes}, {path.stat().st_size // 1024} KB)")


def export_icns(cache, path):
    mac = {
        b"ic07": 128,
        b"ic08": 256,
        b"ic09": 512,
        b"ic10": 1024,
        b"ic11": 32,
        b"ic12": 64,
        b"ic13": 256,
        b"ic14": 512,
    }
    chunks = []
    for code, size in mac.items():
        buf = io.BytesIO()
        cache.setdefault(size, build_icon(size)).save(buf, format="PNG", optimize=True)
        b = buf.getvalue()
        chunks.append(code + (8 + len(b)).to_bytes(4, "big") + b)
    body = b"".join(chunks)
    path.write_bytes(b"icns" + (8 + len(body)).to_bytes(4, "big") + body)
    print(f"  -> {path.name}  ({path.stat().st_size // 1024} KB)")


def main():
    ICONS_DIR.mkdir(parents=True, exist_ok=True)
    cache = {}
    print("Rendering DevWhisp 'Voice Code' icon...")
    master = build_icon(1024)
    cache[1024] = master
    master.save(DESIGN_PNG, format="PNG", optimize=True)
    print(f"master -> {DESIGN_PNG}")

    print("\nTauri bundle PNGs:")
    for name, size in TAURI_BUNDLE_PNGS:
        export_png(cache, ICONS_DIR / name, size)
    print("\nWindows square logos:")
    for name, size in WINDOWS_SQUARE_PNGS:
        export_png(cache, ICONS_DIR / name, size)
    print("\nPublic + dist favicon (512):")
    export_png(cache, PUBLIC_ICON, 512)
    if DIST_ICON.parent.exists():
        export_png(cache, DIST_ICON, 512)
    print("\nWindows .ico:")
    export_ico(cache, ICONS_DIR / "icon.ico")
    print("\nmacOS .icns:")
    export_icns(cache, ICONS_DIR / "icon.icns")

    preview = ROOT / "design" / "ico_preview"
    preview.mkdir(exist_ok=True)
    for size in (16, 20, 24, 32, 48, 64):
        export_png(cache, preview / f"ico_{size}x{size}.png", size)
    print(f"\nDone — {len(cache)} sizes rendered.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
