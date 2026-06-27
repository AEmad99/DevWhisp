"""Extract every icon variant embedded in devwhisp.exe as PNG for inspection."""
from pathlib import Path

import pefile

EXE = Path(r"D:\projects\DevWhisp\src-tauri\target\release\devwhisp.exe")
OUT_DIR = Path(r"D:\projects\DevWhisp\design\exe_icons_png")

pe = pefile.PE(str(EXE))
OUT_DIR.mkdir(exist_ok=True)

# Map icon_id -> PNG bytes from RT_ICON resources
icon_data = {}
for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
    if entry.id != 3:  # RT_ICON
        continue
    for e in entry.directory.entries:
        for le in e.directory.entries:
            rva = le.data.struct.OffsetToData
            size = le.data.struct.Size
            data = pe.get_data(rva, size)
            icon_data[e.id] = data

# Read Group Icon to get the metadata
import struct

for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
    if entry.id == 14:
        for e in entry.directory.entries:
            for le in e.directory.entries:
                rva = le.data.struct.OffsetToData
                size = le.data.struct.Size
                data = pe.get_data(rva, size)
                count = struct.unpack("<H", data[4:6])[0]
                print(f"Group icon has {count} entries:")
                for i in range(count):
                    eo = 6 + i * 14
                    chunk = data[eo:eo + 14]
                    if len(chunk) < 14:
                        break
                    w, h, colors, res, planes, bpp, esize, eid = struct.unpack(
                        "<BBBBHHIH", chunk
                    )
                    aw = w if w > 0 else 256
                    ah = h if h > 0 else 256
                    if eid in icon_data:
                        png = icon_data[eid]
                        out = OUT_DIR / f"devwhisp_{aw}x{ah}.png"
                        out.write_bytes(png)
                        print(f"  {aw}x{ah}: {len(png)} bytes -> {out}")
                    else:
                        print(f"  {aw}x{ah}: MISSING RT_ICON resource id={eid}")
