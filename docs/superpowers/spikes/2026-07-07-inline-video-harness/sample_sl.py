#!/usr/bin/env python3
"""Sample CPU (cores) + RSS of all streamlink processes for N seconds."""
import os
import sys
import time

CLK = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")


def snap():
    ticks, rss, n = 0, 0, 0
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/cmdline", "rb") as f:
                cmd = f.read().decode("utf-8", "replace")
            if "streamlink" not in cmd or "sample_sl" in cmd:
                continue
            with open(f"/proc/{entry}/stat") as f:
                data = f.read()
            fields = data[data.rindex(")") + 2:].split()
            ticks += int(fields[11]) + int(fields[12])
            rss += int(fields[21]) * PAGE
            n += 1
        except OSError:
            continue
    return ticks, rss, n


dur = int(sys.argv[1])
t0 = time.monotonic()
c0, _, _ = snap()
peak_rss, nproc = 0, 0
while time.monotonic() - t0 < dur:
    time.sleep(2)
    _, rss, n = snap()
    peak_rss = max(peak_rss, rss)
    nproc = max(nproc, n)
t1 = time.monotonic()
c1, _, _ = snap()
print(f"SL_CPU avg_cores={(c1 - c0) / CLK / (t1 - t0):.2f} "
      f"peak_rss_mb={peak_rss / 1048576:.0f} n_procs={nproc}")
