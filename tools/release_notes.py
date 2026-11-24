"""Prints the CHANGELOG.md section for one version, for the release page."""
import pathlib
import re
import sys

version = sys.argv[1] if len(sys.argv) > 1 else ""
text = pathlib.Path(__file__).resolve().parent.parent.joinpath("CHANGELOG.md").read_text(encoding="utf-8")
sections = re.split(r"(?m)^## ", text)
for s in sections:
    head = s.splitlines()[0] if s else ""
    if version and version in head:
        body = "\n".join(s.splitlines()[1:]).strip()
        print(body)
        print()
        print("Downloads: Windows (.exe), macOS (.dmg, Apple Silicon and Intel), Linux (x86_64). See the README for first launch notes.")
        break
else:
    print(f"RoomGuard {version}")
