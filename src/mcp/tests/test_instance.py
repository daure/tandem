"""Exercise scoped identity, lifecycle locking and protocol rejection through real stdio."""

import fcntl
from contextlib import closing
import json
import os
import sqlite3
import subprocess
import time
import unittest

import test_restart
from instance_smoke import run_cli, smoke, workspace_fixture
from stdio_smoke import BINARY, Client


@unittest.skipUnless(os.name == "posix" and BINARY.is_file(), "Build Tandem before runtime tests")
class InstanceTests(unittest.TestCase):
    def test_start_stop_keep_deliverables_and_expose_only_self_controls(self):
        smoke(BINARY)

    def test_controls_reject_targets_and_conflicting_lifecycle_work(self):
        with workspace_fixture(BINARY) as (home, _, client):
            result = client.request("tools/call", {
                "name": "stop_self", "arguments": {"name": "other"},
            }, expect_error=True)
            self.assertEqual(result["code"], -32602)
            self.assertIn("unknown field `name`", result["message"])
            with (home / "locks/instance-review").open("a") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                for name in ("start_self", "stop_self", "ping"):
                    result = client.request("tools/call", {"name": name, "arguments": {}})
                    self.assertTrue(result.get("isError"), result)
                    self.assertIn("busy", str(result))
            self.assertEqual(client.tool("stop_self")["state"], "succeeded")

    def test_ping_is_scoped_and_muted_by_default(self):
        with workspace_fixture(BINARY) as (home, _, client):
            before = (home / "workspaces/review/AGENTS.md").read_text()
            self.assertEqual(client.tool("ping"), {
                "instance": "review", "sound_scheduled": False,
            })
            result = client.request("tools/call", {
                "name": "ping", "arguments": {"name": "other"},
            }, expect_error=True)
            self.assertEqual(result["code"], -32602)
            self.assertIn("unknown field `name`", result["message"])
            self.assertEqual((home / "workspaces/review/AGENTS.md").read_text(), before)
            self.assertTrue((home / "workspaces/other").is_dir())

    def test_old_connection_cannot_control_a_recreated_workspace(self):
        with workspace_fixture(BINARY) as (home, environment, client):
            run_cli(BINARY, environment, "delete-instance", "review")
            run_cli(BINARY, environment, "new-instance", "review", "-t", "blank")
            for name in ("start_self", "stop_self", "ping"):
                result = client.request("tools/call", {"name": name, "arguments": {}})
                self.assertTrue(result.get("isError"), result)
                self.assertIn("replaced", str(result))
            self.assertTrue((home / "workspaces/review").is_dir())

    def test_enabled_pings_publish_the_calling_session_metadata_without_replaying_muted_calls(self):
        with workspace_fixture(BINARY) as (home, environment, _):
            bin_directory = home / "bin"
            bin_directory.mkdir()
            player = bin_directory / "canberra-gtk-play"
            player.write_text("#!/bin/sh\nexit 0\n")
            player.chmod(0o755)
            client = Client(BINARY, {**environment, "PATH": f"{bin_directory}:{environment['PATH']}"},
                            command="mcp-instance", cwd=home / "workspaces/review")
            self.addCleanup(client.close)
            client.tool("get_instructions")
            def ping():
                return client.request("tools/call", {
                    "name": "ping", "arguments": {}, "_meta": {"ai.opencode/sessionID": "ses_review"},
                })
            def read_pings():
                with closing(sqlite3.connect(home / "settings.sqlite3")) as connection:
                    payload = connection.execute(
                        "SELECT payload FROM runtime_records WHERE namespace=? AND name=? AND kind=?",
                        ("instance-mcp-test", "review", "journal"),
                    ).fetchone()[0]
                return json.loads(payload).get("pings", {})
            self.assertFalse(ping().get("isError"))
            self.assertEqual(read_pings(), {})
            with closing(sqlite3.connect(home / "settings.sqlite3")) as connection, connection:
                connection.execute("INSERT OR REPLACE INTO app_settings(key,value) VALUES (?,?)",
                                   ("instances.ping_enabled", "true"))
            previous = 0
            for _ in range(2):
                self.assertFalse(ping().get("isError"))
                pings = read_pings()
                self.assertEqual(set(pings), {"ses_review"})
                self.assertGreater(pings["ses_review"], previous)
                previous = pings["ses_review"]

    def test_unowned_workspaces_and_wrong_namespaces_exit_without_protocol_output(self):
        with workspace_fixture(BINARY) as (home, environment, _):
            for directory, namespace in [(home, "instance-mcp-test"),
                                         (home / "workspaces/review", "another")]:
                result = subprocess.run(
                    [str(BINARY), "mcp-instance"], cwd=directory,
                    env={**environment, "TANDEM_NAMESPACE": namespace},
                    capture_output=True, timeout=10,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")


@unittest.skipUnless(os.name == "posix" and BINARY.is_file(), "Build Tandem before runtime tests")
class InstanceContainerTests(unittest.TestCase):
    def setUp(self):
        self.fixture = test_restart.RestartTests(methodName="runTest")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        fixture = self.fixture
        for container in fixture.containers:
            container["State"]["Running"] = container["State"]["Status"] == "running"
        fixture.containers[-1]["Config"]["Labels"]["io.tandem.gateway-port"] = "9876"
        fixture.save()
        template = fixture.client.tool("create_template", {"name": "website"})
        (fixture.home / "templates/website/compose.yaml").write_text(
            "services:\n  web:\n    image: fixture\n  db:\n    image: fixture\n"
        )
        expected = next(instance for instance in fixture.client.tool("list_instances")["instances"]
                        if instance["name"] == "review")
        owner = {"namespace": "restart-test", "template": "website", "instance": "review",
                 "directory": template["directory"], "workspace": expected["workspace"]}
        with closing(sqlite3.connect(fixture.home / "settings.sqlite3")) as connection, connection:
            for kind, payload in [("journal", {"expected": expected}), ("ownership", owner)]:
                connection.execute(
                    "INSERT OR REPLACE INTO runtime_records(namespace,name,kind,payload) VALUES (?,?,?,?)",
                    ("restart-test", "review", kind, json.dumps(payload)),
                )
        docker = test_restart.DOCKER.replace(
            'else:\n    sys.exit("unexpected Docker command: " + repr(args))',
            SELF_COMPOSE,
        ).replace(
            'args = sys.argv[1:]',
            '''args = sys.argv[1:]
if args[0] in {"ps", "inspect"}:
    if (home / "slow-observation").exists():
        import time
        time.sleep(10)
    if (home / "fail-observation").exists():
        sys.exit("observation unavailable")''',
        )
        (fixture.root / "docker").write_text(docker)
        self.client = Client(BINARY, fixture.environment, command="mcp-instance",
                             cwd=fixture.home / "workspaces/review")
        self.addCleanup(self.client.close)

    def test_guidance_observes_service_health_and_stopped_state_under_the_lifecycle_lock(self):
        fixture = self.fixture
        fixture.containers[0]["State"]["Health"] = {"Status": "starting"}
        fixture.save()
        with (fixture.home / "locks/instance-review").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            status = self.client.tool("get_instructions")["status"]
            self.assertEqual(status["instance"], "degraded")
            self.assertEqual(status["services"], [
                {"name": "db", "replica": 1, "status": "stopped"},
                {"name": "setup", "replica": 1, "status": "completed"},
                {"name": "web", "replica": 1, "status": "checking_health"},
            ])
            self.assertTrue(status["topology_known"])
            self.assertIsInstance(status["observed_at_unix_seconds"], int)
            self.assertFalse(status["stale"])
            self.assertIsNone(status["error"])
            fixture.containers[0]["State"]["Status"] = "exited"
            fixture.save()
            self.assertEqual(self.client.tool("get_instructions")["status"]["instance"], "stopped")
        self.assertEqual(fixture.mutations(), [])

    def test_startup_status_includes_waiting_services_and_uncreated_replicas(self):
        fixture = self.fixture
        with closing(sqlite3.connect(fixture.home / "settings.sqlite3")) as connection, connection:
            journal = json.loads(connection.execute(
                "SELECT payload FROM runtime_records WHERE namespace=? AND name=? AND kind=?",
                ("restart-test", "review", "journal"),
            ).fetchone()[0])
            replica = json.loads(json.dumps(journal["expected"]["services"][-1]))
            self.assertEqual(replica["name"], "web")
            replica["container_id"] = ""
            replica["runtime"]["replica"] = 2
            journal["expected"]["services"].append(replica)
            connection.execute(
                "UPDATE runtime_records SET payload=? WHERE namespace=? AND name=? AND kind=?",
                (json.dumps(journal), "restart-test", "review", "journal"),
            )
            operation = {
                "id": "123-456", "action": "create_instance", "name": "review", "template": "website",
                "service": None, "state": "running", "progress": [], "warnings": [],
                "elapsed_seconds": 0, "elapsed_milliseconds": 0, "error": None, "instance": None,
            }
            startup = {
                "operation": operation, "description": None, "branch_instances": False,
                "start_instance": True, "kind": "hot", "started_at": int(time.time()),
                "timeout": 60, "owner_pid": os.getpid(), "workspace_ready": True, "services": [],
            }
            connection.execute(
                "INSERT INTO runtime_records(namespace,name,kind,payload) VALUES (?,?,?,?)",
                ("restart-test", "review", "startup", json.dumps(startup)),
            )
        fixture.containers = [item for item in fixture.containers if item["Id"] != "review-db"]
        fixture.save()
        with (fixture.home / "locks/instance-review").open("a") as lock, \
                (fixture.home / "locks/startup-123-456").open("a") as lease:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            status = self.client.tool("get_instructions")["status"]
            startup["services"] = journal["expected"]["services"]
            for service in startup["services"]:
                service["container_id"] = ""
                service["runtime"]["state"] = "missing"
            journal["expected"]["services"] = []
            with closing(sqlite3.connect(fixture.home / "settings.sqlite3")) as connection, connection:
                for kind, payload in [("journal", journal), ("startup", startup)]:
                    connection.execute(
                        "UPDATE runtime_records SET payload=? WHERE namespace=? AND name=? AND kind=?",
                        (json.dumps(payload), "restart-test", "review", kind),
                    )
            fixture.containers = [item for item in fixture.containers
                                  if not item["Id"].startswith("review-")]
            fixture.save()
            preview = self.client.tool("get_instructions")["status"]
        self.assertEqual(status["instance"], "starting")
        self.assertEqual(status["services"], [
            {"name": "db", "replica": 1, "status": "waiting"},
            {"name": "setup", "replica": 1, "status": "completed"},
            {"name": "web", "replica": 1, "status": "running"},
            {"name": "web", "replica": 2, "status": "waiting"},
        ])
        self.assertFalse(status["stale"])
        self.assertEqual(preview["instance"], "starting")
        self.assertFalse(preview["topology_known"])
        self.assertEqual(preview["services"], [
            {"name": name, "replica": replica, "status": "waiting"}
            for name, replica in [("db", 1), ("setup", 1), ("web", 1), ("web", 2)]
        ])
        self.assertEqual(fixture.mutations(), [])

    def test_observation_failure_and_timeout_preserve_guidance_without_claiming_services_stopped(self):
        fixture = self.fixture
        for marker, error in [("fail-observation", "observation unavailable"),
                              ("slow-observation", "timed out")]:
            with self.subTest(marker=marker):
                flag = fixture.home / marker
                flag.touch()
                started = time.monotonic()
                instructions = self.client.tool("get_instructions", timeout=5)
                self.assertLess(time.monotonic() - started, 4)
                self.assertIn("core_guidance", instructions)
                self.assertTrue(instructions["markdown"])
                status = instructions["status"]
                self.assertEqual(status["instance"], "stale")
                self.assertTrue(status["stale"])
                self.assertIsNone(status["observed_at_unix_seconds"])
                self.assertIn(error, status["error"])
                self.assertEqual({item["status"] for item in status["services"]}, {"stale"})
                flag.unlink()
        self.assertFalse(self.client.tool("get_instructions")["status"]["stale"])
        self.assertEqual(fixture.mutations(), [])

    def test_prepare_only_status_lists_configured_services_without_containers(self):
        fixture = self.fixture
        with closing(sqlite3.connect(fixture.home / "settings.sqlite3")) as connection, connection:
            journal = json.loads(connection.execute(
                "SELECT payload FROM runtime_records WHERE namespace=? AND name=? AND kind=?",
                ("restart-test", "review", "journal"),
            ).fetchone()[0])
            journal["expected"]["runtime"]["prepared_only"] = True
            for service in journal["expected"]["services"]:
                service["container_id"] = ""
            connection.execute(
                "UPDATE runtime_records SET payload=? WHERE namespace=? AND name=? AND kind=?",
                (json.dumps(journal), "restart-test", "review", "journal"),
            )
        fixture.containers = [item for item in fixture.containers
                              if not item["Id"].startswith("review-")]
        fixture.save()
        status = self.client.tool("get_instructions")["status"]
        self.assertEqual(status["instance"], "not_started")
        self.assertEqual(status["services"], [
            {"name": name, "replica": 1, "status": "not_started"} for name in ["db", "setup", "web"]
        ])
        self.assertTrue(status["topology_known"])
        self.assertFalse(status["stale"])
        self.assertEqual(fixture.mutations(), [])

    def test_container_start_stop_preserves_siblings_gateway_and_deliverables(self):
        fixture = self.fixture
        before = json.loads((fixture.home / "containers.json").read_text())
        for action, state in [("stop_self", "exited"), ("start_self", "running")]:
            self.assertEqual(self.client.tool(action, timeout=30)["state"], "succeeded")
            current = json.loads((fixture.home / "containers.json").read_text())
            for original, container in zip(before, current):
                if container["Id"].startswith("review-"):
                    expected = "exited" if container["Id"] == "review-setup" else state
                    self.assertEqual(container["State"]["Status"], expected)
                    self.assertEqual(container["Config"], original["Config"])
                else:
                    self.assertEqual(container, original)
            self.assertEqual(fixture.data.read_text(), "user data")
        mutations = fixture.mutations()
        self.assertEqual(mutations[0][0], "stop")
        self.assertEqual(set(mutations[0][1:]), {"review-web", "review-db", "review-setup"})
        self.assertEqual(mutations[1:], [["compose-up", "review"]])
        self.assertIsNone(self.client.process.poll())

    def test_container_errors_are_reported_without_deleting_data(self):
        fixture = self.fixture
        for tool, marker, text in [("stop_self", "fail-stop", "stop denied by Docker"),
                                   ("start_self", "fail-compose", "compose denied by Docker")]:
            flag = fixture.home / marker
            flag.touch()
            result = self.client.request("tools/call", {"name": tool, "arguments": {}}, timeout=30)
            self.assertTrue(result.get("isError"), result)
            self.assertIn(text, str(result))
            self.assertEqual(fixture.data.read_text(), "user data")
            flag.unlink()


SELF_COMPOSE = '''elif args[0] == "network":
    pass
elif args[0] == "compose":
    if args[-3:] == ["config", "--format", "json"]:
        print(json.dumps({"services": {"web": {"image": "fixture"}, "db": {"image": "fixture"}}}))
    elif "up" in args:
        if (home / "fail-compose").exists():
            sys.exit("compose denied by Docker")
        instance = os.environ["TANDEM_INSTANCE"]
        with (home / "mutations").open("a") as log:
            log.write(json.dumps(["compose-up", instance]) + "\\n")
        for container in containers:
            labels = container["Config"]["Labels"]
            if labels["io.tandem.instance"] == instance and labels["io.tandem.role"] != "oneshot":
                container["State"]["Status"] = "running"
                container["State"]["Running"] = True
                container["State"]["StartedAt"] = datetime.now(timezone.utc).isoformat()
        (home / "containers.json").write_text(json.dumps(containers))
    else:
        sys.exit("unexpected Compose command: " + repr(args))
else:
    sys.exit("unexpected Docker command: " + repr(args))
'''


if __name__ == "__main__":
    unittest.main()
