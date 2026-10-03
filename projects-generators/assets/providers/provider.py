"""Language-neutral event protocol sample using only Python's standard library."""

import copy
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import signal
import sqlite3
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
import uuid


class Provider:
    def __init__(self, state, template, origin, token, batch_size=1):
        if not 1 <= batch_size <= 100:
            raise ValueError("TANDEM_PROVIDER_BATCH must be between 1 and 100")
        self.origin = origin.rstrip("/")
        self.token = token
        self.templates = template if isinstance(template, list) else [template]
        if not self.templates:
            raise ValueError("sample.json must contain at least one event")
        self.batch_size = batch_size
        self.enabled_streams = {event["stream"] for event in self.templates}
        self.db = sqlite3.connect(state)
        self.db.execute("CREATE TABLE IF NOT EXISTS checkpoint (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        self.db.execute("CREATE TABLE IF NOT EXISTS feedback (id INTEGER PRIMARY KEY, payload TEXT NOT NULL)")
        with self.db:
            self.db.execute("INSERT OR IGNORE INTO checkpoint VALUES ('epoch', ?)", (uuid.uuid4().hex,))
            self.db.execute("INSERT OR IGNORE INTO checkpoint VALUES ('sequence', '0')")

    def value(self, key):
        row = self.db.execute("SELECT value FROM checkpoint WHERE key = ?", (key,)).fetchone()
        return row[0] if row else None

    def request(self, path, payload=None):
        body = json.dumps(payload).encode() if payload is not None else None
        request = Request(self.origin + path, data=body, headers={
            "Authorization": "Bearer " + self.token, "Content-Type": "application/json",
        })
        with urlopen(request, timeout=5) as response:
            content = response.read(1_048_576)
            return json.loads(content) if content else None

    def deliver(self):
        pending = self.value("pending")
        if pending is None:
            sequence = int(self.value("sequence"))
            epoch = self.value("epoch")
            events = []
            if not self.enabled_streams:
                return
            for offset in range(self.batch_size * len(self.templates)):
                event_sequence = sequence + offset
                cycle, position = divmod(event_sequence, len(self.templates))
                if self.templates[position]["stream"] not in self.enabled_streams:
                    continue
                event = copy.deepcopy(self.templates[position])
                event["event_id"] = f"{event['profile']}:{epoch}:{event_sequence}"
                event["occurred_at"] = datetime.now(timezone.utc).isoformat()
                event["metadata"]["sample_sequence"] = event_sequence
                event["metadata"]["stream_sequence"] = (
                    cycle * sum(sample["stream"] == event["stream"] for sample in self.templates)
                    + sum(sample["stream"] == event["stream"] for sample in self.templates[:position + 1])
                )
                events.append(event)
                if len(events) == self.batch_size:
                    break
            pending = json.dumps({"events": events})
            with self.db:
                self.db.execute("INSERT INTO checkpoint VALUES ('pending', ?)", (pending,))
        batch = json.loads(pending)
        response = self.request("/v1/events", batch)
        receipts = response.get("receipts", [])
        discarded = response.get("discarded", [])
        pending_ids = [event["event_id"] for event in batch["events"]]
        received = [receipt.get("event_id") for receipt in receipts]
        if (len(received) + len(discarded) != len(pending_ids)
                or set(received) & set(discarded)
                or set(received) | set(discarded) != set(pending_ids)
                or received != [identifier for identifier in pending_ids if identifier in received]
                or discarded != [identifier for identifier in pending_ids if identifier in discarded]):
            raise ValueError("receipt IDs do not match the pending batch")
        with self.db:
            self.db.execute("UPDATE checkpoint SET value = ? WHERE key = 'sequence'",
                             (str(max(int(self.value("sequence")),
                                      max(event["metadata"]["sample_sequence"] for event in batch["events"]) + 1)),))
            self.db.execute("DELETE FROM checkpoint WHERE key = 'pending'")
        outcome = "discarded" if discarded else "delivered"
        print(f"{outcome} {len(batch['events'])} {self.templates[0]['profile']} events", flush=True)

    def receive_controls(self):
        controls = self.request("/v1/streams")["streams"]
        known = {event["stream"] for event in self.templates}
        for control in controls:
            stream = control["stream"]
            if stream not in known or type(control["enabled"]) is not bool:
                raise ValueError("unsupported stream control")
            if control["enabled"]:
                self.enabled_streams.add(stream)
            else:
                self.enabled_streams.discard(stream)
                self.drop_pending_stream(stream)
            # Acknowledge only after this single-threaded collector has stopped stream work.
            self.request("/v1/streams/ack", control)

    def drop_pending_stream(self, stream):
        pending = self.value("pending")
        if pending is None:
            return
        batch = json.loads(pending)
        events = batch["events"]
        remaining = [event for event in events if event["stream"] != stream]
        if len(remaining) == len(events):
            return
        with self.db:
            self.db.execute("UPDATE checkpoint SET value = ? WHERE key = 'sequence'",
                            (str(max(int(self.value("sequence")),
                                     max(event["metadata"]["sample_sequence"] for event in events) + 1)),))
            if remaining:
                self.db.execute("UPDATE checkpoint SET value = ? WHERE key = 'pending'",
                                (json.dumps({"events": remaining}),))
            else:
                self.db.execute("DELETE FROM checkpoint WHERE key = 'pending'")

    def receive_feedback(self):
        for notification in self.request("/v1/notifications")["notifications"]:
            identifier = notification["notification_id"]
            with self.db:
                inserted = self.db.execute("INSERT OR IGNORE INTO feedback VALUES (?, ?)",
                                           (identifier, json.dumps(notification))).rowcount
            if inserted:
                print(f"feedback {notification['kind']} event={notification['event_id']} attempt={notification['attempt_id']}", flush=True)
            self.request(f"/v1/notifications/{identifier}/ack", {})

    def close(self):
        self.db.close()


def stop(_signum, _frame):
    raise SystemExit(0)


def main():
    signal.signal(signal.SIGTERM, stop)
    token = Path(os.environ["TANDEM_PROVIDER_TOKEN_FILE"]).read_text().strip()
    if len(token) != 64:
        raise ValueError("TANDEM_PROVIDER_TOKEN_FILE must contain a provider credential")
    state = Path(os.environ.get("TANDEM_PROVIDER_STATE", "/state"))
    state.mkdir(parents=True, exist_ok=True)
    template = json.loads(Path("sample.json").read_text(encoding="utf-8"))
    interval = float(os.environ.get("TANDEM_PROVIDER_INTERVAL", "3"))
    if interval < 0.1:
        raise ValueError("TANDEM_PROVIDER_INTERVAL must be at least 0.1 seconds")
    provider = Provider(state / "provider.sqlite3", template,
                        os.environ.get("TANDEM_EVENTS_URL", "http://127.0.0.1:7350"), token,
                        int(os.environ.get("TANDEM_PROVIDER_BATCH", "1")))
    next_event = 0
    try:
        while True:
            try:
                provider.receive_controls()
                if time.monotonic() >= next_event:
                    provider.deliver()
                    next_event = time.monotonic() + interval
                provider.receive_feedback()
            except HTTPError as error:
                print(f"sidecar HTTP {error.code}; retaining checkpoint for retry", flush=True)
                time.sleep(2)
            except (URLError, TimeoutError, OSError, ValueError) as error:
                print(f"sidecar unavailable ({type(error).__name__}); retaining checkpoint for retry", flush=True)
                time.sleep(2)
            time.sleep(0.25)
    finally:
        provider.close()


if __name__ == "__main__":
    main()
