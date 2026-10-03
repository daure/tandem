from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest

from test_providers import SOURCE, create_providers


class CollectorControlTests(unittest.TestCase):
    def test_process_stops_stream_work_keeps_control_polling_and_resumes_live_only(self):
        condition = threading.Condition()
        controls = {
            name: {"stream": name, "enabled": True, "revision": 1}
            for name in ["messages", "reactions"]
        }
        events = []
        acknowledgments = []

        class Receiver(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def respond(self, value, status=200):
                body = json.dumps(value).encode() if value is not None else b""
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                if self.headers.get("Authorization") != "Bearer " + "a" * 64:
                    return self.respond(None, 401)
                with condition:
                    value = {"streams": list(controls.values())} if self.path == "/v1/streams" else {"notifications": []}
                    self.respond(value)

            def do_POST(self):
                if self.headers.get("Authorization") != "Bearer " + "a" * 64:
                    return self.respond(None, 401)
                payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                with condition:
                    if self.path == "/v1/streams/ack":
                        acknowledgments.append(payload)
                        self.respond(None, 204)
                    else:
                        events.extend(payload["events"])
                        self.respond({"receipts": [{"event_id": event["event_id"]} for event in payload["events"]]})
                    condition.notify_all()

        server = ThreadingHTTPServer(("127.0.0.1", 0), Receiver)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()

        def wait_for(predicate):
            with condition:
                self.assertTrue(condition.wait_for(predicate, timeout=5), "Collector did not reach the expected state")

        def acknowledged(stream, revision):
            return any(control["stream"] == stream and control["revision"] == revision for control in acknowledgments)

        process = None
        try:
            with tempfile.TemporaryDirectory(prefix="tandem-stream-controls-") as directory:
                root = Path(directory)
                package = create_providers(root) / "slack"
                token = root / "provider.token"
                token.write_text("a" * 64)
                process = subprocess.Popen(
                    [sys.executable, "-u", str(SOURCE)], cwd=package,
                    env={**os.environ, "TANDEM_PROVIDER_TOKEN_FILE": str(token),
                         "TANDEM_PROVIDER_STATE": str(root / "state"),
                         "TANDEM_EVENTS_URL": f"http://127.0.0.1:{server.server_port}",
                         "TANDEM_PROVIDER_INTERVAL": "0.1", "TANDEM_PROVIDER_BATCH": "4"},
                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
                )
                wait_for(lambda: {event["stream"] for event in events} == {"messages", "reactions"})
                with condition:
                    controls["messages"] = {"stream": "messages", "enabled": False, "revision": 2}
                wait_for(lambda: acknowledged("messages", 2))
                with condition:
                    cutoff = len(events)
                wait_for(lambda: len(events) >= cutoff + 8)
                with condition:
                    self.assertTrue(all(event["stream"] == "reactions" for event in events[cutoff:]))
                    controls["reactions"] = {"stream": "reactions", "enabled": False, "revision": 2}
                wait_for(lambda: acknowledged("reactions", 2))
                with condition:
                    stopped_count = len(events)
                    applied_count = len(acknowledgments)
                wait_for(lambda: len(acknowledgments) >= applied_count + 6)
                with condition:
                    self.assertEqual(len(events), stopped_count)
                    resumed_at = datetime.now(timezone.utc)
                    controls["messages"] = {"stream": "messages", "enabled": True, "revision": 3}
                self.assertIsNone(process.poll())
                wait_for(lambda: acknowledged("messages", 3) and len(events) >= stopped_count + 8)
                with condition:
                    resumed = events[stopped_count:]
                    self.assertTrue(all(event["stream"] == "messages" for event in resumed))
                    self.assertTrue(all(datetime.fromisoformat(event["occurred_at"]) >= resumed_at for event in resumed))
                    self.assertEqual(len({event["event_id"] for event in events}), len(events))
                process.terminate()
                _output, errors = process.communicate(timeout=3)
                self.assertEqual(process.returncode, 0, errors)
        finally:
            if process is not None and process.poll() is None:
                process.kill()
                process.communicate()
            server.shutdown()
            server.server_close()
            thread.join(timeout=3)
