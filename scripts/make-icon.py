#!/usr/bin/env python3
"""Draw the Pointcast tray icon: warm black tile, amber rec bead, a tiny frame."""
from PIL import Image, ImageDraw

SIZE = 1024
img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
m = 48
d.rounded_rectangle([m, m, SIZE - m, SIZE - m], radius=220, fill=(14, 13, 11, 255))
# screenshot frame
d.rounded_rectangle([220, 260, 804, 700], radius=48, outline=(244, 239, 230, 255), width=28)
d.rectangle([280, 330, 744, 620], fill=(31, 28, 22, 255))
# amber rec bead
d.ellipse([430, 400, 594, 564], fill=(245, 165, 36, 255))
# pointer
d.polygon([(640, 520), (720, 700), (660, 680), (700, 780), (640, 760), (600, 680)], fill=(143, 211, 232, 255))
img.save("/workspace/src-tauri/icons/app-icon.png")
print("wrote src-tauri/icons/app-icon.png")
