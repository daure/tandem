import os
import json
import fcntl
from pathlib import Path
import tempfile
import time
import unittest

from stdio_smoke import BINARY, Client, disable_history_cleanup


@unittest.skipUnless(os.name == "posix" and BINARY.is_file(), "Build Tandem before runtime integration tests")
class RuntimeTests(unittest.TestCase):
    def test_creation_model_and_variant_reach_the_detached_client_with_optional_defaults(self):
        with tempfile.TemporaryDirectory(prefix="tandem-session-selection-") as directory:
            root = Path(directory)
            home = root / "home"
            disable_history_cleanup(home)
            zellij = root / "zellij"
            zellij.write_text("""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
args = sys.argv[1:]
if 'list-sessions' in args:
    print('main' if '--short' in args else 'main [Created 1s ago]')
elif 'list-panes' in args:
    print(json.dumps([{'id': 100, 'is_plugin': False, 'exited': False, 'tab_id': 9, 'tab_name': 'review'}]))
elif 'new-tab' in args or 'new-pane' in args:
    Path(os.environ['TANDEM_HOME'], 'client-command.json').write_text(json.dumps(args))
    print('9' if 'new-tab' in args else 'terminal_100')
""")
            zellij.chmod(0o755)
            docker = root / "docker"
            docker.write_text("#!/bin/sh\nexit 0\n")
            docker.chmod(0o755)
            environment = {**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                           "TANDEM_HOME": str(home), "TANDEM_NAMESPACE": "selection-test",
                           "ZELLIJ_SESSION_NAME": "main", "XDG_STATE_HOME": str(root / "state"),
                           "OC_DAEMON_STATE": str(root / "daemons")}
            environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
            client = Client(BINARY, environment)
            try:
                client.tool("create_template", {"name": "blank"})
                command = home / "client-command.json"
                operation = client.tool("create_instance", {
                    "template": "blank", "name": "no-client", "confirmed": True, "wait": True,
                })
                self.assertEqual(operation["state"], "succeeded")
                self.assertFalse(command.exists())
                guidance = "This is a workspace-only instance; no service containers are configured. Work locally and ask the user if additional services are needed."
                instructions = "Keep 'literal' instructions; $(touch injected)\n{{unchanged}} 日本語"
                for index, (selection, model, variant) in enumerate([
                    ({}, "", ""),
                    ({"model": "openai/test"}, "openai/test", ""),
                    ({"model": "openai/test", "variant": "high"}, "openai/test#high", "high"),
                    ({"variant": "high"}, "", "high"),
                    ({"agent_instructions": instructions}, "", ""),
                ]):
                    with self.subTest(selection=selection):
                        operation = client.tool("create_instance", {
                            "template": "blank", "name": f"review-{index}", "confirmed": True,
                            "wait": True, "opencode": True, "initial_prompt": "Inspect literal 'text'", **selection,
                        })
                        self.assertEqual(operation["state"], "succeeded")
                        args = json.loads(command.read_text())
                        self.assertIn(f"TANDEM_SESSION_MODEL={model}", args)
                        self.assertIn(f"TANDEM_SESSION_VARIANT={variant}", args)
                        self.assertIn("TANDEM_INITIAL_PROMPT=Inspect literal 'text'", args)
                        expected = guidance + "\n\n" + instructions if "agent_instructions" in selection else guidance
                        self.assertIn(f"TANDEM_SESSION_INSTRUCTIONS={expected}", args)
                        self.assertNotIn(instructions, (home / f"workspaces/blank/review-{index}/AGENTS.md").read_text())
                        command.unlink()
                for selection in [
                    {"model": "openai/test"},
                    {"agent_instructions": instructions},
                    {"opencode": True, "agent_instructions": ""},
                    {"opencode": True, "agent_instructions": "text\0text"},
                    {"opencode": True, "agent_instructions": "x" * 65537},
                    {"opencode": True, "model": "invalid"},
                    {"opencode": True, "variant": ""},
                    {"opencode": True, "model": "openai/test#fast", "variant": "high"},
                ]:
                    result = client.request("tools/call", {"name": "create_instance", "arguments": {
                        "template": "blank", "name": "invalid", "confirmed": True, **selection,
                    }})
                    self.assertTrue(result.get("isError"), result)
                    self.assertFalse((home / "workspaces/blank/invalid").exists())
                    self.assertFalse(command.exists())
            finally:
                client.close()

    def test_manifest_schema_and_validated_update_round_trip_through_stdio(self):
        with tempfile.TemporaryDirectory(prefix="tandem-manifest-") as directory:
            root = Path(directory)
            environment = {**os.environ, "TANDEM_HOME": directory,
                           "XDG_STATE_HOME": str(root / "state")}
            environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
            client = Client(BINARY, environment)
            try:
                instructions = client.tool("get_instructions")
                schema = instructions["manifest_schema"]
                self.assertEqual(schema["type"], "object")
                self.assertFalse(schema["additionalProperties"])
                self.assertIn("repositories", schema["properties"])
                created = client.tool("create_template", {"name": "website"})
                path = Path(created["manifest_file"])
                before = path.read_bytes()
                for arguments in [
                    {"name": "website", "manifest": {"description": "Unconfirmed"}},
                    {"name": "website", "confirmed": True,
                     "manifest": {"repositories": [{"source": "/source", "target": "../escape"}]}},
                ]:
                    result = client.request("tools/call", {
                        "name": "update_template_manifest", "arguments": arguments,
                    })
                    self.assertTrue(result.get("isError"))
                    self.assertEqual(path.read_bytes(), before)
                saved = client.tool("update_template_manifest", {
                    "name": "website", "confirmed": True,
                    "manifest": {"description": "Configured through MCP",
                                 "repositories": [{"source": "/source", "target": "app"}]},
                })
                self.assertEqual(saved["manifest"]["description"], "Configured through MCP")
                self.assertEqual(client.tool("get_template", {"name": "website"}), saved)
                self.assertEqual(list((root / "workspaces").iterdir()), [])
            finally:
                client.close()

    def test_startup_uses_one_budget_across_docker_and_gateway_probes(self):
        with tempfile.TemporaryDirectory(prefix="tandem-deadline-") as directory:
            root = Path(directory)
            executable = root / "docker"
            executable.write_text("""#!/usr/bin/env python3
import json
import sys
import time
if 'label=io.tandem.kind=instance' not in sys.argv:
    time.sleep(2)
if sys.argv[-3:] == ['config', '--format', 'json']:
    print(json.dumps({'services': {'web': {'image': 'nginx'}}}))
""")
            executable.chmod(0o755)
            disable_history_cleanup(root / "home")
            environment = {**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                           "TANDEM_HOME": str(root / "home"), "TANDEM_NAMESPACE": "deadline-test",
                           "TANDEM_GATEWAY_PORT": "9886", "XDG_STATE_HOME": str(root / "state")}
            environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
            client = Client(BINARY, environment)
            operation = None
            try:
                template = client.tool("create_template", {"name": "website"})
                (Path(template["directory"]) / "compose.yaml").write_text("services:\n  web:\n    image: nginx\n")
                start = time.monotonic()
                operation = client.tool("create_instance", {"template": "website", "name": "review",
                                        "confirmed": True, "timeout_seconds": 5, "wait": True}, timeout=20)
                elapsed = time.monotonic() - start
                self.assertEqual(operation["state"], "failed")
                self.assertIn("timed out", operation["error"])
                self.assertLess(elapsed, 7, "Each Docker probe must consume the same startup budget")
                self.assertTrue((root / "home/workspaces/website/review").is_dir())
            finally:
                client.close()
                if operation is not None:
                    lease = root / "home/locks" / f"startup-{operation['id']}"
                    with lease.open("rb") as lock:
                        deadline = time.monotonic() + 10
                        while True:
                            try:
                                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                                break
                            except BlockingIOError:
                                if time.monotonic() >= deadline:
                                    self.fail("Startup worker must exit before fixture cleanup")
                                time.sleep(0.05)
            self.assertTrue(client.process.stdin.closed)
            self.assertTrue(client.process.stdout.closed)


if __name__ == "__main__":
    unittest.main()
