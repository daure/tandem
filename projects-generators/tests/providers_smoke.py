#!/usr/bin/env python3
"""Exercise Tandem-owned provider lifecycle without manually starting Compose or a sidecar."""

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from providers import PROVIDERS, create_providers


def verify(binary):
    with tempfile.TemporaryDirectory(prefix="tandem-providers-smoke-") as directory:
        root = Path(directory)
        create_providers(root)
        home = root / ".tandem"
        namespace = "provider-smoke-" + root.name.removeprefix("tandem-providers-smoke-").replace("_", "-")
        environment = {**os.environ, "TANDEM_HOME": str(home), "TANDEM_NAMESPACE": namespace}
        stop_times = []

        def cli(*arguments):
            result = subprocess.run([str(binary), *arguments], env=environment,
                                    check=False, capture_output=True, text=True, timeout=210)
            if result.returncode:
                raise RuntimeError(f"Tandem {' '.join(arguments)} failed: {result.stderr}")
            return result.stdout

        def providers():
            return {provider["name"]: provider for provider in json.loads(cli("list-providers"))["providers"]}

        def action(kind, name):
            started = time.monotonic()
            result = cli("provider", kind, name)
            if kind == "stop":
                stop_times.append(time.monotonic() - started)
                container = providers()[name]
                assert container["status"] == "stopped"
                exit_code = subprocess.run(
                    ["docker", "inspect", "--format", "{{.State.ExitCode}}", container["container_id"]],
                    check=True, capture_output=True, text=True, timeout=10,
                ).stdout.strip()
                assert exit_code == "0", f"Provider {name} did not exit cleanly: {exit_code}"
            return result

        def count(name="slack"):
            with sqlite3.connect(home / "settings.sqlite3") as database:
                return database.execute("SELECT count(*) FROM events WHERE provider = ?", (name,)).fetchone()[0]

        def wait(check, message):
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if check():
                    return
                time.sleep(0.2)
            raise RuntimeError(message)

        receipt_path = home / "locks" / f"provider-sidecar-{namespace}.json"
        try:
            discovered = providers()
            assert len(discovered) == 4
            assert all(provider["status"] == "not_started" for provider in discovered.values())
            assert not (home / "provider-credentials").exists()
            for name in PROVIDERS.values():
                action("start", name)
            assert all(provider["status"] == "running" for provider in providers().values())
            wait(lambda: all(count(name) > 0 for name in PROVIDERS.values()), "providers did not send events")
            first_receipt = json.loads(receipt_path.read_text())
            assert "delivered" in action("logs", "slack")
            with sqlite3.connect(home / "settings.sqlite3") as database:
                event_id = database.execute("SELECT event_id FROM events WHERE provider = 'slack' ORDER BY sequence LIMIT 1").fetchone()[0]
            epoch = event_id.rsplit(":", 1)[0]
            action("stop", "slack")
            assert providers()["slack"]["status"] == "stopped"
            before = count()
            time.sleep(4)
            assert count() == before
            action("start", "slack")
            wait(lambda: count() > before, "restarted provider did not collect again")
            with sqlite3.connect(home / "settings.sqlite3") as database:
                latest = database.execute("SELECT event_id FROM events WHERE provider = 'slack' ORDER BY sequence DESC LIMIT 1").fetchone()[0]
                assert latest.rsplit(":", 1)[0] == epoch
                assert database.execute("SELECT count(*) FROM provider_notifications WHERE acknowledged = 1").fetchone()[0] > 0
            os.kill(first_receipt["pid"], signal.SIGTERM)
            time.sleep(0.5)
            action("start", "slack")
            next_receipt = json.loads(receipt_path.read_text())
            assert next_receipt["identity"] != first_receipt["identity"]
            assert next_receipt["origin"] == first_receipt["origin"]
            before = count()
            wait(lambda: count() > before, "provider did not recover its sidecar connection")
            shutil.rmtree(home / "templates/providers/slack")
            assert not providers()["slack"]["available"]
            action("stop", "slack")
            assert providers()["slack"]["status"] == "stopped"
            assert count() > 0
            print("PASS: discovery, automatic sidecar/credentials, four Docker providers, logs, stop/start checkpoints, sidecar recovery, template-independent stop")
            print(f"Provider Stop completed in {min(stop_times):.3f}–{max(stop_times):.3f}s; all collectors exited cleanly")
        finally:
            try:
                for name in PROVIDERS.values():
                    runtime = home / "runtime" / namespace / "providers" / name / "compose.json"
                    if runtime.exists():
                        subprocess.run(["docker", "compose", "-p", f"{namespace}-provider-{name}", "-f", str(runtime),
                                        "down", "--volumes", "--remove-orphans"], env=environment,
                                       check=True, capture_output=True, text=True, timeout=60)
            finally:
                if receipt_path.exists():
                    receipt = json.loads(receipt_path.read_text())
                    try:
                        os.kill(receipt["pid"], signal.SIGTERM)
                    except ProcessLookupError:
                        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[2] / "target/debug/tandem")
    arguments = parser.parse_args()
    verify(arguments.binary.resolve())


if __name__ == "__main__":
    main()
