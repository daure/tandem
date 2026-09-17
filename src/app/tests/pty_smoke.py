"""Test template/create/info/stop flows in a real terminal (requires pexpect, pyte, Docker)."""

import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

import pexpect
import pyte


def main():
    with tempfile.TemporaryDirectory(prefix="tandem-tui-") as directory:
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        namespace = f"tandem-tui-{os.getpid()}"
        environment = {**os.environ, "TANDEM_HOME": directory, "TANDEM_NAMESPACE": namespace,
                       "TANDEM_GATEWAY_PORT": str(port), "XDG_STATE_HOME": f"{directory}/state",
                       "TERM": "xterm-256color"}
        for key in list(environment):
            if key.startswith("TANDEM_KEY_") or key == "TANDEM_INSTRUCTIONS_FILE":
                environment.pop(key)
        terminal = pexpect.spawn(str(Path("target/debug/tandem").resolve()), env=environment,
                                 dimensions=(44, 150), encoding="utf-8", codec_errors="replace", timeout=10)
        screen = pyte.Screen(150, 44)
        stream = pyte.Stream(screen)

        def wait_for(predicate, timeout=25):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                try:
                    stream.feed(terminal.read_nonblocking(size=65536, timeout=0.2))
                except pexpect.TIMEOUT:
                    pass
                text = "\n".join(screen.display)
                if predicate(text):
                    return text
            raise AssertionError("Terminal expectation timed out:\n" + "\n".join(screen.display))

        try:
            wait_for(lambda text: "Templates root" in text)
            terminal.send("t")
            wait_for(lambda text: "New template" in text)
            terminal.send("website")
            terminal.send("\r")
            wait_for(lambda text: "create_template / website" in text and "succeeded" in text)
            wait_for(lambda text: "Compose file" in text)
            terminal.send("\r")
            info = wait_for(lambda text: "Template / website" in text and "compose.yaml" in text and "nginx:1.28-alpine" in text)
            assert "nginx:1.28-alpine" in info
            terminal.send("\x1b")
            wait_for(lambda text: "Template / website" not in text)
            terminal.send("n")
            wait_for(lambda text: "Start instance / website" in text)
            terminal.send("ui-review")
            terminal.send("\r")
            wait_for(lambda text: "Ready: gateway content assertions passed" in text, timeout=150)
            wait_for(lambda text: "1 instance" in text)
            terminal.send(" ")
            wait_for(lambda text: any("ui-review" in line[:55] for line in text.splitlines()))
            terminal.send("\x1b[B")
            tree = wait_for(lambda text: "Workspace" in text and f"http://localhost:{port}/ui-review/web/" in text)
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with opener.open(f"http://localhost:{port}/ui-review/web/index.html", timeout=10) as response:
                assert b"Tandem template" in response.read()
            terminal.send("s")
            wait_for(lambda text: "Stop ui-review?" in text)
            terminal.send("\x13")
            wait_for(lambda text: "stop_instance / ui-review · succeeded" in text)
            assert Path(directory, "workspaces", "ui-review").is_dir()
            print("Real TUI template allocation, info dialog, tree expansion, create, browser route, and stop passed.\n")
            print(tree)
            terminal.send("\x11")
            terminal.expect(pexpect.EOF, timeout=10)
        finally:
            terminal.close(force=True)
            rendered = Path(directory, "templates", "website", f".tandem-{namespace}-ui-review.compose.json")
            gateway = Path(directory, "gateway", "compose.json")
            for project, path in [(f"{namespace}-ui-review", rendered), (f"{namespace}-gateway", gateway)]:
                if path.is_file():
                    subprocess.run(["docker", "compose", "-p", project, "-f", str(path), "down"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=40, check=False)
            subprocess.run(["docker", "network", "rm", f"{namespace}-gateway"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20, check=False)


if __name__ == "__main__":
    main()
