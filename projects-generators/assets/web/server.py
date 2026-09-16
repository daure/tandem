"""Small development server with prefix-safe redirects and an optional API proxy."""

import functools
import json
import os
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener


class Handler(SimpleHTTPRequestHandler):
    def send_header(self, keyword, value):
        if keyword.lower() == "location" and value.startswith("/"):
            prefix = self.headers.get("X-Forwarded-Prefix", "")
            if prefix.startswith("/") and not prefix.startswith("//"):
                value = prefix.rstrip("/") + value
        super().send_header(keyword, value)

    def do_GET(self):
        if self.path == "/config.js":
            body = f"window.API_BASE = {json.dumps(os.getenv('API_BASE', 'api/'))};\n".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "about/")
            self.end_headers()
        elif self.path.startswith("/api/"):
            self.proxy()
        else:
            super().do_GET()

    def do_PUT(self):
        self.proxy()

    def do_POST(self):
        self.proxy()

    def proxy(self):
        if not self.path.startswith("/api/"):
            self.send_error(404)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 <= length <= 4096:
                raise ValueError("body too large")
            body = self.rfile.read(length) if length else None
            url = os.getenv("API_URL", "http://api:8000") + self.path[4:]
            request = Request(url, data=body, method=self.command,
                              headers={"Content-Type": "application/json"})
            opener = build_opener(ProxyHandler({}))
            try:
                response = opener.open(request, timeout=10)
            except HTTPError as error:
                response = error
            with response:
                data = response.read()
                self.send_response(response.status)
                self.send_header("Content-Type", response.headers.get("Content-Type", "application/json"))
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
        except ValueError:
            self.send_error(400, "Invalid request size")
        except OSError as error:
            self.log_error("API unavailable: %s", error)
            self.send_error(502, "API unavailable")


if __name__ == "__main__":
    directory = str(Path(__file__).parent / "public")
    handler = functools.partial(Handler, directory=directory)
    ThreadingHTTPServer(("0.0.0.0", int(os.getenv("PORT", "8000"))), handler).serve_forever()
