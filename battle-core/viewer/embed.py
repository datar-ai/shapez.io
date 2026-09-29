"""Bake a replay into the viewer so it opens with no file picker.

    python3 viewer/embed.py out/replay.json out/replay.html
"""
import sys

src, out = sys.argv[1], sys.argv[2]
page = open(__file__.replace("embed.py", "index.html"), encoding="utf-8").read()
data = open(src, encoding="utf-8").read().replace("</", "<\\/")
slot = '<script id="replay-data" type="application/json"></script>'
page = page.replace(slot, slot.replace("></", ">" + data + "</"))
open(out, "w", encoding="utf-8").write(page)
