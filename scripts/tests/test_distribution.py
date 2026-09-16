import json
from pathlib import Path
import shutil
import subprocess
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DistributionTests(unittest.TestCase):
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
