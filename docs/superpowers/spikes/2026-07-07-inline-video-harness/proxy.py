#!/usr/bin/env python3
"""Origin-stripping localhost proxy: GET /p/<urlencoded-url> → upstream fetch.

Stands in for the Rust-side reqwest proxy the real app would use:
connection-pooled, streaming, per-request timing log for slow/error cases.
"""
import sys
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import requests

PORT = 8808
_tls = threading.local()


def session():
    if not hasattr(_tls, "s"):
        _tls.s = requests.Session()
        _tls.s.headers["User-Agent"] = "Mozilla/5.0"
    return _tls.s


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_GET(self):
        if not self.path.startswith("/p/"):
            self.send_error(404)
            return
        url = urllib.parse.unquote(self.path[3:])
        t0 = time.monotonic()
        status, sent = 0, 0
        try:
            with session().get(url, stream=True, timeout=(5, 10)) as r:
                status = r.status_code
                self.send_response(r.status_code)
                self.send_header("Content-Type", r.headers.get("Content-Type", "application/octet-stream"))
                if "Content-Length" in r.headers:
                    self.send_header("Content-Length", r.headers["Content-Length"])
                else:
                    self.send_header("Transfer-Encoding", "chunked")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.end_headers()
                chunked = "Content-Length" not in r.headers
                for chunk in r.iter_content(65536):
                    if chunked:
                        self.wfile.write(f"{len(chunk):x}\r\n".encode())
                        self.wfile.write(chunk)
                        self.wfile.write(b"\r\n")
                    else:
                        self.wfile.write(chunk)
                    sent += len(chunk)
                if chunked:
                    self.wfile.write(b"0\r\n\r\n")
        except Exception as e:  # noqa: BLE001
            ms = (time.monotonic() - t0) * 1000
            print(f"ERR {status} {ms:.0f}ms {sent}B {type(e).__name__}: {e} :: {url[:100]}",
                  file=sys.stderr, flush=True)
            return
        ms = (time.monotonic() - t0) * 1000
        if ms > 1500 or status >= 400:
            print(f"SLOW/BAD {status} {ms:.0f}ms {sent}B :: {url[:100]}",
                  file=sys.stderr, flush=True)


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
