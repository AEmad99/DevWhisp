"""DevWhisp app icon — "Voice Waves" (family-matched, geometric).

Single source of truth for ALL raster icon assets. Replaces the two old,
conflicting pipelines (render_clean_icon.py wisps + regen_icons.py raster).

Design grammar shared with DevTerm (>_) and DevSpace (<*>):
  - violet rounded-square tile, gradient #c4b5fd -> #7c3aed, rx=196, inset 96
  - one solid white fill element (the source dot) + bold white strokes
  - round caps / round joins, generous padding, legible at 16 px

DevWhisp glyph = a voice source dot with three concentric sound-wave arcs
radiating right. The gentle opacity fade reads as "whisper" softness and
echoes the pill's three states (idle / listening / processing). Arcs are
true circular arcs (geometric, not organic) so it sits cleanly beside the
chevron glyphs of its siblings.

Rendering: heavy supersampling + LANCZOS downsample. Strokes are stamped as
dense overlapping discs => exact even width with perfect round caps and no
Pillow arc-width ambiguity. Anti-aliasing comes from the downsample.
"""

from __future__ import annotations

import io
import math
import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(r"D:\projects\DevWhisp")
ICONS_DIR = ROOT / "src-tauri" / "icons"
PUBLIC_ICON = ROOT / "public" / "icon.png"
DIST_ICON = ROOT / "dist" / "icon.png"
DESIGN_PNG = ROOT / "design" / "devwhisp-icon-1024.png"

CANVAS = 1024
TILE_INSET = 96
TILE_SIZE = 832
TILE_RADIUS = 196
TOP_COLOR = (196, 181, 253)      # #c4b5fd
BOTTOM_COLOR = (124, 58, 237)    # #7c3aed

# --- Glyph geometry, in 1024 coords -----------------------------------------
# Source dot (the "voice"): a solid white disc, left-of-centre.
SRC_X, SRC_Y = 326, 512
DOT_R = 62

# Three concentric right-facing arcs (sound waves). Each: (radius, half-angle
# deg, stroke width, opacity). Radii evenly spaced; widths + opacity taper so
# it whispers outward. Group is visually centred on the tile.
ARC_SPAN_DEG = 54  # arc runs from -SPAN to +SPAN around due-east
ARCS = [
    (152, 66, 1.00),
    (282, 58, 0.80),
    (412, 50, 0.60),
]


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(len(a)))


def supersample_for(size: int) -> int:
    if size <= 32:
        return 16
    if size <= 64:
        return 10
    if size <= 128:
        return 8
    if size <= 256:
        return 6
    return 4


def draw_tile(internal: int) -> Image.Image:
    """Diagonal-gradient rounded square matching the family proportions."""
    s = internal / CANVAS
    inset = TILE_INSET * s
    end = internal - inset
    radius = TILE_RADIUS * s

    # Build the gradient as a small vertical strip then stretch — but we want a
    # top-left -> bottom-right diagonal like DevSpace. Compute per-pixel via a
    # cheap gradient image: paint rows, then the mask handles corners.
    grad = Image.new("RGB", (internal, internal), BOTTOM_COLOR)
    px = grad.load()
    span = 2 * (end - inset)
    for y in range(internal):
        for x in range(internal):
            t = (x - inset + (y - inset)) / span if span else 0.0
            t = 0.0 if t < 0 else 1.0 if t > 1 else t
            px[x, y] = lerp(TOP_COLOR, BOTTOM_COLOR, t)

    mask = Image.new("L", (internal, internal), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (inset, inset, end - 1, end - 1), radius=radius, fill=255
    )
    tile = Image.new("RGBA", (internal, internal), (0, 0, 0, 0))
    tile.paste(grad, (0, 0), mask)

    # Faint inner rim — same trick the siblings use for depth.
    rim = ImageDraw.Draw(tile, "RGBA")
    rim.rounded_rectangle(
        (inset + 3 * s, inset + 3 * s, end - 1 - 3 * s, end - 1 - 3 * s),
        radius=max(2, radius - 3 * s),
        outline=(255, 255, 255, 46),
        width=max(1, int(6 * s)),
    )
    return tile


def stamp_arc(draw, cx, cy, r, half_span_deg, width):
    """Stamp dense discs along a right-facing arc => even stroke, round caps."""
    half = width / 2
    arclen = r * math.radians(2 * half_span_deg)
    n = max(24, int(arclen / max(1.0, half * 0.35)))
    for i in range(n + 1):
        ang = math.radians(-half_span_deg + (2 * half_span_deg) * i / n)
        x = cx + r * math.cos(ang)
        y = cy + r * math.sin(ang)
        draw.ellipse((x - half, y - half, x + half, y + half),
                     fill=(255, 255, 255, 255))


def draw_glyph(tile: Image.Image, internal: int, detail: str):
    s = internal / CANVAS
    cx, cy = SRC_X * s, SRC_Y * s

    if detail == "minimal":
        arcs = ARCS[:2]
    elif detail == "standard":
        arcs = ARCS
    else:
        arcs = ARCS

    for radius, width, opacity in arcs:
        # Tiny sizes: keep faint arcs readable.
        op = opacity if detail == "full" else min(1.0, opacity + 0.18)
        layer = Image.new("RGBA", tile.size, (0, 0, 0, 0))
        d = ImageDraw.Draw(layer)
        stamp_arc(d, cx, cy, radius * s, ARC_SPAN_DEG, width * s)
        if op < 1.0:
            a = layer.split()[3].point(lambda v: int(v * op))
            layer.putalpha(a)
        tile.alpha_composite(layer)

    # Source dot on top, solid white.
    rr = DOT_R * s
    dot = Image.new("RGBA", tile.size, (0, 0, 0, 0))
    ImageDraw.Draw(dot).ellipse((cx - rr, cy - rr, cx + rr, cy + rr),
                                fill=(255, 255, 255, 255))
    tile.alpha_composite(dot)


def detail_level(size: int) -> str:
    if size <= 24:
        return "minimal"
    if size <= 48:
        return "standard"
    return "full"


def build_icon(size: int) -> Image.Image:
    ss = supersample_for(size)
    internal = size * ss
    tile = draw_tile(internal)
    draw_glyph(tile, internal, detail_level(size))
    if internal != size:
        tile = tile.resize((size, size), Image.Resampling.LANCZOS)
    return tile


# --- asset export -----------------------------------------------------------
TAURI_BUNDLE_PNGS = [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("256x256.png", 256),
    ("1024x1024.png", 1024),
    ("icon.png", 1024),
]
WINDOWS_SQUARE_PNGS = [
    ("Square30x30Logo.png", 30), ("Square44x44Logo.png", 44),
    ("Square71x71Logo.png", 71), ("Square89x89Logo.png", 89),
    ("Square107x107Logo.png", 107), ("Square142x142Logo.png", 142),
    ("Square150x150Logo.png", 150), ("Square284x284Logo.png", 284),
    ("Square310x310Logo.png", 310), ("StoreLogo.png", 50),
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
    mac = {b"ic07": 128, b"ic08": 256, b"ic09": 512, b"ic10": 1024,
           b"ic11": 32, b"ic12": 64, b"ic13": 256, b"ic14": 512}
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
    print("Rendering DevWhisp 'Voice Waves' icon (geometric, family-matched)...")
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
