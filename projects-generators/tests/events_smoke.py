#!/usr/bin/env python3
"""Verify four provider profiles against an isolated Tandem sidecar, optionally in Docker."""

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
from providers import PROFILES, create_providers


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
        (home / "templates/instances").mkdir()
        with socket.socket() as socket_:
            socket_.bind(("127.0.0.1", 0))
            port = socket_.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        namespace = "events-smoke-" + root.name.removeprefix("tandem-events-smoke-").replace("_", "-")
        environment = {**os.environ, "TANDEM_HOME": str(home), "TANDEM_NAMESPACE": namespace,
                       "TANDEM_EVENTS_URL": origin, "TANDEM_PROVIDER_INTERVAL": "3600",
                       "TANDEM_PROVIDER_BATCH": "10"}
        setup = subprocess.run([str(binary), "providers-setup"], env=environment, check=True,
                               text=True, capture_output=True)
        credentials = Path(setup.stdout.strip())
        assert credentials == home / "provider-credentials" / namespace / "providers.env"
        assert credentials.stat().st_mode & 0o777 == 0o600
        tokens = {f"TANDEM_PROVIDER_{profile.upper()}_TOKEN":
                  (credentials.parent / f"dev-{profile.replace('_', '-')}.token").read_text()
                  for profile in PROFILES}
        compose = ["docker", "compose", "-p", namespace + "-providers", "--env-file", str(credentials),
                   "-f", str(packages / "compose.providers.yaml")]
        log = (root / "sidecar.log").open("wb")
        server = None
        clients = []
        containers_started = False

        def start():
            return subprocess.Popen([str(binary), "serve-events", "--bind", f"127.0.0.1:{port}"],
                                    env=environment, stdout=log, stderr=log)

        def ready():
            if server.poll() is not None:
                raise RuntimeError("sidecar exited before becoming ready")
            try:
                request(origin, tokens["TANDEM_PROVIDER_MESSAGE_TOKEN"], "/v1/notifications")
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
                source = packages / "message/src/provider.py"
                specification = importlib.util.spec_from_file_location("sample_provider", source)
                module = importlib.util.module_from_spec(specification)
                specification.loader.exec_module(module)
                for profile in PROFILES:
                    package = packages / profile.replace("_", "-")
                    provider = module.Provider(root / f"{profile}.sqlite3", json.loads((package / "sample.json").read_text()),
                                               origin, tokens[f"TANDEM_PROVIDER_{profile.upper()}_TOKEN"], 10)
                    clients.append(provider)
                    provider.deliver()
                    provider.receive_feedback()
            wait_for(lambda: rows("SELECT count(*) FROM events")[0][0] == 40, "all four provider batches were not accepted")
            wait_for(lambda: rows("SELECT count(*) FROM provider_notifications WHERE acknowledged = 1")[0][0] == 40,
                     "providers did not consume and acknowledge their feedback")
            assert {json.loads(payload)["profile"] for (payload,) in rows("SELECT payload FROM events")} == set(PROFILES)
            provider, payload = rows("SELECT provider, payload FROM events ORDER BY sequence LIMIT 1")[0]
            variable = provider.removeprefix("dev-").replace("-", "_").upper()
            receipt = request(origin, tokens[f"TANDEM_PROVIDER_{variable}_TOKEN"], "/v1/events", {"events": [json.loads(payload)]})
            assert receipt["receipts"][0]["duplicate"] is True
            assert rows("SELECT count(*) FROM events")[0][0] == 40
            server.terminate()
            server.wait(timeout=10)
            server = start()
            wait_for(ready, "sidecar did not restart")
            assert rows("SELECT count(*) FROM events")[0][0] == 40
            assert rows("SELECT count(*) FROM event_attempts WHERE status = 'pending'")[0][0] == 40
            print("PASS: four profiles, 40 durable events, feedback acknowledgment, duplicate IDs, sidecar restart")
        finally:
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
