#!/usr/bin/env python3
"""Minimal WebKitGTK 4.1 window mirroring the Tauri app's environment.

Usage: webkit_harness.py <url> [seconds]
Console output goes to stdout via enable-write-console-messages-to-stdout.
"""
import os
import sys

# Mirror lib.rs::apply_linux_webkit_workarounds BEFORE gtk init
if os.environ.get("HARNESS_NO_DMABUF_WORKAROUND") == "1":
    os.environ.pop("WEBKIT_DISABLE_DMABUF_RENDERER", None)
else:
    os.environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
os.environ.setdefault("GDK_BACKEND", "x11")

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, WebKit2, GLib  # noqa: E402

url = sys.argv[1]
seconds = int(sys.argv[2]) if len(sys.argv) > 2 else 0

win = Gtk.Window(title="hls spike harness")
win.set_default_size(1760, 800)

view = WebKit2.WebView()
s = view.get_settings()
s.set_enable_write_console_messages_to_stdout(True)
s.set_media_playback_requires_user_gesture(False)
s.set_enable_media_stream(True)
s.set_enable_mediasource(True)
s.set_enable_webgl(True)
s.set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.ALWAYS)

win.add(view)
win.connect("destroy", Gtk.main_quit)
view.load_uri(url)
win.show_all()

if seconds:
    GLib.timeout_add_seconds(seconds, Gtk.main_quit)

print(f"HARNESS_PID {os.getpid()}", flush=True)
Gtk.main()
print("HARNESS_EXIT", flush=True)
