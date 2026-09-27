import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DistributionTests(unittest.TestCase):
    def test_release_workflow_prepares_the_shell_installer(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn(
            "python3 scripts/prepare_installer.py target/distrib/tandem-installer.sh",
            workflow,
        )

    def test_release_installer_runs_opencode_companion_setup(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory) / "tandem-installer.sh"
            installer.write_text('''install() {
    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "everything's installed!"
}
''')
            command = ["python3", str(ROOT / "scripts/prepare_installer.py"), str(installer)]
            first = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=10)
            self.assertEqual(first.returncode, 0, first.stderr)
            prepared = installer.read_text()
            self.assertIn('ensure "$_install_dir/tandem" opencode-setup', prepared)
            self.assertLess(prepared.index("opencode-setup"), prepared.index("everything's installed!"))

            second = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=10)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(installer.read_text(), prepared)

    @unittest.skipUnless(shutil.which("dist"), "cargo-dist is required for distribution planning")
    def test_distribution_plan_contains_the_archive_checksum_and_installer(self):
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
        result = subprocess.run(["dist", "plan", "--output-format=json", "--target=x86_64-unknown-linux-gnu",
                                 f"--tag=v{version}"], cwd=ROOT, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        artifacts = json.loads(result.stdout)["artifacts"]
        self.assertLessEqual({"tandem-installer.sh", "tandem-x86_64-unknown-linux-gnu.tar.xz",
                              "tandem-x86_64-unknown-linux-gnu.tar.xz.sha256"}, set(artifacts))


if __name__ == "__main__":
    unittest.main()
