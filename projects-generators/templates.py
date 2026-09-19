"""Tandem recipes: clone sources once, then run from the writable instance workspace."""

import copy
import json

from recipes import asset, healthy, redis_infrastructure, write, write_json

FIXTURES = {
    "guestbook": {"repos": ["guestbook"], "web": "guestbook/frontend", "api": "guestbook/backend"},
    "greetings": {"repos": ["greetings-api", "greetings-ui"], "web": "greetings-ui", "api": "greetings-api"},
    "postcard": {"repos": ["postcard"], "web": "postcard"},
    "mailroom": {"repos": ["mailroom"], "web": "mailroom/frontend", "api": "mailroom/backend"},
    "repo-only": {"repos": ["repo-only"]},
    "compose-only": {"repos": []},
    "guidance-only": {"repos": []},
}
WORKSPACE = {"type": "bind", "source": "${TANDEM_WORKSPACE:?Tandem supplies the workspace}",
             "target": "/workspace", "bind": {"create_host_path": False}}
USER = "${TANDEM_UID:?Tandem supplies the uid}:${TANDEM_GID:?Tandem supplies the gid}"


def workspace_service(service, source, root):
    service = copy.deepcopy(service)
    service.pop("ports", None)
    service.update(build={"context": str(root / source)}, user=USER, working_dir="/workspace",
                   volumes=[WORKSPACE], command=["python", f"/workspace/{source}/server.py"])
    return service


def create_templates(root, names=FIXTURES):
    for name in names:
        fixture = FIXTURES[name]
        directory = root / ".tandem/templates" / name
        directory.mkdir(parents=True)
        if name == "guidance-only":
            asset("guidance/guidance-only.md", directory / "tandem-agents.md")
            continue
        if name == "repo-only":
            write_json(directory / "tandem.json", {
                "description": "Local Python development without services",
                "repositories": [{"source": str(root / "repo-only"), "target": "app"}],
            })
            asset("guidance/repo-only.md", directory / "tandem-agents.md")
            write(directory / ".gitignore", ".tandem-*\n")
            continue
        if name == "compose-only":
            write_json(directory / "compose.yaml", redis_infrastructure())
            asset("guidance/compose-only.md", directory / "tandem-agents.md")
            write(directory / ".gitignore", ".tandem-*\n")
            continue
        seed = root / fixture["repos"][0]
        model = json.loads((seed / "compose.yaml").read_text())
        if "include" in model:
            model["include"] = [str(seed / "infra/compose.yaml")]
        sources = model["services"]
        if name == "greetings":
            sources["web"] = json.loads((root / "greetings-ui/compose.yaml").read_text())["services"]["web"]
        services = {}
        routes = {}
        for role in ("web", "api"):
            if role not in fixture:
                continue
            services[role] = workspace_service(sources[role], fixture[role], root)
            environment = services[role].setdefault("environment", {})
            if role == "web":
                environment["API_BASE"] = "../api/"
                routes[role] = {"port": 8000, "strip_prefix": True,
                                "readiness_path": "", "readiness_contains": name.title()}
            else:
                environment.update(FAIL_READINESS="${FAIL_READINESS:-0}", STARTUP_DELAY="${STARTUP_DELAY:-0}")
                routes[role] = {"port": 8000, "strip_prefix": True,
                                "readiness_path": "health", "readiness_contains": '"status": "ready"'}
        one_shots = []
        if name == "greetings":
            migrate = workspace_service(sources["migrate"], fixture["api"], root)
            migrate["command"] = ["python", "/workspace/greetings-api/migrate.py"]
            migrate["environment"]["FAIL_MIGRATION"] = "${FAIL_MIGRATION:-0}"
            services["migrate"] = migrate
            one_shots.append("migrate")
        if name == "mailroom":
            worker = workspace_service(sources["worker"], fixture["api"], root)
            worker["command"] = ["python", "/workspace/mailroom/backend/worker.py"]
            worker["environment"]["FAIL_WORKER"] = "${FAIL_WORKER:-0}"
            worker["healthcheck"]["test"] = ["CMD", "python", "-c",
                "import sys; sys.path.insert(0, '/workspace/mailroom/backend'); import store; store.health()"]
            services["worker"] = worker
            services["api"]["depends_on"].update(healthy("worker"))
        model["services"] = services
        write_json(directory / "compose.yaml", model)
        write_json(directory / "tandem.json", {"description": f"{name.title()} development fixture",
                                               "repositories": [{"source": str(root / repo), "target": repo}
                                                                for repo in fixture["repos"]],
                                                "routes": routes, "one_shots": one_shots})
        asset(f"guidance/{name}.md", directory / "tandem-agents.md")
        write(directory / ".gitignore", ".env\n.tandem-*.compose.json\n")
        write(directory / ".env.example", "FAIL_READINESS=0\nSTARTUP_DELAY=0\n"
              + ("FAIL_MIGRATION=0\n" if name == "greetings" else "")
              + ("FAIL_WORKER=0\n" if name == "mailroom" else ""))
