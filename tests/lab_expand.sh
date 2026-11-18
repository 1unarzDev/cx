#!/usr/bin/env bash
# Mid-animation frames of the attach expand (lab): tests/lab_expand.sh OUT_DIR writes
# <view>-<ms>ms.{png,txt} for the hub and a device screen (SIZES, default 120x40).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?out dir}
mkdir -p "$out"
cd "$root"
for view in hub device:innovation; do
  for ms in 0 15 30 45 55 65 75 90; do
    t=$((ms * 1000 / 90))
    d=$(mktemp -d "$out/.f.XXXX")
    CX_LAB_OUT="$d" CX_LAB_VIEWS="$view" CX_LAB_SIZES=${SIZES:-120x40} CX_LAB_VARIANTS=dark-truecolor \
      CX_LAB_EXPAND="refactor:$t" nice -n 15 cargo test --locked --quiet --bin cx ui::lab::capture::lab_capture -- --ignored --exact --nocapture >/dev/null 2>&1
    python3 tests/lab_png.py "$d" "$d/png" >/dev/null 2>&1 || true
    stem=$(echo "$view" | tr ':' '-')
    f=$(ls "$d"/*.txt | head -1); cp "$f" "$out/$stem-${ms}ms.txt"
    p=$(ls "$d"/*.png 2>/dev/null | grep -v targets | head -1 || true); [ -n "$p" ] && cp "$p" "$out/$stem-${ms}ms.png"
    rm -rf "$d"
  done
done
