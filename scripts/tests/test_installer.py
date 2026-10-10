import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("prepare_installer", ROOT / "scripts/prepare_installer.py")
PREPARER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARER)


class InstallerTests(unittest.TestCase):
    def run_installer(self, state="active", systemctl=True, companion_fails=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tandem = root / "tandem"
            tandem.write_text('''#!/bin/sh
printf 'tandem %s\n' "$*" >> "$CALLS"
[ "$COMPANION_FAILS" != 1 ]
''')
            tandem.chmod(0o700)
            if systemctl:
                program = root / "systemctl"
                program.write_text('''#!/bin/sh
printf 'systemctl %s\n' "$*" >> "$CALLS"
case "$*" in
    '--user show-environment') [ "$SERVICE_STATE" != unavailable ] ;;
    '--user is-active --quiet tandem-mcp.service')
        case "$SERVICE_STATE" in
            active|restart-fails) exit 0 ;;
            *) exit 3 ;;
        esac ;;
    '--user try-restart tandem-mcp.service') [ "$SERVICE_STATE" != restart-fails ] ;;
    *) exit 99 ;;
esac
''')
                program.chmod(0o700)
            installer = root / "installer.sh"
            installer.write_text('''#!/bin/sh
say() { printf '%s\n' "$*"; }
ensure() { "$@" || exit 1; }
ignore() { :; }
install() {
''' + PREPARER.MARKER + '''
}
install
''')
            PREPARER.prepare(installer)
            syntax = subprocess.run(["/bin/sh", "-n", str(installer)], capture_output=True, text=True)
            self.assertEqual(syntax.returncode, 0, syntax.stderr)
            calls = root / "calls"
            result = subprocess.run(
                ["/bin/sh", str(installer)],
                env={**os.environ, "PATH": str(root), "_install_dir": str(root),
                     "_install_temp": "unused", "_lib_install_temp": "unused",
                     "CALLS": str(calls), "SERVICE_STATE": state,
                     "COMPANION_FAILS": "1" if companion_fails else "0"},
                capture_output=True, text=True, timeout=10,
            )
            return result, calls.read_text().splitlines()

    def test_active_user_service_restarts_after_companion_setup(self):
        result, calls = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            "tandem opencode-setup",
            "systemctl --user show-environment",
            "systemctl --user is-active --quiet tandem-mcp.service",
            "systemctl --user try-restart tandem-mcp.service",
        ])
        self.assertIn("reconnect clients", result.stdout)

    def test_stopped_disabled_and_absent_services_are_preserved(self):
        for state in ["stopped", "disabled", "absent"]:
            with self.subTest(state=state):
                result, calls = self.run_installer(state)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, [
                    "tandem opencode-setup",
                    "systemctl --user show-environment",
                    "systemctl --user is-active --quiet tandem-mcp.service",
                ])

    def test_installation_without_systemctl_skips_service_management(self):
        result, calls = self.run_installer(systemctl=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["tandem opencode-setup"])

    def test_unavailable_user_manager_warns_without_failing_installation(self):
        result, calls = self.run_installer("unavailable")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["tandem opencode-setup", "systemctl --user show-environment"])
        self.assertIn("restart tandem-mcp.service manually", result.stderr)

    def test_restart_failure_does_not_report_installation_success(self):
        result, calls = self.run_installer("restart-fails")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls[-1], "systemctl --user try-restart tandem-mcp.service")
        self.assertNotIn("everything's installed!", result.stdout)

    def test_companion_failure_does_not_restart_the_service(self):
        result, calls = self.run_installer(companion_fails=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, ["tandem opencode-setup"])

    def test_preparing_existing_companion_installers_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory) / "installer.sh"
            installer.write_text(PREPARER.COMPANION_SETUP)
            PREPARER.prepare(installer)
            self.assertEqual(installer.read_text(), PREPARER.POST_INSTALL)
            PREPARER.prepare(installer)
            self.assertEqual(installer.read_text(), PREPARER.POST_INSTALL)


if __name__ == "__main__":
    unittest.main()
