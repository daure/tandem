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
        self.template = template
        self.batch_size = batch_size
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
            for offset in range(self.batch_size):
                event = copy.deepcopy(self.template)
                event["event_id"] = f"{event['profile']}:{epoch}:{sequence + offset}"
                event["occurred_at"] = datetime.now(timezone.utc).isoformat()
                event["metadata"]["sample_sequence"] = sequence + offset
                events.append(event)
            pending = json.dumps({"events": events})
            with self.db:
                self.db.execute("INSERT INTO checkpoint VALUES ('pending', ?)", (pending,))
        batch = json.loads(pending)
        response = self.request("/v1/events", batch)
        receipts = response.get("receipts", [])
        discarded = response.get("discarded", [])
        pending_ids = [event["event_id"] for event in batch["events"]]
        if discarded:
            if receipts or discarded != pending_ids:
                raise ValueError("discarded IDs do not match the pending batch")
        elif [receipt.get("event_id") for receipt in receipts] != pending_ids:
            raise ValueError("receipt IDs do not match the pending batch")
        with self.db:
            self.db.execute("UPDATE checkpoint SET value = ? WHERE key = 'sequence'",
                            (str(int(self.value("sequence")) + len(batch["events"])),))
            self.db.execute("DELETE FROM checkpoint WHERE key = 'pending'")
        outcome = "discarded" if discarded else "delivered"
        print(f"{outcome} {len(batch['events'])} {self.template['profile']} events", flush=True)

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
