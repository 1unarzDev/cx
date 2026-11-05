#!/usr/bin/env bash
# Render lab: capture one concept and turn it into PNGs + contact sheets.
#   tests/lab.sh CONCEPT [VIEWS]
# VIEWS: comma list (hub,device:innovation,session:refactor,files,file-preview,palette:inno,
#        secrets,onboarding,join,edge:tranquility-innovation,help) or "all" (default).
# Optional env: CX_LAB_SIZES, CX_LAB_VARIANTS, CX_LAB_FIXTURES (small,typical,large),
#   CX_LAB_FOCUS, CX_LAB_SELECTED, CX_LAB_HOVER=x,y, CX_LAB_FRAME, CX_LAB_FONT, CX_LAB_FONT_PX.
# Prints the output directory (local-evidence/ui-lab/<concept>/round-N) on stdout.
set -euo pipefail
concept=${1:?usage: tests/lab.sh CONCEPT [VIEWS]}
views=${2:-${CX_LAB_VIEWS:-all}}
root=$(cd "$(dirname "$0")/.." && pwd)
py=${CX_LAB_PYTHON:-/home/lunarz/cx/worktrees/.venv-test/bin/python}
[ -x "$py" ] || py=python3

base="$root/local-evidence/ui-lab/$concept"
mkdir -p "$base"
n=1
while [ -e "$base/round-$n" ]; do n=$((n + 1)); done
out="$base/round-$n"
mkdir -p "$out/frames"

cd "$root"
CX_LAB_CONCEPT="$concept" CX_LAB_OUT="$out/frames" CX_LAB_VIEWS="$views" \
    cargo test --locked --quiet --bin cx ui::lab::capture::lab_capture -- \
    --ignored --exact --nocapture >"$out/capture.log" 2>&1 || {
    tail -n 30 "$out/capture.log" >&2
    echo "lab: capture failed; log in $out/capture.log" >&2
    exit 1
}
grep -E '^(lint|lab_capture)' "$out/capture.log" | tail -n 12 >&2 || true
"$py" "$root/tests/lab_png.py" "$out/frames" "$out" >"$out/render.log" 2>&1 || {
    tail -n 30 "$out/render.log" >&2
    echo "lab: png render failed; log in $out/render.log" >&2
    exit 1
}
echo "$out"
