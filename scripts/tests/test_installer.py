import importlib.util
import os
from pathlib import Path
import pty
import select
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
APP = "tandem"
SPEC = importlib.util.spec_from_file_location("prepare_installer", ROOT / "scripts/prepare_installer.py")
PREPARER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARER)
SOURCE = '''#!/bin/sh
set -eu
ensure() { "$@" || exit 1; }
ignore() { "$@" || :; }
say() { printf '%s\n' "$*"; }
download_binary_and_run_installer() {
    [ "${1:-}" != --help ] || { printf 'installer help\n'; return; }
    [ "${FAIL_DOWNLOAD:-0}" != 1 ] || return 1
    _install_dir="$INSTALL_DIR"
    _install_temp="$STAGE"
    _lib_install_temp="$LIB_STAGE"
    _bins="$APP"
    for _bin_name in $_bins; do
        ensure mv "$_install_temp/$_bin_name" "$_install_dir"
    done
    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "everything's installed!"
}
download_binary_and_run_installer "$@" || exit 1
'''


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="tandem-upgrade-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.destination, self.stage = self.root / "install path", self.root / "stage"
        self.destination.mkdir()
        self.stage.mkdir()
        self.binary = self.destination / APP
        shutil.copyfile(shutil.which("sleep"), self.binary)
        self.binary.chmod(0o755)
        shutil.copyfile(shutil.which("true"), self.stage / APP)
        (self.stage / APP).chmod(0o755)
        self.process = subprocess.Popen([self.binary, "120"])
        self.addCleanup(self.stop_process)
        self.installer = self.root / "installer.sh"
        self.installer.write_text(SOURCE)
        PREPARER.prepare(self.installer)
        self.env = {**os.environ, "APP": APP, "INSTALL_DIR": str(self.destination),
                    "STAGE": str(self.stage), "LIB_STAGE": str(self.root / "lib-stage")}

    def stop_process(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=5)

    def test_noninteractive_upgrade_requires_approval_before_replacement(self):
        before = self.binary.read_bytes()
        result = subprocess.run(["sh", self.installer], env=self.env, capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Upgrade cancelled", result.stderr)
        self.assertEqual(self.binary.read_bytes(), before)
        self.assertIsNone(self.process.poll())

    def test_approved_upgrade_replaces_the_build_and_stops_the_old_process(self):
        master, slave = pty.openpty()
        child = subprocess.Popen(["sh", self.installer], env=self.env, stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        output, answered = b"", False
        try:
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.1)[0]:
                    try:
                        output += os.read(master, 65536)
                    except OSError:
                        break
                    if b"[y/N]" in output and not answered:
                        os.write(master, b"y\n")
                        answered = True
                elif child.poll() is not None:
                    break
            self.assertEqual(child.wait(timeout=5), 0, output.decode())
            self.assertIsNotNone(self.process.poll())
            self.assertEqual(self.binary.read_bytes(), Path(shutil.which("true")).read_bytes())
            self.assertIn(b"everything's installed!", output)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)

    def test_download_failure_and_help_preserve_the_running_installation(self):
        before = self.binary.read_bytes()
        for args, env in [(["--help"], self.env), ([], {**self.env, "FAIL_DOWNLOAD": "1"})]:
            result = subprocess.run(["sh", self.installer, *args], env=env, capture_output=True, text=True, timeout=15)
            self.assertNotIn("[y/N]", result.stderr)
            self.assertEqual(self.binary.read_bytes(), before)
            self.assertIsNone(self.process.poll())

    def test_preparation_is_idempotent_and_rejects_unknown_layouts(self):
        prepared = self.installer.read_text()
        PREPARER.prepare(self.installer)
        self.assertEqual(self.installer.read_text(), prepared)
        for anchor in (PREPARER.MOVE, PREPARER.MARKER, PREPARER.ENTRY):
            self.installer.write_text(SOURCE.replace(anchor, ""))
            with self.assertRaises(RuntimeError):
                PREPARER.prepare(self.installer)

    def test_source_installer_requires_approval_before_replacement(self):
        before = self.binary.read_bytes()
        result = subprocess.run(["sh", ROOT / "scripts/install.sh", self.stage / APP, self.destination],
                                env=self.env, capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Upgrade cancelled", result.stderr)
        self.assertEqual(self.binary.read_bytes(), before)
        self.assertIsNone(self.process.poll())


if __name__ == "__main__":
    unittest.main()
