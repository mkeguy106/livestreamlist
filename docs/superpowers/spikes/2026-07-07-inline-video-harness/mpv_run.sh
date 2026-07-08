#!/usr/bin/env bash
# Spawn one mpv per url file given; clean up children on TERM.
# Usage: mpv_run.sh <hwdec:auto|no> <name>...
set -u
hwdec="$1"; shift
cd "$(dirname "$0")"
pids=()
for name in "$@"; do
  mpv --no-config --mute=yes --no-terminal --really-quiet --hwdec="$hwdec" \
      --geometry=640x360 --title="spike-mpv-$name" \
      "$(cat "url_$name.txt")" &
  pids+=($!)
done
trap 'kill "${pids[@]}" 2>/dev/null' TERM INT
wait
