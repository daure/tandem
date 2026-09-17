"""Exercise focus-aware refresh and MCP notifications with a PTY and fake Docker."""

import fcntl
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import tempfile
import termios
import time
import unittest

from stdio_smoke import Client


BINARY = Path(__file__).resolve().parents[3] / "target/debug/tandem"


class Terminal:
    def __init__(self, environment):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execve(str(BINARY), [str(BINARY)], environment)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 44, 150, 0, 0))
        self.output = b""

    def send(self, data):
        os.write(self.fd, data)

    def wait_for(self, predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            if select.select([self.fd], [], [], 0.05)[0]:
                self.output += os.read(self.fd, 65536)
        text = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", self.output)
        raise AssertionError(f"Terminal expectation timed out: {text[-12000:]!r}")

    def drain_for(self, duration):
        deadline = time.monotonic() + duration
        self.wait_for(lambda: time.monotonic() >= deadline, timeout=duration + 1)

    def close(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        finally:
            os.close(self.fd)


@unittest.skipUnless(os.name == "posix" and BINARY.is_file(), "Build Tandem before PTY tests")
class RefreshTests(unittest.TestCase):
    def test_focus_schedule_and_scoped_mcp_notifications(self):
        with tempfile.TemporaryDirectory(prefix="tandem-refresh-") as directory:
            root = Path(directory)
            home = root / "home"
            home.mkdir()
            docker = root / "docker"
            docker.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$TANDEM_HOME/docker-calls"\n')
            docker.chmod(0o755)
            environment = {
                **os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                "TANDEM_HOME": str(home), "TANDEM_NAMESPACE": "refresh-test",
                "XDG_STATE_HOME": str(root / "state"), "XDG_CONFIG_HOME": str(root / "config"),
                "TERM": "xterm-256color",
            }
            for key in list(environment):
                if key.startswith("TANDEM_KEY_") or key == "TANDEM_INSTRUCTIONS_FILE":
                    environment.pop(key)
            calls = home / "docker-calls"

            def inventory_calls():
                if not calls.exists():
                    return 0
                return sum(line.startswith("ps --all --quiet") for line in calls.read_text().splitlines())

            terminal = Terminal(environment)
            try:
                terminal.wait_for(lambda: inventory_calls() == 1 and b"Instances" in terminal.output)
                terminal.send(b"\x1b[O")
                terminal.drain_for(11)
                self.assertEqual(inventory_calls(), 1, "Unfocused TUI must skip ten-second inventory polls")

                client = Client(BINARY, environment)
                try:
                    client.tool("create_template", {"name": "mcp-visible"})
                    terminal.wait_for(lambda: b"mcp-visible" in terminal.output)
                    self.assertEqual(inventory_calls(), 1, "Template notification must not inspect Docker")

                    client.tool("update_template_manifest", {
                        "name": "mcp-visible", "confirmed": True,
                        "manifest": {"description": "Fresh MCP manifest"},
                    })
                    terminal.drain_for(1.5)
                    self.assertEqual(inventory_calls(), 1)
                    terminal.send(b"\x1b[I")
                    terminal.wait_for(lambda: inventory_calls() == 2, timeout=3)
                    terminal.output = b""
                    terminal.send(b"\r")
                    terminal.wait_for(lambda: b"Fresh MCP manifest" in terminal.output)
                    external = home / "templates" / "manual-only"
                    external.mkdir()
                    (external / "compose.yaml").write_text("services: {}\n")
                    terminal.wait_for(lambda: inventory_calls() == 3, timeout=12)
                    terminal.send(b"\x1b")
                    terminal.drain_for(0.5)
                    self.assertNotIn(b"manual-only", terminal.output)
                    self.assertNotIn(b"Refresh complete", terminal.output)
                    terminal.send(b"R")
                    terminal.wait_for(lambda: b"manual-only" in terminal.output and inventory_calls() == 4
                                      and b"Refresh complete" in terminal.output)
                    (external / "tandem.json").write_text("{ invalid json")
                    terminal.output = b""
                    terminal.send(b"R")
                    terminal.wait_for(lambda: b"Refresh completed with errors" in terminal.output
                                      and inventory_calls() == 5)
                    terminal.send(b"\x1b[O")

                    operation = client.tool("create_instance", {
                        "template": "mcp-visible", "name": "fails-config",
                        "confirmed": True, "timeout_seconds": 5,
                    })
                    self.assertEqual(operation["state"], "failed")
                    terminal.wait_for(lambda: inventory_calls() > 5)
                finally:
                    client.close()
            finally:
                terminal.close()


if __name__ == "__main__":
    unittest.main()
