#!/usr/bin/env python3
"""Embed N mpv instances into a GTK3 window via --wid (X11 child windows).

Usage: wid_harness.py <seconds> <name>... (reads url_<name>.txt)
Prints SPIKE_WID lines to stdout.
"""
import os
import subprocess
import sys

os.environ.setdefault("GDK_BACKEND", "x11")

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib, GdkX11  # noqa: E402,F401

here = os.path.dirname(os.path.abspath(__file__))
seconds = int(sys.argv[1])
names = sys.argv[2:]

win = Gtk.Window(title="mpv wid spike")
win.set_default_size(440 * len(names), 260)
box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=2)
win.add(box)

areas = []
for name in names:
    da = Gtk.DrawingArea()
    da.set_size_request(430, 250)
    box.pack_start(da, True, True, 0)
    areas.append((name, da))

procs = []


def start_mpv():
    for name, da in areas:
        xid = da.get_window().get_xid()
        with open(os.path.join(here, f"url_{name}.txt")) as f:
            url = f.read().strip()
        p = subprocess.Popen(
            ["mpv", "--no-config", "--mute=yes", "--no-terminal", "--really-quiet",
             "--hwdec=auto", f"--wid={xid}", url])
        print(f"SPIKE_WID spawned {name} xid={xid} pid={p.pid}", flush=True)
        procs.append(p)
    GLib.timeout_add_seconds(5, report)
    return False


def report():
    alive = sum(1 for p in procs if p.poll() is None)
    print(f"SPIKE_WID alive={alive}/{len(procs)}", flush=True)
    return True


win.connect("destroy", Gtk.main_quit)
win.show_all()
GLib.idle_add(start_mpv)
GLib.timeout_add_seconds(seconds, Gtk.main_quit)
Gtk.main()
for p in procs:
    p.terminate()
print("SPIKE_WID exit", flush=True)
