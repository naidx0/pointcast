#!/usr/bin/env python3
"""Regenerate Pointcast icons from the Sequence Nova mark (icons/icon.png)."""
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src-tauri" / "icons" / "icon.png"
OUT = ROOT / "src-tauri" / "icons"

src = Image.open(SRC).convert("RGBA")
src.save(OUT / "app-icon.png")

sizes = {
    "32x32.png": 32,
    "64x64.png": 64,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}
for name, s in sizes.items():
    src.resize((s, s), Image.Resampling.LANCZOS).save(OUT / name)

icos = [src.resize((s, s), Image.Resampling.LANCZOS) for s in (16, 32, 48, 64, 128, 256)]
icos[0].save(OUT / "icon.ico", format="ICO", sizes=[(i.width, i.height) for i in icos], append_images=icos[1:])
print("wrote Pointcast icons from Sequence Nova mark")
