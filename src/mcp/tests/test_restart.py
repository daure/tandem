"""Exercise container lifecycle scope, approval, locking and failures through stdio."""

import fcntl
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest

from stdio_smoke import Client

BINARY = Path(__file__).resolve().parents[3] / "target/debug/tandem"

DOCKER = '''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

home = Path(os.environ["TANDEM_HOME"])
containers = json.loads((home / "containers.json").read_text())
args = sys.argv[1:]
if args[0] == "ps":
    filters = [arg.removeprefix("label=").split("=", 1) for arg in args if arg.startswith("label=")]
    print("\\n".join(container["Id"] for container in containers
                    if all(container["Config"]["Labels"].get(key) == value for key, value in filters)))
elif args[0] == "inspect":
    for container in containers:
        if (home / (container["Id"] + ".boot")).exists():
            container["State"]["Health"] = {"Status": "starting"}
    print(json.dumps([container for container in containers if container["Id"] in args[1:]]))
elif args[0] in {"restart", "start", "stop"}:
    with (home / "mutations").open("a") as log:
        log.write(json.dumps(args) + "\\n")
    if (home / ("fail-" + args[0])).exists():
        sys.exit(args[0] + " denied by Docker")
    for container in containers:
        if container["Id"] in args[1:]:
            container["State"]["Status"] = "exited" if args[0] == "stop" else "running"
            container["State"]["StartedAt"] = "2026-09-17T12:00:00Z"
            if (home / ("crash-" + args[0])).exists():
                container["State"]["Status"] = "exited"
                container["State"]["ExitCode"] = 1
    (home / "containers.json").write_text(json.dumps(containers))
else:
    sys.exit("unexpected Docker command: " + repr(args))
'''


