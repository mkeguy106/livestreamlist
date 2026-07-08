#!/usr/bin/env python3
"""Run a video-playback command and measure CPU (process tree) + GPU (dmon).

Usage:
  measure.py --label hls_2 --duration 60 -- python3 webkit_harness.py 'file://...'
Writes results_<label>.json and console_<label>.log in cwd.
"""
import argparse
import json
import os
import signal
import subprocess
import sys
import time

CLK = os.sysconf("SC_CLK_TCK")


def proc_tree_cpu(root_pid):
    """Sum utime+stime (ticks) across root_pid and all descendants."""
    ppids = {}
    ticks = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", "rb") as f:
                data = f.read().decode("utf-8", "replace")
        except OSError:
            continue
        # comm may contain spaces/parens; split after last ')'
        rp = data.rindex(")")
        fields = data[rp + 2:].split()
        pid = int(entry)
        ppids[pid] = int(fields[1])  # ppid = field 4 overall, index 1 after comm
        ticks[pid] = int(fields[11]) + int(fields[12])  # utime, stime
    total = 0
    members = set()
    for pid in ticks:
        p = pid
        seen = set()
        while p > 1 and p not in seen:
            if p == root_pid:
                members.add(pid)
                total += ticks[pid]
                break
            seen.add(p)
            p = ppids.get(p, 1)
    return total, members


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", required=True)
    ap.add_argument("--duration", type=int, default=60)
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    cmd = args.cmd
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]

    console_log = open(f"console_{args.label}.log", "w")
    dmon_log = open(f"dmon_{args.label}.log", "w")

    dmon = subprocess.Popen(
        ["nvidia-smi", "dmon", "-s", "u", "-d", "1"],
        stdout=dmon_log, stderr=subprocess.DEVNULL,
    )
    child = subprocess.Popen(cmd, stdout=console_log, stderr=subprocess.STDOUT)

    time.sleep(3)  # let processes spawn before first sample
    t0 = time.monotonic()
    cpu0, _ = proc_tree_cpu(child.pid)
    samples = []
    prev_cpu, prev_t = cpu0, t0
    end = t0 + args.duration
    while time.monotonic() < end and child.poll() is None:
        time.sleep(2)
        now = time.monotonic()
        cpu, members = proc_tree_cpu(child.pid)
        cores = (cpu - prev_cpu) / CLK / (now - prev_t)
        samples.append(round(cores, 2))
        prev_cpu, prev_t = cpu, now
    t1 = time.monotonic()
    cpu1, members = proc_tree_cpu(child.pid)

    result = {
        "label": args.label,
        "wall_s": round(t1 - t0, 1),
        "avg_cores": round((cpu1 - cpu0) / CLK / (t1 - t0), 2),
        "peak_cores_2s": max(samples) if samples else None,
        "samples": samples,
        "n_procs": len(members),
    }

    child.send_signal(signal.SIGTERM)
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.kill()
    dmon.terminate()
    console_log.close()
    dmon_log.close()

    with open(f"results_{args.label}.json", "w") as f:
        json.dump(result, f, indent=1)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
