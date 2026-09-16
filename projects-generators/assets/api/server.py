"""Hello-world API. Storage is supplied by the fixture's store module."""

import json
import os
import time
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

import store


class Handler(BaseHTTPRequestHandler):
    def respond(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def dispatch(self):
        try:
            path = urlsplit(self.path).path
            if self.command == "GET" and path == "/health":
                if os.getenv("FAIL_READINESS") == "1":
                    self.respond(503, {"error": "injected readiness failure"})
                    return
                store.health()
                config = json.loads(Path(__file__).with_name("config.json").read_text())
                self.respond(200, {"status": "ready", "app": config["app"]})
                return
            message = None
            if self.command in {"PUT", "POST"}:
                length = int(self.headers.get("Content-Length", "0"))
                if not 1 <= length <= 4096:
                    raise ValueError("body must contain 1–4096 bytes")
                payload = json.loads(self.rfile.read(length))
                message = payload.get("message") if isinstance(payload, dict) else None
                if not isinstance(message, str) or not 1 <= len(message.strip()) <= 200:
                    raise ValueError("message must contain 1–200 characters")
                message = message.strip()
            status, payload = store.request(self.command, path, message)
            self.respond(status, payload)
        except (ValueError, UnicodeDecodeError) as error:
            self.respond(400, {"error": str(error)})
        except Exception:
            traceback.print_exc()
            self.respond(503, {"error": "dependency unavailable; inspect service logs"})

    do_GET = dispatch
    do_PUT = dispatch
    do_POST = dispatch


if __name__ == "__main__":
    time.sleep(float(os.getenv("STARTUP_DELAY", "0")))
    ThreadingHTTPServer(("0.0.0.0", int(os.getenv("PORT", "8000"))), Handler).serve_forever()
