#!/usr/bin/env python3
"""Verify five fixture streams and six rules against an isolated sidecar, optionally in Docker."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from urllib.error import URLError
from urllib.request import Request, urlopen

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src/mcp/tests"))
from providers import PROFILES, PROVIDERS, create_providers
from stdio_smoke import Client

BATCH_SIZE = 18
EVENT_COUNT = BATCH_SIZE * len(PROVIDERS)


def wait_for(check, message, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.1)
    raise RuntimeError(message)


def request(origin, token, path, payload=None):
    data = json.dumps(payload).encode() if payload is not None else None
    with urlopen(Request(origin + path, data=data, headers={
        "Authorization": "Bearer " + token, "Content-Type": "application/json",
    }), timeout=3) as response:
        content = response.read()
        return json.loads(content) if content else None


def verify(binary, docker):
    with tempfile.TemporaryDirectory(prefix="tandem-events-smoke-") as directory:
        root = Path(directory)
        packages = create_providers(root)
        home = root / ".tandem"
        guidance = home / "templates/instances/guidance-only"
        guidance.mkdir(parents=True)
        (guidance / "tandem.json").write_text("{}")
        with socket.socket() as socket_:
            socket_.bind(("127.0.0.1", 0))
            port = socket_.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        namespace = "events-smoke-" + root.name.removeprefix("tandem-events-smoke-").replace("_", "-")
        environment = {**os.environ, "TANDEM_HOME": str(home), "TANDEM_NAMESPACE": namespace,
                       "TANDEM_EVENTS_URL": origin, "TANDEM_PROVIDER_INTERVAL": "3600",
                       "TANDEM_PROVIDER_BATCH": str(BATCH_SIZE), "XDG_STATE_HOME": str(root / "state")}
        setup = subprocess.run([str(binary), "providers-setup"], env=environment, check=True,
                               text=True, capture_output=True)
        credentials = Path(setup.stdout.strip())
        assert credentials == home / "provider-credentials" / namespace / "providers.env"
        assert credentials.stat().st_mode & 0o777 == 0o600
        tokens = {name: (credentials.parent / f"{name}.token").read_text()
                  for name in PROVIDERS.values()}
        rules = json.loads(subprocess.run(
            [str(binary), "rules-setup", "--model", "openai/test"], env=environment,
            check=True, text=True, capture_output=True,
        ).stdout)
        assert len(rules) == 6 and all(not rule["definition"]["enabled"] for rule in rules)
        compose = ["docker", "compose", "-p", namespace + "-providers", "--env-file", str(credentials),
                   "-f", str(packages / "compose.providers.yaml")]
        log = (root / "sidecar.log").open("wb")
        server = None
        clients = []
        mcp = None
        containers_started = False

        def start():
            return subprocess.Popen([str(binary), "serve-events", "--bind", f"127.0.0.1:{port}"],
                                    env=environment, stdout=log, stderr=log)

        def ready():
            if server.poll() is not None:
                raise RuntimeError("sidecar exited before becoming ready")
            try:
                request(origin, tokens["slack"], "/v1/notifications")
                return True
            except (URLError, TimeoutError):
                return False

        def rows(query):
            with sqlite3.connect(home / "settings.sqlite3") as database:
                return database.execute(query).fetchall()

        try:
            server = start()
            wait_for(ready, "sidecar did not start")
            if docker:
                containers_started = True
                subprocess.run([*compose, "up", "-d", "--build"], env=environment, check=True,
                               capture_output=True, text=True, timeout=180)
            else:
                source = packages / "slack/src/provider.py"
                specification = importlib.util.spec_from_file_location("sample_provider", source)
                module = importlib.util.module_from_spec(specification)
                specification.loader.exec_module(module)
                for profile, name in PROVIDERS.items():
                    package = packages / name
                    provider = module.Provider(root / f"{profile}.sqlite3", json.loads((package / "sample.json").read_text()),
                                               origin, tokens[name], BATCH_SIZE)
                    clients.append(provider)
                    provider.deliver()
                    provider.receive_feedback()
            wait_for(lambda: rows("SELECT count(*) FROM events")[0][0] == EVENT_COUNT, "all four provider batches were not accepted")
            wait_for(lambda: rows("SELECT count(*) FROM provider_notifications WHERE acknowledged = 1")[0][0] == EVENT_COUNT,
                     "providers did not consume and acknowledge their feedback")
            assert {json.loads(payload)["profile"] for (payload,) in rows("SELECT payload FROM events")} == set(PROFILES)
            assert {json.loads(payload)["stream"] for (payload,) in rows("SELECT payload FROM events")} == {
                "messages", "reactions", "backlog", "production-gateway-issue", "releases"}
            expected_rules = {
                ("slack", "messages"): {"slack-pr-request", "slack-support-query"},
                ("slack", "reactions"): {"slack-message-reaction"},
                ("jira", "backlog"): {"jira-ticket-triage"},
                ("datadog", "production-gateway-issue"): {"datadog-gateway-issue"},
                ("github", "releases"): {"github-release-notes"},
            }
            mcp = Client(binary, environment)
            mcp.tool("get_instructions")
            matched_sequences = {name: set() for names in expected_rules.values() for name in names}
            for sequence, provider, payload in rows("SELECT sequence, provider, payload FROM events ORDER BY sequence"):
                event = json.loads(payload)
                stream_sequence = event["metadata"]["stream_sequence"]
                expected = expected_rules[(provider, event["stream"])] if stream_sequence % 3 == 0 else set()
                matched = set()
                for rule in rules:
                    preview = mcp.tool("preview_rule", {"definition": rule["definition"], "event_sequence": sequence})
                    if preview["matched"]:
                        name = rule["definition"]["name"]
                        matched.add(name)
                        matched_sequences[name].add(stream_sequence)
                        assert event["event_id"] in preview["resolved_prompt"]
                assert matched == expected, (provider, event["stream"], stream_sequence, matched, expected)
            assert all({3, 6, 9} <= sequences for sequences in matched_sequences.values()), matched_sequences
            assert rows("SELECT count(*) FROM rule_acceptances")[0][0] == 0
            provider, payload = rows("SELECT provider, payload FROM events ORDER BY sequence LIMIT 1")[0]
            receipt = request(origin, tokens[provider], "/v1/events", {"events": [json.loads(payload)]})
            assert receipt["receipts"][0]["duplicate"] is True
            assert rows("SELECT count(*) FROM events")[0][0] == EVENT_COUNT
            server.terminate()
            server.wait(timeout=10)
            server = start()
            wait_for(ready, "sidecar did not restart")
            assert rows("SELECT count(*) FROM events")[0][0] == EVENT_COUNT
            assert rows("SELECT count(*) FROM event_attempts WHERE status = 'pending'")[0][0] == EVENT_COUNT
            print(f"PASS: four profiles, five streams, six disabled rules matching every third stream event, {EVENT_COUNT} durable events, feedback acknowledgment, duplicate IDs, sidecar restart; no model prompts")
        finally:
            if mcp is not None:
                mcp.close()
            for client in clients:
                client.close()
            try:
                if containers_started:
                    subprocess.run([*compose, "down", "--volumes", "--remove-orphans"], env=environment, check=True,
                                   capture_output=True, text=True, timeout=60)
            finally:
                if server is not None:
                    server.terminate()
                    server.wait(timeout=10)
                log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[2] / "target/debug/tandem")
    parser.add_argument("--docker", action="store_true", help="Build/start the four provider images and clean up their isolated containers/volumes")
    arguments = parser.parse_args()
    verify(arguments.binary.resolve(), arguments.docker)


if __name__ == "__main__":
    main()
