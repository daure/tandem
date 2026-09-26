"""Opt-in live proof: python3 src/environments/opencode/tests/live.py.

Creates isolated OpenCode state and a disposable Zellij session. Sends no model prompts.
"""

import json
import fcntl
import os
import pty
from pathlib import Path
import shutil
import socket
import subprocess
import struct
import tempfile
import termios
import threading
import time
import urllib.request


def wait_for(check, timeout=45):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except Exception as error:
            last = error
        time.sleep(0.15)
    raise AssertionError(f"Live check timed out: {last}")


def main():
    opencode = shutil.which("opencode")
    zellij = shutil.which("zellij")
    assert opencode and zellij, "OpenCode and Zellij must be installed"
    plugin = Path(__file__).resolve().parents[1] / "bridge.mjs"
    with tempfile.TemporaryDirectory(prefix="tandem-live-") as temporary:
        root = Path(temporary)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("OPENCODE_", "ZELLIJ", "XDG_", "OC_"))}
        env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"),
                   XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"),
                   XDG_CACHE_HOME=str(root / "cache"), OPENCODE_DISABLE_AUTOUPDATE="1",
                   OPENCODE_DISABLE_MODELS_FETCH="1", OPENCODE_DISABLE_PROJECT_CONFIG="1")
        workspace = root / "workspace"
        workspace.mkdir()
        config = root / "config" / "opencode"
        config.mkdir(parents=True)
        (config / "opencode.json").write_text(json.dumps({"$schema": "https://opencode.ai/config.json", "mcp": {}, "plugin": []}))
        (config / "tui.json").write_text(json.dumps({"$schema": "https://opencode.ai/tui.json", "plugin": [str(plugin)]}))
        zellij_config = root / "zellij.kdl"
        zellij_config.write_text('default_shell "/bin/sh"\npane_frames true\n')
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        url = f"http://127.0.0.1:{port}"

        def api(path, body=None):
            request = urllib.request.Request(url + path, data=None if body is None else json.dumps(body).encode(),
                                             headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=3) as response:
                return json.load(response)

        name = f"tandem-live-{os.getpid()}"

        def zj(*args):
            return subprocess.check_output([zellij, "--session", name, *args], env=env,
                                           cwd=workspace, text=True, stderr=subprocess.STDOUT, timeout=10).strip()

        with (root / "server.log").open("w") as log:
            server = subprocess.Popen([opencode, "serve", "--port", str(port), "--hostname", "127.0.0.1"],
                                      env={**env, "OPENCODE_PURE": "1"}, cwd=workspace,
                                      stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            created = False
            client = None
            master = None
            try:
                wait_for(lambda: api("/global/health"))
                first = api("/session", {"title": "Tandem live first"})["id"]
                second = api("/session", {"title": "Tandem live second"})["id"]
                subprocess.run([zellij, "--config", str(zellij_config), "attach", "--create-background", name],
                               env=env, cwd=workspace, check=True, capture_output=True, timeout=15)
                created = True
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
                client = subprocess.Popen([zellij, "attach", name], env={**env, "TERM": "xterm-256color"},
                                          cwd=workspace, stdin=slave, stdout=slave, stderr=slave,
                                          start_new_session=True)
                os.close(slave)
                def drain():
                    try:
                        while os.read(master, 65536):
                            pass
                    except OSError:
                        pass
                threading.Thread(target=drain, daemon=True).start()
                wait_for(lambda: len(zj("action", "list-clients").splitlines()) > 1)
                pane = zj("action", "new-pane", "--stacked", "--cwd", str(workspace), "--",
                          opencode, "attach", url, "--dir", str(workspace), "--session", first)
                pane_id = int(pane.removeprefix("terminal_"))
                presence = root / "state" / "tandem" / "opencode"

                def records():
                    return [json.loads(path.read_text()) for path in presence.glob("*.json")]

                record = wait_for(lambda: next((r for r in records() if r["id"] == first), None))
                assert record["zellij_session"] == name, record
                assert record["pane_id"] == pane_id, record
                assert record["server"] == url, record
                assert record["activity"] == "idle", record
                print("PASS: real OpenCode TUI loads companion and publishes exact session/pane/server identity")
                api("/tui/select-session", {"sessionID": second})
                wait_for(lambda: next((r for r in records() if r["id"] == second), None))
                assert not any(r["id"] == first for r in records())
                print("PASS: changing conversations updates the existing pane registration")

                other = zj("action", "new-pane", "--stacked", "--", "sleep", "60")
                zj("action", "focus-pane-id", pane)
                panes = json.loads(zj("action", "list-panes", "--all", "--json"))
                target = next(p for p in panes if not p["is_plugin"] and p["id"] == pane_id)
                sibling = next(p for p in panes if not p["is_plugin"] and p["id"] == int(other.removeprefix("terminal_")))
                assert target["is_focused"], target
                assert not target["is_suppressed"] and target["pane_content_rows"] > 1, target
                assert sibling["is_suppressed"] or target["pane_content_rows"] > sibling["pane_content_rows"], (target, sibling)
                print("PASS: focusing a stacked pane expands the exact target")

                zj("action", "close-pane", "--pane-id", pane)
                wait_for(lambda: not records() or all(not Path(f"/proc/{r['pid']}").exists() for r in records()), timeout=15)
                print("PASS: closing the client invalidates its presence")
            except Exception as error:
                if isinstance(error, subprocess.CalledProcessError):
                    print(error.output)
                print((root / "server.log").read_text()[-8000:])
                if created:
                    try:
                        print(zj("action", "dump-screen", "--pane-id", pane))
                    except Exception:
                        pass
                raise
            finally:
                if created:
                    subprocess.run([zellij, "kill-session", name], env=env, capture_output=True, timeout=10)
                if client:
                    client.terminate()
                    client.wait(timeout=5)
                if master is not None:
                    os.close(master)
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=5)


if __name__ == "__main__":
    main()
