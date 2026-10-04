"""Exercise scoped identity, lifecycle locking and protocol rejection through real stdio."""

import fcntl
from contextlib import closing
import json
import os
import sqlite3
import subprocess
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
                for name in ("start_self", "stop_self"):
                    result = client.request("tools/call", {"name": name, "arguments": {}})
                    self.assertTrue(result.get("isError"), result)
                    self.assertIn("busy", str(result))
            self.assertEqual(client.tool("stop_self")["state"], "succeeded")

    def test_old_connection_cannot_control_a_recreated_workspace(self):
        with workspace_fixture(BINARY) as (home, environment, client):
            run_cli(BINARY, environment, "delete-instance", "review")
            run_cli(BINARY, environment, "new-instance", "review", "-t", "blank")
            for name in ("start_self", "stop_self"):
                result = client.request("tools/call", {"name": name, "arguments": {}})
                self.assertTrue(result.get("isError"), result)
                self.assertIn("replaced", str(result))
            self.assertTrue((home / "workspaces/review").is_dir())

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
        )
        (fixture.root / "docker").write_text(docker)
        self.client = Client(BINARY, fixture.environment, command="mcp-instance",
                             cwd=fixture.home / "workspaces/review")
        self.addCleanup(self.client.close)

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
