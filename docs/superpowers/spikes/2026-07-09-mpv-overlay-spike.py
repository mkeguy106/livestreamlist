#!/usr/bin/env python3
# Spike: does mpv --wid composite correctly when its surface lives inside the
# app's gtk::Fixed overlay-pass-through layer, on TOP of the React webview?
# Replicates src-tauri/src/embed.rs::install_overlay exactly:
#   GtkApplicationWindow -> vbox -> GtkOverlay
#     base child:    WebKitWebView (React stand-in: a labelled gradient)
#     overlay child: gtk::Fixed, set_overlay_pass_through(True)
#       -> N GtkDrawingArea column slots; mpv --wid into each.
# If the video panels appear ABOVE the gradient in their column slots and are
# hardware-decoded (nvdec), the architecture is viable.
import os
import subprocess
import sys

os.environ.setdefault("GDK_BACKEND", "x11")
# Match the app: dmabuf renderer disabled (WebKit UI renders in software).
os.environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, WebKit2, GLib, GdkX11  # noqa: F401

here = os.path.dirname(os.path.abspath(__file__))
names = ["jinnytty", "stormfall33", "lydiaviolet", "nagzz"]
COLS = len(names)

win = Gtk.ApplicationWindow(title="mpv-in-overlay spike (video panels should be ON TOP of the gradient)")
win.set_default_size(340 * COLS, 620)
win.connect("destroy", Gtk.main_quit)

vbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
win.add(vbox)

overlay = Gtk.Overlay()
vbox.pack_start(overlay, True, True, 0)

# Base child = React stand-in. Bright gradient + big text so it's obvious
# whether mpv draws above (good) or the panels are missing/under it (bad).
webview = WebKit2.WebView()
webview.load_html(
    "<html><body style='margin:0;height:100vh;background:linear-gradient("
    "135deg,#6d28d9,#db2777);color:#fff;font:700 26px sans-serif;"
    "display:flex;align-items:center;justify-content:center;text-align:center'>"
    "REACT UI (base webview)<br>video panels should cover the middle band"
    "</body></html>",
    "file:///",
)
overlay.add(webview)

fixed = Gtk.Fixed()
overlay.add_overlay(fixed)
overlay.set_overlay_pass_through(fixed, True)  # app-critical: input pass-through

win.show_all()

SLOT_W, SLOT_H = 320, 180
GAP = 12
procs = []


def start_mpv():
    for i, name in enumerate(names):
        da = Gtk.DrawingArea()
        da.set_size_request(SLOT_W, SLOT_H)
        fixed.put(da, GAP + i * (SLOT_W + GAP), 210)
        da.show()
        # Realize so the GdkWindow (and its XID) exists.
        da.realize()
        xid = da.get_window().get_xid()
        with open(os.path.join(here, f"mv_{name}.txt")) as f:
            url = f.read().strip()
        vo = os.environ.get("MPV_VO", "gpu")
        hwdec = os.environ.get("MPV_HWDEC", "auto")
        extra = os.environ.get("MPV_EXTRA", "").split()
        p = subprocess.Popen(
            ["mpv", "--no-config", "--mute=yes", "--no-terminal", "--really-quiet",
             f"--vo={vo}", f"--hwdec={hwdec}", "--profile=low-latency",
             *extra, f"--wid={xid}", url]
        )
        print(f"SPIKE mpv {name} xid={xid} pid={p.pid}", flush=True)
        procs.append(p)
    GLib.timeout_add_seconds(5, report)
    return False


def report():
    alive = sum(1 for p in procs if p.poll() is None)
    print(f"SPIKE alive={alive}/{len(procs)}", flush=True)
    return True


GLib.idle_add(start_mpv)
GLib.timeout_add_seconds(1800, Gtk.main_quit)
print(f"SPIKE_PID {os.getpid()}", flush=True)
Gtk.main()
for p in procs:
    p.terminate()
print("SPIKE exit", flush=True)
