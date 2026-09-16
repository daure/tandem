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
