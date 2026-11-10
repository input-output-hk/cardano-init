#!/usr/bin/env bash
# Render assets/demo.gif from the two VHS tapes (see demo.tape for requirements).
# Run from the repository root:  assets/render-demo.sh
set -euo pipefail

cd "$(dirname "$0")/.."

vhs assets/demo.tape
vhs assets/demo-test.tape

# Never publish a demo whose `just test` failed (the devnet run can be flaky).
if [ "$(cat assets/.demo-status 2>/dev/null)" != "0" ]; then
  echo "just test did not pass in the recording; not writing assets/demo.gif" >&2
  rm -f assets/.demo-1.mp4 assets/.demo-2.mp4 assets/.demo-status
  exit 1
fi

# Speed scene 2 up 4x, join the scenes, and encode one GIF with a shared palette.
ffmpeg -y -loglevel error \
  -i assets/.demo-1.mp4 -i assets/.demo-2.mp4 \
  -filter_complex "[1:v]setpts=PTS/4[t];[0:v][t]concat=n=2:v=1[v];[v]fps=12,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  assets/demo.gif

rm -f assets/.demo-1.mp4 assets/.demo-2.mp4 assets/.demo-status
echo "wrote assets/demo.gif"
