#!/usr/bin/env bash
# Spawn one streamlink external-http server per channel, ports 8901..
# Usage: sl_serve.sh <chan>...   (TERM/INT kills children)
set -u
port=8901
pids=()
for chan in "$@"; do
  streamlink --player-external-http --player-external-http-port "$port" \
    --twitch-low-latency --quiet \
    "twitch.tv/$chan" 720p60,720p,best &
  pids+=($!)
  echo "SL $chan port=$port pid=$!"
  port=$((port + 1))
done
trap 'kill "${pids[@]}" 2>/dev/null' TERM INT
wait
