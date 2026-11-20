"""Tiny reverse proxy: one public port (Railway gives a service one domain).
  /relayer/<path> -> relayer   (127.0.0.1:8787/<path>)
  /api/<path>     -> thesis API (127.0.0.1:8790/<path>)
  /health         -> this proxy + both upstreams
The client's X-Forwarded-For (set by the platform edge) is passed through
unchanged, so the relayer's per-IP limits see the real client."""
import http.client
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROUTES = {"/relayer": ("127.0.0.1", 8787), "/api": ("127.0.0.1", 8790)}
HOP = {"connection", "keep-alive", "transfer-encoding", "upgrade", "proxy-connection", "te", "trailer", "host", "content-length"}


def upstream_ok(host, port, path):
    try:
        c = http.client.HTTPConnection(host, port, timeout=5)
        c.request("GET", path)
        return c.getresponse().status == 200
    except Exception:
        return False


class Proxy(BaseHTTPRequestHandler):
    server_version = "constella-proxy"

    def log_message(self, fmt, *args):
        pass  # upstreams log; never log bodies

    def _route(self):
        for prefix, target in ROUTES.items():
            if self.path == prefix or self.path.startswith(prefix + "/"):
                return target, self.path[len(prefix):] or "/"
        return None, None

    def _forward(self):
        if self.path == "/health":
            body = json.dumps({"ok": True, "relayer": upstream_ok("127.0.0.1", 8787, "/health"),
                               "thesis_api": upstream_ok("127.0.0.1", 8790, "/health")}).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        target, path = self._route()
        if not target:
            self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers()
            return
        n = int(self.headers.get("Content-Length") or 0)
        data = self.rfile.read(n) if n > 0 else None
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
        try:
            c = http.client.HTTPConnection(*target, timeout=120)
            c.request(self.command, path, body=data, headers=headers)
            r = c.getresponse()
            payload = r.read()
        except Exception:
            self.send_response(502); self.send_header("Content-Length", "0"); self.end_headers()
            return
        self.send_response(r.status)
        for k, v in r.getheaders():
            if k.lower() not in HOP:
                self.send_header(k, v)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = do_POST = do_OPTIONS = _forward


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", int(os.environ.get("PORT", "8080"))), Proxy).serve_forever()
