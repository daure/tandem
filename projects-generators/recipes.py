"""Source repositories and standalone Compose applications for the four fixtures."""

import json
from pathlib import Path

ASSETS = Path(__file__).parent / "assets"
PYTHON_IMAGE = "python:3.13.5-slim-bookworm"
REPOSITORIES = ("guestbook", "greetings-api", "greetings-ui", "postcard", "mailroom")
DATABASE_URL = "postgresql://fixture:fixture@db:5432/greetings"
REDIS_URL = "redis://redis:6379/0"


def write(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def write_json(path, value):
    write(path, json.dumps(value, indent=2) + "\n")


def asset(source, target, replacements=None):
    content = (ASSETS / source).read_text(encoding="utf-8")
    for name, value in (replacements or {}).items():
        content = content.replace(f"@@{name}@@", value)
    write(target, content)


def dockerfile(directory, dependencies=""):
    write(directory / "requirements.txt", dependencies)
    write(directory / "Dockerfile", f"""FROM {PYTHON_IMAGE}
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY . .
EXPOSE 8000
CMD ["python", "server.py"]
""")
    write(directory / ".dockerignore", ".git\n.env\n.venv\n__pycache__\n*.pyc\n")


def web(directory, title, description, mode="greeting"):
    replacements = {"TITLE": title, "DESCRIPTION": description, "MODE": mode,
                    "ACTION": "Send greeting" if mode == "jobs" else "Save greeting"}
    for source in sorted((ASSETS / "web").rglob("*")):
        if source.is_file() and "__pycache__" not in source.parts:
            asset(source.relative_to(ASSETS), directory / source.relative_to(ASSETS / "web"), replacements)
    dockerfile(directory)


def api(directory, name, storage, dependencies=""):
    asset("api/server.py", directory / "server.py")
    asset(f"api/{storage}.py", directory / "store.py")
    write_json(directory / "config.json", {"app": name, "greeting": "Hello, world!"})
    dockerfile(directory, dependencies)


def healthy(service):
    return {service: {"condition": "service_healthy"}}


def completed(service):
    return {service: {"condition": "service_completed_successfully"}}


def http_health(path="health"):
    return {"test": ["CMD", "python", "-c",
                     f"import urllib.request; urllib.request.urlopen('http://localhost:8000/{path}', timeout=2)"],
            "interval": "2s", "timeout": "3s", "retries": 30, "start_period": "5s"}


def app_service(context, port=None, web_app=False):
    service = {"build": {"context": context}, "init": True,
               "healthcheck": http_health("" if web_app else "health")}
    if port:
        variable = "WEB_PORT" if web_app else "API_PORT"
        service["ports"] = [f"127.0.0.1:${{{variable}:-{port}}}:8000"]
    return service


def postgres_infrastructure():
    return {
        "services": {"db": {
            "image": "postgres:17.5-alpine",
            "environment": {"POSTGRES_USER": "fixture", "POSTGRES_PASSWORD": "fixture",
                            "POSTGRES_DB": "greetings"},
            "volumes": ["postgres-data:/var/lib/postgresql/data"],
            # The image's initialization server accepts Unix sockets before TCP is ready.
            "healthcheck": {"test": ["CMD", "pg_isready", "-h", "127.0.0.1", "-U", "fixture", "-d", "greetings"],
                            "interval": "2s", "timeout": "3s", "retries": 30},
        }},
        "volumes": {"postgres-data": {}},
    }


def redis_infrastructure():
    return {
        "services": {"redis": {
            "image": "redis:7.4.5-alpine", "command": ["redis-server", "--appendonly", "yes"],
            "volumes": ["redis-data:/data"],
            "healthcheck": {"test": ["CMD", "redis-cli", "ping"], "interval": "2s",
                            "timeout": "3s", "retries": 30},
        }},
        "volumes": {"redis-data": {}},
    }


def guestbook(root):
    repo = root / "guestbook"
    api(repo / "backend", "guestbook", "memory")
    web(repo / "frontend", "Guestbook", "A greeting shared by a frontend and an in-memory API in one repository.")
    backend = app_service("./backend")
    frontend = app_service("./frontend", 18080, True)
    frontend["depends_on"] = healthy("api")
    write_json(repo / "compose.yaml", {"services": {"api": backend, "web": frontend}})


def greetings(root):
    backend = root / "greetings-api"
    frontend = root / "greetings-ui"
    api(backend, "greetings", "postgres", "psycopg[binary]==3.2.9\n")
    for name in ("migrate.py", "schema.sql"):
        asset(f"api/{name}", backend / name)
    web(frontend, "Greetings", "Two repositories, one greeting persisted in PostgreSQL.")
    write_json(backend / "infra/compose.yaml", postgres_infrastructure())
    migrate = {"build": {"context": "."}, "command": ["python", "migrate.py"],
               "environment": {"DATABASE_URL": DATABASE_URL}, "depends_on": healthy("db")}
    service = app_service(".", 18081)
    service.update(environment={"DATABASE_URL": DATABASE_URL}, depends_on=completed("migrate"))
    write_json(backend / "compose.yaml", {"include": ["infra/compose.yaml"],
                                         "services": {"api": service, "migrate": migrate}})
    frontend_service = app_service(".", 18082, True)
    frontend_service["depends_on"] = healthy("api")
    write_json(frontend / "compose.yaml", {"include": ["../greetings-api/compose.yaml"],
                                          "services": {"web": frontend_service}})


def postcard(root):
    repo = root / "postcard"
    web(repo, "Postcard", "A static hello-world page with assets, nested links, and redirects.")
    asset("postcard.html", repo / "public/index.html")
    (repo / "public/app.js").unlink()
    write_json(repo / "compose.yaml", {"services": {"web": app_service(".", 18083, True)}})


def mailroom(root):
    repo = root / "mailroom"
    api(repo / "backend", "mailroom", "jobs", "redis==5.2.1\n")
    asset("api/worker.py", repo / "backend/worker.py")
    web(repo / "frontend", "Mailroom", "Send a greeting to a Redis-backed worker and wait for delivery.", "jobs")
    write_json(repo / "infra/compose.yaml", redis_infrastructure())
    worker = {"build": {"context": "./backend"}, "command": ["python", "worker.py"],
              "init": True, "environment": {"REDIS_URL": REDIS_URL}, "depends_on": healthy("redis"),
              "healthcheck": {"test": ["CMD", "python", "-c", "import store; store.health()"],
                              "interval": "2s", "timeout": "3s", "retries": 10}}
    backend = app_service("./backend")
    backend.update(environment={"REDIS_URL": REDIS_URL}, depends_on=healthy("worker"))
    frontend = app_service("./frontend", 18084, True)
    frontend["depends_on"] = healthy("api")
    write_json(repo / "compose.yaml", {"include": ["infra/compose.yaml"],
                                      "services": {"api": backend, "web": frontend, "worker": worker}})


def create_repositories(root):
    for name in REPOSITORIES:
        (root / name).mkdir()
    guestbook(root)
    greetings(root)
    postcard(root)
    mailroom(root)
    for name in REPOSITORIES:
        write(root / name / ".gitignore", ".env\n.venv/\n__pycache__/\n*.pyc\n.tandem-*.compose.json\n")
        write(root / name / "README.md", f"""# {name}

A local Tandem development fixture. Source and infrastructure are intentionally small.

Run `docker compose up --build -d` from this repository. Stop with `docker compose down`;
named data volumes are preserved. Compose publishes only loopback HTTP ports.

Standalone URLs: Guestbook 18080, Greetings API 18081, Greetings UI 18082,
Postcard 18083, Mailroom 18084. Set `WEB_PORT` or `API_PORT` to override published ports.
Greetings UI includes the sibling API's Compose app and proxies to its internal service;
start the full Greetings app from `greetings-ui`, or the API alone from `greetings-api`.
The API owns its PostgreSQL Compose include and schema migration.

For isolated instances, use the generated Tandem templates instead of standalone Compose.
Templates clone this repository's committed `main` branch on first start. Work in the
instance clone; existing checkouts are preserved on subsequent starts.
Refresh the browser after editing frontend assets. Restart the affected service after
Python changes. Dependency and Dockerfile changes require rebuilding its image.

These unauthenticated apps and demo credentials are for local development only.
""")
