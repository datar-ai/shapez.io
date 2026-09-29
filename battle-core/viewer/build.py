"""Build a self-contained page from viewer/index.html.

    python3 viewer/build.py out/battle.html                  # playable: the core runs in the page
    python3 viewer/build.py out/replay.html out/r1.json      # a recorded battle, opens without a file picker

The playable page needs the WebAssembly build of the core; this script runs
`cargo build --release --lib --target wasm32-unknown-unknown` first
(install the target once with `rustup target add wasm32-unknown-unknown`).
"""
import base64
import os
import subprocess
import sys

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(here)
out = sys.argv[1]
replay = sys.argv[2] if len(sys.argv) > 2 else None

page = open(os.path.join(here, "index.html"), encoding="utf-8").read()


def fill(page, slot_id, data):
    slot = f'<script id="{slot_id}" type="'
    start = page.index(slot)
    end = page.index("</script>", start)
    head = page[start:page.index(">", start) + 1]
    return page[:start] + head + data + page[end:]


if replay:
    data = open(replay, encoding="utf-8").read().replace("</", "<\\/")
    page = fill(page, "replay-data", data)
else:
    subprocess.run(
        ["cargo", "build", "--release", "--lib", "--target", "wasm32-unknown-unknown"],
        cwd=root,
        check=True,
    )
    wasm = os.path.join(root, "target", "wasm32-unknown-unknown", "release", "battlecore.wasm")
    page = fill(page, "wasm-data", base64.b64encode(open(wasm, "rb").read()).decode())

os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
open(out, "w", encoding="utf-8").write(page)
print(f"wrote {out} ({len(page) // 1024} KB)")
