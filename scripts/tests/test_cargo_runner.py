import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
KEYS = ("TANDEM_HOME", "TANDEM_NAMESPACE", "TANDEM_GATEWAY_PORT")


@unittest.skipUnless(os.name == "posix", "The Cargo runner uses a Unix shell")
class CargoRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="tandem runner ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        (self.root / "projects").mkdir()
        (self.root / "bin").mkdir()
        self.runner = self.root / "scripts/cargo-runner.sh"
        shutil.copy2(ROOT / "scripts/cargo-runner.sh", self.runner)
        self.binary = self.root / "bin/tandem"
        self.binary.write_text("""#!/usr/bin/env python3
import json
import os
import sys
print(json.dumps({"args": sys.argv[1:], "home": os.getenv("TANDEM_HOME"),
                  "namespace": os.getenv("TANDEM_NAMESPACE"), "port": os.getenv("TANDEM_GATEWAY_PORT")}))
sys.exit(int(os.getenv("TEST_EXIT", "0")))
""")
        self.binary.chmod(0o755)
        self.environment = {key: value for key, value in os.environ.items() if key not in KEYS}

    def write_environment(self):
        (self.root / "projects/env.sh").write_text("""export TANDEM_HOME='/fixture home'
export TANDEM_NAMESPACE=fixture-test
export TANDEM_GATEWAY_PORT=9886
""")

    def run_binary(self, *arguments, binary=None, environment=None):
        return subprocess.run([str(self.runner), str(binary or self.binary), *arguments],
                              cwd=self.root / "bin", env=environment or self.environment,
                              capture_output=True, text=True, timeout=10)

    def test_dev_sources_fixture_values_and_preserves_arguments(self):
        self.write_environment()
        result = self.run_binary("dev", "--bind", "127.0.0.1:7348", "argument with spaces",
                                 environment={**self.environment, "TANDEM_HOME": "/inherited"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertEqual(json.loads(result.stdout), {
            "args": ["dev", "--bind", "127.0.0.1:7348", "argument with spaces"],
            "home": "/fixture home", "namespace": "fixture-test", "port": "9886",
        })

    def test_missing_fixture_file_preserves_the_inherited_environment(self):
        result = self.run_binary("dev", environment={**self.environment, "TANDEM_HOME": "/custom"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["home"], "/custom")

    def test_other_commands_and_binaries_do_not_source_the_file(self):
        (self.root / "projects/env.sh").write_text("exit 91\n")
        for arguments in ((), ("mcp",), ("serve",), ("--help",)):
            with self.subTest(arguments=arguments):
                result = self.run_binary(*arguments)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIsNone(json.loads(result.stdout)["home"])
        helper = self.root / "bin/release-command"
        shutil.copy2(self.binary, helper)
        self.assertEqual(self.run_binary("dev", binary=helper).returncode, 0)

    def test_sourcing_failure_stops_launch_and_child_exit_codes_are_preserved(self):
        (self.root / "projects/env.sh").write_text("false\n")
        failed = self.run_binary("dev")
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(failed.stdout, "")
        result = self.run_binary("mcp", environment={**self.environment, "TEST_EXIT": "23"})
        self.assertEqual(result.returncode, 23)

    @unittest.skipUnless(shutil.which("cargo"), "Cargo is required for runner integration")
    def test_cargo_resolves_the_runner_from_a_checkout_subdirectory(self):
        self.write_environment()
        (self.root / ".cargo").mkdir()
        shutil.copy2(ROOT / ".cargo/config.toml", self.root / ".cargo/config.toml")
        (self.root / "src").mkdir()
        (self.root / "Cargo.toml").write_text('[package]\nname = "tandem"\nversion = "0.0.0"\nedition = "2024"\n')
        (self.root / "src/main.rs").write_text('fn main() { println!("{}", std::env::var("TANDEM_HOME").unwrap()); }\n')
        result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--", "dev"],
                                cwd=self.root / "src", env=self.environment,
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "/fixture home\n")
        self.assertEqual(result.stderr, "")


if __name__ == "__main__":
    unittest.main()
