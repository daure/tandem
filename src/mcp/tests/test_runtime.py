import os
from pathlib import Path
import tempfile
import time
import unittest

from stdio_smoke import BINARY, Client, disable_history_cleanup


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
                self.assertTrue((root / "home/workspaces/review").is_dir())
            finally:
                client.close()
            self.assertTrue(client.process.stdin.closed)
            self.assertTrue(client.process.stdout.closed)


if __name__ == "__main__":
    unittest.main()
