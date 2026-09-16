"""Exercise generated fixtures through the real Tandem MCP server and Docker gateway."""

import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

ROOT = Path(__file__).resolve().parents[1]
CHECKOUT = ROOT.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(CHECKOUT / "src/mcp/tests"))
from generate import generate
from stdio_smoke import Client
from templates import FIXTURES


class Smoke:
    def __init__(self, root, client, port):
        self.root = root
        self.client = client
        self.origin = f"http://localhost:{port}"
        self.opener = build_opener(ProxyHandler({}))

    def request(self, instance, path, method="GET", body=None, status=200):
        data = None if body is None else json.dumps(body).encode()
        request = Request(f"{self.origin}/{instance}/{path}", data=data, method=method,
                          headers={"Content-Type": "application/json"})
        try:
            response = self.opener.open(request, timeout=10)
        except HTTPError as error:
            response = error
        with response:
            payload = response.read().decode()
            assert response.status == status, (response.status, payload)
            return payload, response.geturl()

    def api(self, instance, path, **options):
        return json.loads(self.request(instance, f"api/{path}", **options)[0])

    def start(self, template, instance, expected="succeeded", timeout=300):
        operation = self.client.tool("create_instance", {
            "template": template, "name": instance, "confirmed": True,
            "wait": True, "timeout_seconds": timeout,
        }, timeout=timeout + 20)
        if operation["state"] != expected:
            namespace = json.loads((self.root / "fixtures.json").read_text())["environment"]["TANDEM_NAMESPACE"]
            rendered = self.root / ".tandem/templates" / template / f".tandem-{namespace}-{instance}.compose.json"
            subprocess.run(["docker", "compose", "-p", f"{namespace}-{instance}", "-f", str(rendered),
                            "logs", "--no-color"], check=False, timeout=20)
        assert operation["state"] == expected, operation
        print(f"{instance}: {expected}", flush=True)

    def stop(self, instance):
        operation = self.client.tool("stop_instance", {"name": instance, "confirmed": True})
        deadline = time.monotonic() + 60
        while operation["state"] == "running" and time.monotonic() < deadline:
            time.sleep(0.2)
            operation = self.client.tool("get_operation", {"id": operation["id"]})
        assert operation["state"] == "succeeded", operation

    def check_routes_and_git(self, template, instance):
        assert template.title() in self.request(instance, "web/")[0]
        self.request(instance, "web-other/", status=404)
        assert "font-family" in self.request(instance, "web/style.css")[0]
        for path in ("web/about/", "web/about", "web/redirect"):
            body, url = self.request(instance, path)
            assert f"About {template.title()}" in body
            assert url == f"{self.origin}/{instance}/web/about/", url
        for repo in FIXTURES[template]["repos"]:
            clone = self.root / ".tandem/workspaces" / instance / repo
            assert clone.stat().st_uid == os.getuid()
            assert git(clone, "remote", "get-url", "origin") == str(self.root / repo)
            assert git(clone, "status", "--porcelain") == ""

    def exercise(self):
        infrastructure = self.root / "greetings-api/infra/compose.yaml"
        model = json.loads(infrastructure.read_text())
        delay = self.root / "greetings-api/infra/slow-init.sh"
        delay.write_text("sleep 5\n")
        model["services"]["db"]["volumes"].append({
            "type": "bind", "source": str(delay),
            "target": "/docker-entrypoint-initdb.d/slow-init.sh", "read_only": True,
        })
        infrastructure.write_text(json.dumps(model))
        templates = self.client.tool("list_templates")["templates"]
        assert {template["name"] for template in templates} == set(FIXTURES)
        for template in FIXTURES:
            instance = f"{template}-one"
            self.start(template, instance)
            self.check_routes_and_git(template, instance)

        self.start("guestbook", "guestbook-two")
        self.api("guestbook-one", "greeting", method="PUT", body={"message": "Hello from one"})
        assert self.api("guestbook-two", "greeting")["message"] == "Hello, world!"
        self.api("guestbook-one", "greeting", method="PUT", body={"message": ""}, status=400)
        clone = self.root / ".tandem/workspaces/guestbook-one/guestbook"
        git(clone, "switch", "-c", "local-edits")
        config = clone / "backend/config.json"
        payload = json.loads(config.read_text())
        payload["greeting"] = "Hello from my checkout"
        config.write_text(json.dumps(payload))
        (clone / "notes.txt").write_text("Keep this untracked file")
        before = git(clone, "status", "--porcelain")
        self.stop("guestbook-one")
        assert self.api("guestbook-two", "greeting")["message"] == "Hello, world!"
        self.start("guestbook", "guestbook-one")
        assert self.api("guestbook-one", "greeting")["message"] == "Hello from my checkout"
        assert git(clone, "branch", "--show-current") == "local-edits"
        assert git(clone, "status", "--porcelain") == before
        assert (clone / "notes.txt").read_text() == "Keep this untracked file"

        self.start("greetings", "greetings-two")
        self.api("greetings-one", "greeting", method="PUT", body={"message": "Persist me"})
        assert self.api("greetings-two", "greeting")["message"] == "Hello, world!"
        self.stop("greetings-one")
        self.start("greetings", "greetings-one")
        assert self.api("greetings-one", "greeting")["message"] == "Persist me"

        job = self.api("mailroom-one", "jobs", method="POST", body={"message": "Hello, worker!"}, status=202)
        for _ in range(30):
            result = self.api("mailroom-one", f"jobs/{job['id']}")
            if result["status"] == "done":
                break
            time.sleep(0.2)
        assert result["result"] == "Delivered: Hello, worker!", result
        self.api("mailroom-one", "jobs/missing", status=404)

        for template, setting in [("postcard", "FAIL_CLONE"), ("greetings", "FAIL_MIGRATION"),
                                  ("mailroom", "FAIL_WORKER"), ("guestbook", "FAIL_READINESS")]:
            fault = self.root / ".tandem/templates" / template / ".env"
            fault.write_text(f"{setting}=1\n")
            try:
                self.start(template, f"{template}-broken", expected="failed", timeout=15)
            finally:
                fault.unlink()
        self.start("postcard", "postcard-broken")
        self.check_routes_and_git("postcard", "postcard-broken")
        self.standalone_apps()
        print("Routes, independent clones, dirty restart, SQL isolation/persistence, jobs, and injected failures passed.", flush=True)

    def standalone_apps(self):
        namespace = json.loads((self.root / "fixtures.json").read_text())["environment"]["TANDEM_NAMESPACE"]
        for repository in ("guestbook", "greetings-ui", "postcard", "mailroom"):
            with socket.socket() as web, socket.socket() as api:
                web.bind(("127.0.0.1", 0))
                api.bind(("127.0.0.1", 0))
                web_port, api_port = web.getsockname()[1], api.getsockname()[1]
            environment = {**os.environ, "WEB_PORT": str(web_port), "API_PORT": str(api_port)}
            command = ["docker", "compose", "-p", f"{namespace}-standalone-{repository}",
                       "-f", str(self.root / repository / "compose.yaml")]
            try:
                result = subprocess.run(command + ["up", "--build", "--wait", "--wait-timeout", "90"],
                                        env=environment, capture_output=True, text=True, timeout=180)
                assert result.returncode == 0, result.stderr
                with self.opener.open(f"http://localhost:{web_port}/", timeout=10) as response:
                    assert repository.split("-")[0].title() in response.read().decode()
                if repository == "postcard":
                    continue
                endpoint = "jobs" if repository == "mailroom" else "greeting"
                request = Request(f"http://localhost:{web_port}/api/{endpoint}",
                                  method="POST" if endpoint == "jobs" else "PUT",
                                  data=json.dumps({"message": "Hello through the proxy"}).encode(),
                                  headers={"Content-Type": "application/json"})
                with self.opener.open(request, timeout=10) as response:
                    assert json.loads(response.read())["message"] == "Hello through the proxy"
            finally:
                result = subprocess.run(command + ["down", "--volumes", "--remove-orphans"],
                                        env=environment, capture_output=True, text=True, timeout=60)
                assert result.returncode == 0, result.stderr
            print(f"{repository}: standalone Compose passed", flush=True)


