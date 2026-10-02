import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import select
import signal
import sqlite3
import subprocess
import tempfile
import sys
import unittest
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from providers import PROFILES, create_providers, sample

SOURCE = Path(__file__).resolve().parents[1] / "assets/providers/provider.py"
SPEC = importlib.util.spec_from_file_location("provider_fixture", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="tandem-provider-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def test_each_profile_has_an_independent_build_context_and_shared_context(self):
        directory = create_providers(self.root)
        model = json.loads((directory / "compose.providers.yaml").read_text())
        self.assertEqual(len(model["services"]), 4)
        for profile in PROFILES:
            name = profile.replace("_", "-")
            package = directory / name
            self.assertTrue((package / "Dockerfile").is_file())
            self.assertEqual((package / "src/provider.py").read_bytes(), SOURCE.read_bytes())
            event = json.loads((package / "sample.json").read_text())
            self.assertEqual(event["profile"], profile)
            self.assertTrue(event["attachments"] and event["people"] and event["relations"])
            self.assertEqual(model["services"][name]["build"]["context"], f"./{name}")
            self.assertNotIn("ports", model["services"][name])
        with self.assertRaisesRegex(ValueError, "Refusing to overwrite"):
            create_providers(self.root)

    def test_unacknowledged_delivery_keeps_identical_ids_and_payload_across_restart(self):
        state = self.root / "state.sqlite3"
        provider = MODULE.Provider(state, sample("message"), "http://localhost", "unused", 10)
        batches = []

        def lose_receipt(path, payload=None):
            batches.append(payload)
            raise URLError("lost receipt")

        provider.request = lose_receipt
        with self.assertRaises(URLError):
            provider.deliver()
        provider.close()
        provider = MODULE.Provider(state, sample("message"), "http://localhost", "unused", 10)
        self.addCleanup(provider.close)

        def acknowledge(path, payload=None):
            batches.append(payload)
            return {"receipts": [{"event_id": event["event_id"]} for event in payload["events"]]}

        provider.request = acknowledge
        with contextlib.redirect_stdout(io.StringIO()):
            provider.deliver()
        self.assertEqual(batches[0], batches[1])
        self.assertEqual(len({event["event_id"] for event in batches[1]["events"]}), 10)
        self.assertEqual(provider.value("sequence"), "10")
        self.assertIsNone(provider.value("pending"))

    def test_feedback_is_persisted_once_and_repeated_delivery_is_acknowledged(self):
        provider = MODULE.Provider(self.root / "state.sqlite3", sample("ticket"), "http://localhost", "unused")
        self.addCleanup(provider.close)
        requests = []
        notification = {"notification_id": 7, "event_id": "ticket-1", "attempt_id": 9, "kind": "received"}

        def request(path, payload=None):
            requests.append(path)
            return {"notifications": [notification]} if path == "/v1/notifications" else None

        provider.request = request
        with contextlib.redirect_stdout(io.StringIO()) as output:
            provider.receive_feedback()
            provider.receive_feedback()
        self.assertEqual(output.getvalue().count("feedback received"), 1)
        self.assertEqual(provider.db.execute("SELECT count(*) FROM feedback").fetchone()[0], 1)
        self.assertEqual(requests.count("/v1/notifications/7/ack"), 2)

    def test_discarded_batches_advance_the_checkpoint_and_are_not_retried(self):
        provider = MODULE.Provider(self.root / "state.sqlite3", sample("message"), "http://localhost", "unused", 10)
        self.addCleanup(provider.close)
        batches = []

        def discard(path, payload=None):
            batches.append(payload)
            return {"receipts": [], "discarded": [event["event_id"] for event in payload["events"]]}

        provider.request = discard
        with contextlib.redirect_stdout(io.StringIO()) as output:
            provider.deliver()
            self.assertIsNone(provider.value("pending"))
            provider.deliver()
        self.assertEqual(provider.value("sequence"), "20")
        self.assertIsNone(provider.value("pending"))
        self.assertTrue(set(event["event_id"] for event in batches[0]["events"]).isdisjoint(
            event["event_id"] for event in batches[1]["events"]))
        self.assertEqual(output.getvalue().count("discarded 10 message events"), 2)

    def test_mismatched_discard_acknowledgments_keep_the_pending_batch(self):
        for response in [
            {"receipts": [], "discarded": ["foreign-event"]},
            {"receipts": [{"event_id": "foreign-event"}], "discarded": ["foreign-event"]},
            {"receipts": [], "discarded": []},
        ]:
            with self.subTest(response=response):
                provider = MODULE.Provider(self.root / "state.sqlite3", sample("message"), "http://localhost", "unused")
                try:
                    provider.request = lambda path, payload=None: response
                    with self.assertRaises(ValueError):
                        provider.deliver()
                    self.assertEqual(provider.value("sequence"), "0")
                    self.assertIsNotNone(provider.value("pending"))
                finally:
                    provider.close()

    @unittest.skipUnless(os.name == "posix", "Requires Unix signals")
    def test_sigterm_interrupts_a_pending_request_and_closes_the_checkpoint_cleanly(self):
        package = create_providers(self.root) / "message"
        token = self.root / "provider.token"
        token.write_text("a" * 64)
        state = self.root / "state"
        script = """
import importlib.util, sys, time
spec = importlib.util.spec_from_file_location("fixture", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
def request(self, path, payload=None):
    print("request-ready", flush=True)
    time.sleep(60)
close = module.Provider.close
def close_checkpoint(self):
    close(self)
    print("checkpoint-closed", flush=True)
module.Provider.request = request
module.Provider.close = close_checkpoint
module.main()
"""
        process = subprocess.Popen(
            [sys.executable, "-u", "-c", script, str(SOURCE)], cwd=package,
            env={**os.environ, "TANDEM_PROVIDER_TOKEN_FILE": str(token),
                 "TANDEM_PROVIDER_STATE": str(state)},
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            self.assertTrue(select.select([process.stdout], [], [], 5)[0], "Provider did not reach its request")
            self.assertEqual(process.stdout.readline().strip(), "request-ready")
            process.send_signal(signal.SIGTERM)
            output, error = process.communicate(timeout=1)
            self.assertEqual(process.returncode, 0, error)
            self.assertIn("checkpoint-closed", output)
            with sqlite3.connect(state / "provider.sqlite3") as database:
                self.assertIsNotNone(database.execute("SELECT value FROM checkpoint WHERE key = 'pending'").fetchone())
                self.assertEqual(database.execute("SELECT value FROM checkpoint WHERE key = 'sequence'").fetchone()[0], "0")
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate()
