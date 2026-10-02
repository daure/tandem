"""Four independent Docker provider packages sharing Tandem's event contract."""

from recipes import PYTHON_IMAGE, asset, write, write_json

PROFILES = ("message", "ticket", "system_event", "generic")


def sample(profile):
    payloads = {
        "message": {"author": "Alex", "channel": "development", "thread": "thread-events",
                    "text": "Can someone inspect the event feed?"},
        "ticket": {"key": "DEV-42", "title": "Build provider event ingestion", "status": "In progress",
                   "assignee": "Sam"},
        "system_event": {"resource": "build/events", "signal": "build.completed", "severity": "info",
                         "description": "The event-provider build completed successfully"},
        "generic": {"observation": "sample counter", "value": 42, "unit": "items"},
    }
    return {
        "schema_version": 1, "event_id": "assigned-by-provider", "stream": "developer-samples",
        "type": f"sample.{profile}.observed", "summary": f"Developer {profile} observation",
        "subject": f"sample-{profile}", "profile": profile, "data": payloads[profile],
        "people": [{"id": "sample:alex", "name": "Alex", "role": "participant"}],
        "attachments": [{"name": "example.txt", "url": "https://example.invalid/example.txt", "media_type": "text/plain"}],
        "relations": [{"kind": "relates_to", "subject": "DEV-42"}],
        "context": {"complete": False, "items": [{"summary": "Earlier supporting observation"}]},
        "metadata": {"fixture": True, "custom": {"team": "developer-tools"}},
    }


def create_providers(root):
    directory = root / ".tandem/templates/providers"
    if directory.exists() or directory.is_symlink():
        raise ValueError("Refusing to overwrite provider fixture paths")
    directory.mkdir(parents=True)
    services = {}
    volumes = {}
    for profile in PROFILES:
        name = profile.replace("_", "-")
        package = directory / name
        package.mkdir()
        asset("providers/provider.py", package / "src/provider.py")
        write_json(package / "sample.json", sample(profile))
        write_json(package / "provider.json", {
            "schema_version": 1, "name": f"dev-{name}", "profile": profile,
            "description": f"Synthetic {profile} events with durable delivery and feedback",
            "protocol": "tandem-events-v1", "feedback": ["received", "replayed", "acknowledged"],
        })
        write(package / "Dockerfile", f"FROM {PYTHON_IMAGE}\n"
              "ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1\nWORKDIR /app\n"
              "COPY src/provider.py sample.json ./\nCMD [\"python\", \"provider.py\"]\n")
        write(package / ".dockerignore", ".git\n.env\n__pycache__\n*.pyc\n")
        write(package / "README.md", f"# Developer {profile} provider\n\n"
              "Edit `sample.json` to change the normalized payload and custom metadata. "
              "The runtime assigns stable event IDs and stores retry/checkpoint state under `/state`. "
              "`src/provider.py` consumes and acknowledges provider-scoped feedback.\n")
        services[name] = {
            "build": {"context": f"./{name}"}, "network_mode": "host", "restart": "unless-stopped",
            "labels": {"tandem.provider": f"dev-{name}"},
            "environment": {
                "TANDEM_EVENTS_URL": "${TANDEM_EVENTS_URL:-http://127.0.0.1:7350}",
                "TANDEM_PROVIDER_TOKEN_FILE": "/run/secrets/provider-token",
                "TANDEM_PROVIDER_INTERVAL": "${TANDEM_PROVIDER_INTERVAL:-3}",
                "TANDEM_PROVIDER_BATCH": "${TANDEM_PROVIDER_BATCH:-1}",
            },
            "volumes": [
                {"type": "volume", "source": f"{name}-state", "target": "/state"},
                {"type": "bind", "source": f"${{TANDEM_PROVIDER_CREDENTIALS_DIR:?Run tandem providers-setup}}/dev-{name}.token",
                 "target": "/run/secrets/provider-token", "read_only": True, "bind": {"create_host_path": False}},
            ],
        }
        volumes[f"{name}-state"] = {}
    write_json(directory / "compose.providers.yaml", {
        "name": "${TANDEM_NAMESPACE:?Source the fixture env.sh}-providers",
        "services": services, "volumes": volumes,
    })
    return directory


def instance_catalog(root):
    nested = root / ".tandem/templates/instances"
    return nested if nested.is_dir() else root / ".tandem/templates"
