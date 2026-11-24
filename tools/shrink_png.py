"""Recompresses the screenshots the app saves (it writes them uncompressed)."""
import pathlib
import sys

from PIL import Image

folder = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "docs/screenshots")
for path in sorted(folder.glob("*.png")):
    img = Image.open(path).convert("RGB")
    img.save(path, optimize=True)
    print(path, img.size, path.stat().st_size // 1024, "KB")