def git(repo, *arguments):
    result = subprocess.run(["git", "-C", str(repo), *arguments], check=True, capture_output=True, text=True)
    return result.stdout.strip()


def cleanup(root, namespace):
    failures = []
    for path in sorted((root / ".tandem/templates").glob("*/.tandem-*.compose.json")):
        instance = path.name.removeprefix(f".tandem-{namespace}-").removesuffix(".compose.json")
        result = subprocess.run(["docker", "compose", "-p", f"{namespace}-{instance}", "-f", str(path),
                                 "down", "--volumes", "--remove-orphans"], capture_output=True, text=True, timeout=60)
        if result.returncode:
            failures.append(result.stderr)
    gateway = root / ".tandem/gateway/compose.json"
    if gateway.exists():
        result = subprocess.run(["docker", "compose", "-p", f"{namespace}-gateway", "-f", str(gateway), "down"],
                                capture_output=True, text=True, timeout=60)
        if result.returncode:
            failures.append(result.stderr)
        result = subprocess.run(["docker", "network", "rm", f"{namespace}-gateway"],
                                capture_output=True, text=True, timeout=20)
        if result.returncode:
            failures.append(result.stderr)
    if failures:
        raise RuntimeError("Smoke cleanup failed; retained fixture root " + str(root) + "\n" + "\n".join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=CHECKOUT / "target/debug/tandem")
    options = parser.parse_args()
    if not options.binary.is_file():
        parser.error("Build Tandem first with cargo build")
    projects = CHECKOUT / "projects"
    projects.mkdir(exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix=".smoke-", dir=projects))
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    with contextlib.redirect_stdout(io.StringIO()):
        generate(root, seed_commits=True, port=port)
    environment = {**os.environ, **json.loads((root / "fixtures.json").read_text())["environment"],
                   "XDG_STATE_HOME": str(root / "state")}
    environment.pop("TANDEM_INSTRUCTIONS_FILE", None)
    client = Client(options.binary.resolve(), environment)
    succeeded = False
    try:
        Smoke(root, client, port).exercise()
        succeeded = True
    finally:
        client.close()
        cleanup(root, environment["TANDEM_NAMESPACE"])
        if succeeded:
            shutil.rmtree(root)
        else:
            print(f"Retained failed smoke workspace: {root}", file=sys.stderr)


if __name__ == "__main__":
    main()
