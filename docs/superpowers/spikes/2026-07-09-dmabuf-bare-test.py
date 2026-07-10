#!/usr/bin/env python3
# Minimal bare WebKitGTK window, dmabuf/GL renderer ENABLED (we do NOT set
# WEBKIT_DISABLE_DMABUF_RENDERER). No Tauri, no wry, no video — just a solid
# red page. If this shows RED, WebKit can GPU-render on this box and the black
# window is Tauri/wry-specific. If it's BLACK, dmabuf rendering is broken here
# regardless of Tauri.
import os
# Match the app's backend, but leave dmabuf ENABLED.
os.environ.setdefault("GDK_BACKEND", "x11")
os.environ.pop("WEBKIT_DISABLE_DMABUF_RENDERER", None)

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, WebKit2

win = Gtk.Window(title="DMABUF BARE TEST (should be solid RED)")
win.set_default_size(700, 400)
win.connect("destroy", Gtk.main_quit)
view = WebKit2.WebView()
view.get_settings().set_hardware_acceleration_policy(
    WebKit2.HardwareAccelerationPolicy.ALWAYS
)
view.load_html(
    "<html><body style='margin:0;background:#e01b1b;color:#fff;"
    "font:700 40px sans-serif;display:flex;align-items:center;"
    "justify-content:center;height:100vh'>DMABUF RENDER OK</body></html>",
    "file:///",
)
win.add(view)
win.show_all()
Gtk.main()