@unittest.skipUnless(os.name == "posix" and BINARY.is_file(), "Build Tandem before runtime tests")
class RestartTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="tandem-restart-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        docker = self.root / "docker"
        docker.write_text(DOCKER)
        docker.chmod(0o755)
        self.home = self.root / "home"
        self.home.mkdir()
        self.containers = []
        for name, service, role, state in [
            ("review", "web", "service", "running"),
            ("review", "db", "service", "exited"),
            ("review", "setup", "oneshot", "exited"),
            ("other", "web", "service", "running"),
            ("gateway", "gateway", "service", "running"),
        ]:
            self.containers.append({
                "Id": f"{name}-{service}",
                "Config": {"Labels": {
                    "com.docker.compose.project": f"restart-test-{name}",
                    "com.docker.compose.service": service,
                    "io.tandem.namespace": "restart-test",
                    "io.tandem.kind": "gateway" if name == "gateway" else "instance",
                    "io.tandem.instance": name, "io.tandem.template": "website",
                    "io.tandem.template-directory": str(self.home / "templates/website"),
                    "io.tandem.workspace": str(self.home / "workspaces" / name),
                    "io.tandem.role": role,
                }},
                "State": {"Status": state, "ExitCode": 0, "StartedAt": "2026-09-16T12:00:00Z"},
            })
        self.save()
        workspace = self.home / "workspaces/review"
        workspace.mkdir(parents=True)
        self.data = workspace / "keep"
        self.data.write_text("user data")
        environment = {
            **os.environ, "PATH": f"{self.root}{os.pathsep}{os.environ['PATH']}",
            "TANDEM_HOME": str(self.home), "TANDEM_NAMESPACE": "restart-test",
            "XDG_STATE_HOME": str(self.root / "state"),
        }
        for key in list(environment):
            if key.startswith("TANDEM_KEY_") or key == "TANDEM_INSTRUCTIONS_FILE":
                environment.pop(key)
        self.client = Client(BINARY, environment)
        self.addCleanup(self.client.close)

    def save(self):
        (self.home / "containers.json").write_text(json.dumps(self.containers))

    def mutations(self):
        path = self.home / "mutations"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def restart(self, service=None):
        arguments = {"name": "review", "confirmed": True}
        tool = "restart_instance"
        if service is not None:
            tool = "restart_service"
            arguments["service"] = service
        operation = self.client.tool(tool, arguments)
        self.assertEqual(operation["action"], tool)
        self.assertEqual(operation["name"], "review")
        self.assertEqual(operation["service"], service)
        return self.wait_operation(operation)

    def wait_operation(self, operation):
        deadline = time.monotonic() + 5
        while operation["state"] == "running" and time.monotonic() < deadline:
            time.sleep(0.02)
            operation = self.client.tool("get_operation", {"id": operation["id"]})
        self.assertNotEqual(operation["state"], "running")
        return operation

    def wait_progress(self, operation, text):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            current = self.client.tool("get_operation", {"id": operation["id"]})
            self.assertEqual(current["state"], "running", current)
            if any(text in line for line in current["progress"]):
                return current
            time.sleep(0.02)
        self.fail(f"Restart did not reach {text}: {current}")

    def test_service_completion_waits_for_health_and_gateway_content(self):
        ready = threading.Event()
        paths = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                paths.append(self.path)
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"application ready" if ready.is_set() else b"warming up")

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(server.server_close)
        self.addCleanup(worker.join)
        self.addCleanup(server.shutdown)
        template = self.client.tool("create_template", {"name": "website"})
        Path(template["manifest_file"]).write_text(json.dumps({"routes": {
            "web": {"port": 80, "readiness_path": "health", "readiness_contains": "application ready"},
            "db": {"port": 80, "readiness_path": "unrelated", "readiness_contains": "unused"},
        }}))
        self.containers[0]["Config"]["Labels"]["io.tandem.url"] = f"http://127.0.0.1:{server.server_port}/review/web/"
        self.containers[0]["State"]["Health"] = {"Status": "healthy"}
        self.save()
        health = self.home / "review-web.boot"
        for action in ["restart", "start"]:
            with self.subTest(action=action):
                ready.clear()
                paths.clear()
                health.touch()
                operation = self.client.tool(action + "_service", {"name": "review", "service": "web", "confirmed": True})
                self.wait_progress(operation, "Waiting for web: boot")
                self.assertEqual(paths, [], "Route probes wait for container health")
                health.unlink()
                self.wait_progress(operation, "Waiting for content assertion")
                self.assertEqual(set(paths), {"/review/web/health"})
                ready.set()
                self.assertEqual(self.wait_operation(operation)["state"], "succeeded")
        self.assertEqual(self.mutations(), [["restart", "review-web"], ["start", "review-web"]])

    def service_state(self, action, service="web", name="review"):
        operation = self.client.tool(action + "_service", {
            "name": name, "service": service, "confirmed": True,
        })
        self.assertEqual(operation["action"], action + "_service")
        self.assertEqual(operation["service"], service)
        return self.wait_operation(operation)

    def test_service_stop_start_preserves_scope_replicas_and_data(self):
        replica = json.loads(json.dumps(self.containers[0]))
        replica["Id"] = "review-web-replica"
        self.containers.append(replica)
        self.save()
        for action, status in [("stop", "exited"), ("start", "running")]:
            with self.subTest(action=action):
                self.assertEqual(self.service_state(action)["state"], "succeeded")
                self.assertEqual(set(self.mutations()[-1]), {action, "review-web", "review-web-replica"})
                after = json.loads((self.home / "containers.json").read_text())
                for before, current in zip(self.containers, after):
                    if current["Id"] in {"review-web", "review-web-replica"}:
                        self.assertEqual(current["State"]["Status"], status)
                        self.assertEqual(current["Config"], before["Config"])
                    else:
                        self.assertEqual(current, before)
                self.assertEqual(self.data.read_text(), "user data")

    def test_service_state_requires_approval_ownership_and_instance_lock(self):
        for action in ["start", "stop"]:
            with self.subTest(action=action):
                for arguments in [
                    {"name": "review", "service": "web"},
                    {"name": "gateway", "service": "gateway", "confirmed": True},
                    {"name": "review", "service": "", "confirmed": True},
                ]:
                    rejected = self.client.request("tools/call", {"name": action + "_service", "arguments": arguments})
                    self.assertTrue(rejected.get("isError"))
                for service in ["missing", "setup", "--all"]:
                    self.assertIn("not found", self.service_state(action, service)["error"])
                with (self.home / "locks/instance-review").open("w") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    self.assertIn("busy", self.service_state(action)["error"])
                unmanaged = json.loads(json.dumps(self.containers[0]))
                unmanaged["Id"] = "unmanaged"
                del unmanaged["Config"]["Labels"]["io.tandem.kind"]
                self.containers.append(unmanaged)
                self.save()
                self.assertIn("unmanaged", self.service_state(action)["error"])
                self.containers.pop()
                self.save()
        self.assertEqual(self.mutations(), [])

    def test_service_state_reports_docker_errors_and_start_crashes(self):
        for action in ["start", "stop"]:
            failure = self.home / ("fail-" + action)
            failure.touch()
            operation = self.service_state(action)
            self.assertEqual(operation["state"], "failed")
            self.assertIn(action + " denied by Docker", operation["error"])
            failure.unlink()
        (self.home / "crash-start").touch()
        self.assertIn("web: down (exit 1)", self.service_state("start")["error"])

    def test_routed_service_can_stop_without_template_configuration(self):
        self.containers[0]["Config"]["Labels"]["io.tandem.url"] = "http://127.0.0.1:9876/review/web/"
        self.containers[0]["State"]["Status"] = "paused"
        self.save()
        self.assertEqual(self.service_state("stop")["state"], "succeeded")
        self.assertEqual(self.mutations(), [["stop", "review-web"]])

    def test_instance_completion_waits_for_every_restarted_service(self):
        self.containers[1]["State"]["Health"] = {"Status": "healthy"}
        self.save()
        health = self.home / "review-db.boot"
        health.touch()
        operation = self.client.tool("restart_instance", {"name": "review", "confirmed": True})
        self.wait_progress(operation, "Waiting for db: boot")
        health.unlink()
        self.assertEqual(self.wait_operation(operation)["state"], "succeeded")

    def test_crashed_service_fails_instead_of_reporting_completion(self):
        (self.home / "crash-restart").touch()
        operation = self.restart("web")
        self.assertEqual(operation["state"], "failed")
        self.assertIn("web: down (exit 1)", operation["error"])

    def test_instance_and_service_restart_only_their_existing_containers(self):
        for arguments, tool in [
            ({"name": "review"}, "restart_instance"),
            ({"name": "review", "service": "web"}, "restart_service"),
            ({"name": "gateway", "confirmed": True}, "restart_instance"),
        ]:
            rejected = self.client.request("tools/call", {"name": tool, "arguments": arguments})
            self.assertTrue(rejected.get("isError"))
        self.assertEqual(self.mutations(), [])
        self.assertEqual(self.restart("web")["state"], "succeeded")
        self.assertEqual(self.mutations(), [["restart", "review-web"]])
        self.assertEqual(self.restart()["state"], "succeeded")
        self.assertEqual(self.mutations()[1], ["restart", "review-db", "review-web"])
        after = json.loads((self.home / "containers.json").read_text())
        for before, current in zip(self.containers, after):
            self.assertEqual(current["Id"], before["Id"])
            self.assertEqual(current["Config"], before["Config"])
            if current["Id"] in {"review-web", "review-db"}:
                self.assertEqual(current["State"]["Status"], "running")
            else:
                self.assertEqual(current, before)
        self.assertEqual(self.data.read_text(), "user data")

    def test_invalid_targets_locks_and_docker_failures_are_reported(self):
        for service in ["missing", "setup", "--all"]:
            operation = self.restart(service)
            self.assertEqual(operation["state"], "failed")
            self.assertIn("not found", operation["error"])
        with (self.home / "locks/instance-review").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            operation = self.restart("web")
            self.assertEqual(operation["state"], "failed")
            self.assertIn("busy", operation["error"])
        self.containers[0]["State"]["Status"] = "paused"
        self.save()
        self.assertIn("unpause", self.restart()["error"])
        self.containers[0]["State"]["Status"] = "running"
        unmanaged = json.loads(json.dumps(self.containers[0]))
        unmanaged["Id"] = "unmanaged"
        del unmanaged["Config"]["Labels"]["io.tandem.kind"]
        self.containers.append(unmanaged)
        self.save()
        self.assertIn("unmanaged", self.restart("web")["error"])
        self.assertEqual(self.mutations(), [])
        self.containers.pop()
        self.save()
        (self.home / "fail-restart").touch()
        operation = self.restart("web")
        self.assertEqual(operation["state"], "failed")
        self.assertIn("restart denied by Docker", operation["error"])
        self.assertEqual(self.data.read_text(), "user data")


if __name__ == "__main__":
    unittest.main()
