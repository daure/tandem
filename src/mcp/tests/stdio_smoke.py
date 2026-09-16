"""Exercise the real stdio protocol; --live also starts/stops a routed Docker instance."""

import argparse
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time
import urllib.request


class Client:
    def __init__(self, binary, environment):
        self.process = subprocess.Popen(
            [str(binary), "mcp"], env=environment,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
        self.sequence = 0
        self.buffer = b""
        try:
            self.request("initialize", {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "tandem-smoke", "version": "1"},
            })
            self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        except Exception:
            self.close()
            raise

    def send(self, payload):
        self.process.stdin.write(json.dumps(payload).encode() + b"\n")
        self.process.stdin.flush()

    def request(self, method, params, timeout=20):
        self.sequence += 1
        self.send({"jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                message = json.loads(line)
                if message.get("id") == self.sequence:
                    assert "error" not in message, message
                    return message["result"]
            readable, _, _ = select.select([self.process.stdout], [], [], max(0, deadline - time.monotonic()))
            if not readable:
                break
            chunk = os.read(self.process.stdout.fileno(), 65536)
            assert chunk, f"MCP exited: {self.process.poll()}"
            self.buffer += chunk
        raise TimeoutError(f"MCP did not answer {method}")

    def tool(self, name, arguments=None, timeout=20):
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}}, timeout)
        assert not result.get("isError"), result
        return result["structuredContent"]

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        finally:
            try:
                self.process.stdin.close()
            except BrokenPipeError:
                pass
            finally:
                self.process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/tandem"))
    parser.add_argument("--live", action="store_true")
    options = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="tandem-stdio-") as directory:
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        namespace = f"tandem-stdio-{os.getpid()}"
        environment = {**os.environ, "TANDEM_HOME": directory, "TANDEM_NAMESPACE": namespace,
                       "TANDEM_GATEWAY_PORT": str(port), "XDG_STATE_HOME": f"{directory}/state"}
        environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
        client = Client(options.binary.resolve(), environment)
        try:
            tools = client.request("tools/list", {})["tools"]
            assert {"get_instructions", "get_template", "create_instance", "get_operation"} <= {tool["name"] for tool in tools}
            instructions = client.tool("get_instructions")
            Path(instructions["file"]).write_text("# Local smoke guidance\n", encoding="utf-8")
            assert client.tool("get_instructions")["markdown"] == "# Local smoke guidance\n"
            template = client.tool("create_template", {"name": "website"})
            assert Path(template["compose_file"]).parent == Path(template["directory"])
            assert client.tool("list_templates")["templates"][0]["name"] == "website"
            assert client.tool("get_template", {"name": "website"})["compose_source"].startswith("services:")
            rejected = client.request("tools/call", {"name": "create_instance", "arguments": {"template": "website", "name": "review"}})
            assert rejected.get("isError"), rejected
            if options.live:
                operation = client.tool("create_instance", {"template": "website", "name": "review", "confirmed": True, "wait": True, "timeout_seconds": 120}, timeout=150)
                assert operation["state"] == "succeeded", operation
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with opener.open(f"http://localhost:{port}/review/web/index.html", timeout=10) as response:
                    assert b"Tandem template" in response.read()
                observer = Client(options.binary.resolve(), environment)
                try:
                    assert observer.tool("list_instances")["instances"][0]["name"] == "review"
                finally:
                    observer.close()
                stopped = client.tool("stop_instance", {"name": "review", "confirmed": True})
                deadline = time.monotonic() + 30
                while stopped["state"] == "running" and time.monotonic() < deadline:
                    time.sleep(0.2)
                    stopped = client.tool("get_operation", {"id": stopped["id"]})
                assert stopped["state"] == "succeeded", stopped
                assert client.tool("list_instances")["instances"] == []
                assert Path(directory, "workspaces", "review").is_dir()
            print("Stdio handshake, object tool payloads, editable instructions and confirmation passed" + ("; live route and cross-process discovery passed." if options.live else "."))
        finally:
            client.close()
            if options.live:
                rendered = Path(directory, "templates", "website", f".tandem-{namespace}-review.compose.json")
                gateway = Path(directory, "gateway", "compose.json")
                for project, path in [(f"{namespace}-review", rendered), (f"{namespace}-gateway", gateway)]:
                    if path.is_file():
                        subprocess.run(["docker", "compose", "-p", project, "-f", str(path), "down"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=40, check=False)
                subprocess.run(["docker", "network", "rm", f"{namespace}-gateway"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20, check=False)


if __name__ == "__main__":
    main()
