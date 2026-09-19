import os
from pathlib import Path
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT / "target/debug/tandem"
from stdio_smoke import Client


@unittest.skipUnless(os.name == "posix" and BINARY.is_file(), "Build Tandem before runtime integration tests")
class RuntimeTests(unittest.TestCase):
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

    def test_open_command_is_persisted_and_runs_only_after_approval(self):
        with tempfile.TemporaryDirectory(prefix="tandem-open-command-") as directory:
            root = Path(directory)
            environment = {**os.environ, "TANDEM_HOME": directory,
                           "XDG_STATE_HOME": str(root / "state")}
            environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
            writer = Client(BINARY, environment)
            try:
                reader = Client(BINARY, environment)
                try:
                    writer.tool("get_instructions")
                    reader.tool("get_instructions")
                    tools = writer.request("tools/list", {})["tools"]
                    self.assertTrue({"get_open_command", "set_open_command", "run_open_command"} <= {tool["name"] for tool in tools})
                    self.assertEqual(reader.tool("get_open_command"), {"command": ""})
                    command = 'printf \'%s\' "$TANDEM_WORKSPACE" > "$TANDEM_HOME/unexpected"'
                    workspace = root / "workspaces" / "review"
                    workspace.mkdir()
                    (workspace / "AGENTS.md").write_text("Workspace guidance\n")
                    rejected = writer.request("tools/call", {
                        "name": "set_open_command", "arguments": {"command": command},
                    })
                    self.assertTrue(rejected.get("isError"))
                    saved = writer.tool("set_open_command", {"command": command, "confirmed": True})
                    self.assertEqual(saved, {"command": command})
                    self.assertEqual(reader.tool("get_open_command"), saved)
                    self.assertFalse((root / "unexpected").exists())
                    rejected = writer.request("tools/call", {
                        "name": "run_open_command", "arguments": {"name": "review"},
                    })
                    self.assertTrue(rejected.get("isError"))
                    self.assertFalse((root / "unexpected").exists())
                    self.assertEqual(
                        writer.tool("run_open_command", {"name": "review", "confirmed": True}),
                        {"workspace": str(workspace)},
                    )
                    self.assertEqual((root / "unexpected").read_text(), str(workspace))
                finally:
                    reader.close()
            finally:
                writer.close()
            restarted = Client(BINARY, environment)
            try:
                restarted.tool("get_instructions")
                self.assertEqual(restarted.tool("get_open_command"), {"command": command})
                self.assertEqual(restarted.tool("set_open_command", {"command": "", "confirmed": True}), {"command": ""})
                self.assertEqual(restarted.tool("get_open_command"), {"command": ""})
            finally:
                restarted.close()

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
            environment = {**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                           "TANDEM_HOME": str(root / "home"), "TANDEM_NAMESPACE": "deadline-test",
                           "TANDEM_GATEWAY_PORT": "9886", "XDG_STATE_HOME": str(root / "state")}
            environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
            client = Client(BINARY, environment)
            try:
                client.tool("create_template", {"name": "website"})
                start = time.monotonic()
                operation = client.tool("create_instance", {"template": "website", "name": "review",
                                        "confirmed": True, "timeout_seconds": 5, "wait": True}, timeout=20)
                elapsed = time.monotonic() - start
                self.assertEqual(operation["state"], "failed")
                self.assertIn("timed out", operation["error"])
                self.assertLess(elapsed, 7, "Each Docker probe must consume the same startup budget")
                self.assertTrue((root / "home/workspaces/review").is_dir())
            finally:
                client.close()
            self.assertTrue(client.process.stdin.closed)
            self.assertTrue(client.process.stdout.closed)


if __name__ == "__main__":
    unittest.main()
