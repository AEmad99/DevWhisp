import io
import struct
import sys
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE_IMG = Path(
    r"C:\Users\ahmed\.gemini\antigravity\brain\16e84702-f7a5-4df7-a1a5-434a66e87d95\devwhisp_sophisticated_icon_1784846471519.jpg"
)

ICONS_DIR = ROOT / "src-tauri" / "icons"
PUBLIC_ICON = ROOT / "public" / "icon.png"
DIST_ICON = ROOT / "dist" / "icon.png"
DESIGN_PNG = ROOT / "design" / "devwhisp-icon-1024.png"

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


def export_ico(cache, path):
    sizes = sorted(ICO_SIZES)
    blobs = []
    for s in sizes:
        layer = cache[s]
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
        cache[size].save(buf, format="PNG", optimize=True)
        b = buf.getvalue()
        chunks.append(code + (8 + len(b)).to_bytes(4, "big") + b)
    body = b"".join(chunks)
    path.write_bytes(b"icns" + (8 + len(body)).to_bytes(4, "big") + body)
    print(f"  -> {path.name}  ({path.stat().st_size // 1024} KB)")


def main():
    if not SOURCE_IMG.exists():
        print(f"Error: source image {SOURCE_IMG} does not exist.")
        return 1

    base_img = Image.open(SOURCE_IMG).convert("RGBA")
    if base_img.size != (1024, 1024):
        base_img = base_img.resize((1024, 1024), Image.Resampling.LANCZOS)

    ICONS_DIR.mkdir(parents=True, exist_ok=True)
    cache = {}

    all_sizes = set(
        [size for _, size in TAURI_BUNDLE_PNGS + WINDOWS_SQUARE_PNGS]
        + ICO_SIZES
        + [32, 64, 128, 256, 512, 1024]
    )
    for s in all_sizes:
        if s == 1024:
            cache[1024] = base_img
        else:
            cache[s] = base_img.resize((s, s), Image.Resampling.LANCZOS)

    # Save master PNG
    cache[1024].save(DESIGN_PNG, format="PNG", optimize=True)
    print(f"Saved master -> {DESIGN_PNG}")

    # Tauri bundle PNGs
    for name, size in TAURI_BUNDLE_PNGS:
        target = ICONS_DIR / name
        cache[size].save(target, format="PNG", optimize=True)
        print(f"  -> {target.name} ({size}px)")

    # Windows square PNGs
    for name, size in WINDOWS_SQUARE_PNGS:
        target = ICONS_DIR / name
        cache[size].save(target, format="PNG", optimize=True)
        print(f"  -> {target.name} ({size}px)")

    # Public & Dist
    cache[512].save(PUBLIC_ICON, format="PNG", optimize=True)
    print(f"  -> {PUBLIC_ICON.name} (512px)")
    if DIST_ICON.parent.exists():
        cache[512].save(DIST_ICON, format="PNG", optimize=True)
        print(f"  -> {DIST_ICON.name} (512px)")

    # ICO & ICNS
    export_ico(cache, ICONS_DIR / "icon.ico")
    export_icns(cache, ICONS_DIR / "icon.icns")

    # Preview files
    preview = ROOT / "design" / "ico_preview"
    preview.mkdir(exist_ok=True)
    for size in (16, 20, 24, 32, 48, 64):
        cache[size].save(preview / f"ico_{size}x{size}.png", format="PNG")

    print("Successfully generated all sophisticated icon assets!")
    return 0


if __name__ == "__main__":
    sys.exit(main())
