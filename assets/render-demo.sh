#!/usr/bin/env bash
# Render assets/demo.gif, assets/demo-scaffold.mp4 and assets/demo-test.mp4 from
# the two VHS tapes (see demo.tape for requirements).
# Run from the repository root:  assets/render-demo.sh
set -euo pipefail

cd "$(dirname "$0")/.."

# Must match `Set Framerate`, `Set Width`, `Set Height`, `Set Padding` and the
# `Set Theme` background in both tapes.
FPS=30
WIDTH=1680
HEIGHT=1000
PADDING=24
BACKGROUND=0x1e1e2e # Catppuccin Mocha

FRAMES=(assets/.demo-1.png assets/.demo-2.png)
cleanup() { rm -rf "${FRAMES[@]}" assets/.demo-status; }
trap cleanup EXIT
cleanup

vhs assets/demo.tape
vhs assets/demo-test.tape

# Never publish a demo whose `just test` failed (the devnet run can be flaky).
if [ "$(cat assets/.demo-status 2>/dev/null)" != "0" ]; then
  echo "just test did not pass in the recording; not writing the demo" >&2
  exit 1
fi

# The tapes output raw frames, not MP4: VHS's own MP4 encode resamples to
# 25 fps whatever `Set Framerate` says. VHS moves the frames out of its temp dir
# with rename(2), which fails silently across filesystems.
for dir in "${FRAMES[@]}"; do
  if [ ! -f "$dir/frame-text-00001.png" ]; then
    echo "VHS left no frames in $dir; set TMPDIR to a directory on the same filesystem as the repo" >&2
    exit 1
  fi
done

# Composite the text and cursor layers the way VHS does, at the recorded rate.
encode() {
  ffmpeg -y -loglevel error \
    -framerate "$FPS" -i "$1/frame-text-%05d.png" \
    -framerate "$FPS" -i "$1/frame-cursor-%05d.png" \
    -filter_complex "[0][1]overlay,scale=$((WIDTH - 2 * PADDING)):$((HEIGHT - 2 * PADDING)):force_original_aspect_ratio=1,pad=$WIDTH:$HEIGHT:(ow-iw)/2:(oh-ih)/2:$BACKGROUND,format=yuv420p" \
    -c:v libx264 -crf 20 -r "$FPS" -an "$2"
}
encode assets/.demo-1.png assets/demo-scaffold.mp4
encode assets/.demo-2.png assets/demo-test.mp4

# Speed scene 2 up 4x, join the scenes, and encode one GIF with a shared palette.
ffmpeg -y -loglevel error \
  -i assets/demo-scaffold.mp4 -i assets/demo-test.mp4 \
  -filter_complex "[1:v]setpts=PTS/4[t];[0:v][t]concat=n=2:v=1[v];[v]fps=12,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  assets/demo.gif

echo "wrote assets/demo.gif, assets/demo-scaffold.mp4, assets/demo-test.mp4"
