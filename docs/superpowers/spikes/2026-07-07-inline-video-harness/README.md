Spike harness — see ../2026-07-07-inline-video-playback-spike.md

Setup: curl -sL https://cdn.jsdelivr.net/npm/hls.js@1.6.7/dist/hls.min.js -o hls.min.js
Serve: python3 -m http.server 8807 --bind 127.0.0.1 &  +  python3 proxy.py &
URLs:  streamlink --stream-url twitch.tv/<chan> 720p60,720p,best > url_<chan>.txt
Run:   python3 measure.py --label x --duration 60 -- python3 webkit_harness.py "$(python3 mkurl.py chan1 chan2)&stagger=-1&rebuild=1" 75

Hybrid (candidate D) additions:
  tsspike.html — mpegts.js N-stream page (frames-based watchdog, ?stagger=ms)
  sl_serve.sh <chan>... — one streamlink --player-external-http per channel, ports 8901+
  sample_sl.py <sec> — CPU/RSS sampler for streamlink procs
  mpegts.js: curl -sL https://cdn.jsdelivr.net/npm/mpegts.js@1.8.0/dist/mpegts.js -o mpegts.js
  Route video URLs through proxy.py (adds the ACAO header streamlink lacks).
