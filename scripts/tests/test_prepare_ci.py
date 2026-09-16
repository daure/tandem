import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import prepare_ci

MANIFEST = '[package]\nname = "app"\nversion = "1.0.0"\n[dependencies]\ntuicore = { path = "../tuicore" }\n'
REGISTRY = 'source = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "abc"\n'


class PrepareCiTests(unittest.TestCase):
    def test_latest_stable_version_crosses_major_boundaries(self):
        versions = [
            {"num": version, "yanked": yanked} for version, yanked in [
                ("9.9.9", False), ("11.0.0", True), ("12.0.0-rc.1", False),
                ("10.2.0", False), ("10.10.0", False),
            ]
        ]
        with patch.object(prepare_ci.urllib.request, "urlopen", return_value=io.BytesIO(json.dumps({"versions": versions}).encode())):
            self.assertEqual(prepare_ci.latest_tuicore(), "10.10.0")

    def test_local_invocation_cannot_modify_the_manifest(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}), patch.object(prepare_ci, "latest_tuicore") as latest:
            with self.assertRaisesRegex(ValueError, "only allowed in GitHub Actions"):
                prepare_ci.prepare(Path("unused"))
            latest.assert_not_called()

    def test_ci_pins_latest_and_requires_its_registry_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(MANIFEST)

            def resolve(command, *, cwd, check):
                self.assertEqual(command, ["cargo", "update", "--workspace"])
                self.assertEqual(cwd, root)
                self.assertTrue(check)
                self.assertEqual(tomllib.loads((root / "Cargo.toml").read_text())["dependencies"]["tuicore"], "=10.10.0")
                (root / "Cargo.lock").write_text(f'version = 4\n[[package]]\nname = "tuicore"\nversion = "10.10.0"\n{REGISTRY}')

            with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}), patch.object(prepare_ci, "latest_tuicore", return_value="10.10.0"), patch.object(prepare_ci.subprocess, "run", side_effect=resolve):
                prepare_ci.prepare(root)

    def test_registry_or_resolution_failure_stops_preparation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(MANIFEST)
            (root / "Cargo.lock").write_text("version = 4\n")
            with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}), patch.object(prepare_ci, "latest_tuicore", return_value="10.10.0"), patch.object(prepare_ci.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "cargo")):
                with self.assertRaises(subprocess.CalledProcessError):
                    prepare_ci.prepare(root)


if __name__ == "__main__":
    unittest.main()
