"""Synthetic Slack, Jira, Datadog, and GitHub Docker provider packages."""

from recipes import PYTHON_IMAGE, asset, write, write_json

PROVIDERS = {"message": "slack", "ticket": "jira", "system_event": "datadog", "generic": "github"}
PROFILES = tuple(PROVIDERS)


def sample(profile):
    scenarios = {
        "message": ("messages", "slack.message.posted", "PR review requested for gateway timeout handling",
                    "slack:pull-requests:gateway-timeout", "Maya Patel", "slack:maya-patel", "author",
                    {"author": "Maya Patel", "channel": "#pull-requests", "thread": "gateway-timeout",
                     "text": "Could someone review gateway PR #87? It adds bounded retries for upstream timeouts."},
                    "pr-review-notes.md", "The PR includes regression coverage for upstream timeouts."),
        "ticket": ("backlog", "jira.issue.updated", "PLAT-142: Fix gateway timeouts",
                   "PLAT-142", "Owen Brooks", "jira:owen-brooks", "assignee",
                   {"key": "PLAT-142", "title": "Fix gateway timeouts", "status": "To Do",
                    "assignee": "Owen Brooks"},
                   "gateway-timeout-reproduction.txt", "Support reported intermittent HTTP 504 responses."),
        "system_event": ("production-gateway-issue", "datadog.monitor.alert", "Production gateway latency exceeds threshold",
                         "datadog:monitor:gateway-latency", "Leah Chen", "datadog:leah-chen", "on-call",
                         {"resource": "production/gateway", "signal": "monitor.alert", "severity": "critical",
                          "environment": "production", "description": "Gateway p95 latency is 2400 ms; the threshold is 1000 ms."},
                         "gateway-latency-snapshot.txt", "Gateway latency increased after release v2.8.1."),
        "generic": ("releases", "github.release.published", "Gateway v2.8.1 published",
                    "github:gateway:release:v2.8.1", "Nina Torres", "github:nina-torres", "publisher",
                    {"repository": "northstar/gateway", "tag": "v2.8.1", "name": "Gateway v2.8.1",
                     "prerelease": False, "release_notes": "Adds bounded retries for upstream timeouts."},
                    "release-notes.md", "PR #87 adds bounded retries for upstream timeouts."),
    }
    stream, event_type, summary, subject, person, person_id, role, data, attachment, context = scenarios[profile]
    event = {
        "schema_version": 1, "event_id": "assigned-by-provider", "stream": stream,
        "type": event_type, "summary": summary, "subject": subject, "profile": profile, "data": data,
        "people": [{"id": person_id, "name": person, "role": role}],
        "attachments": [{"name": attachment, "url": f"https://fixtures.invalid/gateway/{attachment}", "media_type": "text/plain"}],
        "relations": [{"kind": "relates_to", "subject": "PLAT-142"},
                      {"kind": "relates_to", "subject": "github:gateway:pull:87"}],
        "context": {"complete": False, "items": [{"summary": context}]},
        "metadata": {"fixture": True, "custom": {"team": "platform", "application": "gateway"}},
    }
    if profile == "ticket":
        event["url"] = "https://jira.example.com/browse/PLAT-142"
    if profile != "message":
        return [event]
    support = {
        **event, "summary": "Support asks for help with gateway timeouts", "subject": "slack:support:gateway-timeouts",
        "data": {"author": "Maya Patel", "channel": "#support", "thread": "gateway-timeouts",
                 "text": "Customers report intermittent HTTP 504 responses. What should support check before escalating?"},
        "attachments": [{"name": "support-error-samples.txt", "url": "https://fixtures.invalid/gateway/support-error-samples.txt",
                         "media_type": "text/plain"}],
        "context": {"complete": False, "items": [{"summary": "Three customers reported timeouts in the last ten minutes."}]},
    }
    events = []
    for message in [event, support]:
        events.append(message)
        events.append({
            **message, "stream": "reactions", "type": "slack.reaction.added",
            "summary": f"Owen Brooks reacted with eyes in {message['data']['channel']}",
            "subject": message["subject"] + ":reaction:eyes",
            "data": {**message["data"], "author": "Owen Brooks",
                     "text": "Owen Brooks added :eyes: to Maya Patel's message."},
            "people": [{"id": "slack:owen-brooks", "name": "Owen Brooks", "role": "reactor"}],
            "relations": [{"kind": "reacts_to", "subject": message["subject"]}],
            "context": {"complete": False, "items": [{"summary": message["data"]["text"]}]},
            "metadata": {"fixture": True, "custom": {"team": "platform", "application": "gateway",
                         "reaction": "eyes", "message_subject": message["subject"]}},
        })
    return events


def create_providers(root):
    directory = root / ".tandem/templates/providers"
    if directory.exists() or directory.is_symlink():
        raise ValueError("Refusing to overwrite provider fixture paths")
    directory.mkdir(parents=True)
    services = {}
    volumes = {}
    for profile, name in PROVIDERS.items():
        package = directory / name
        package.mkdir()
        asset("providers/provider.py", package / "src/provider.py")
        write_json(package / "sample.json", sample(profile))
        write_json(package / "provider.json", {
            "schema_version": 2, "name": name,
            "description": f"Synthetic {name.title()} events with durable delivery and feedback",
            "protocol": "tandem-events-v1", "feedback": ["received", "replayed", "acknowledged"],
            "streams": [{"name": stream, "profile": profile}
                        for stream in sorted({event["stream"] for event in sample(profile)})],
            "stream_control": True,
        })
        write(package / "Dockerfile", f"FROM {PYTHON_IMAGE}\n"
              "ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1\nWORKDIR /app\n"
              "COPY src/provider.py sample.json ./\nCMD [\"python\", \"provider.py\"]\n")
        write(package / ".dockerignore", ".git\n.env\n__pycache__\n*.pyc\n")
        write(package / "README.md", f"# {name.title()} provider\n\n"
              "Synthetic events; no external account or API connection is required. "
              "Edit the ordered event samples in `sample.json` to change payloads and streams. "
              "Declare each stream's name and profile in the schema-version-2 `provider.json`; "
              "one collector can expose streams with different profiles. "
              "The runtime cycles through the samples and adds a one-based `metadata.stream_sequence` per stream. "
              "It polls authenticated stream controls, stops generating disabled streams before acknowledgment, "
              "and resumes with new events only. "
              "The runtime assigns stable event IDs and stores retry/checkpoint state under `/state`. "
              "`src/provider.py` consumes and acknowledges provider-scoped feedback.\n")
        services[name] = {
            "build": {"context": f"./{name}"}, "network_mode": "host", "restart": "unless-stopped",
            "labels": {"tandem.provider": name},
            "environment": {
                "TANDEM_EVENTS_URL": "${TANDEM_EVENTS_URL:-http://127.0.0.1:7350}",
                "TANDEM_PROVIDER_TOKEN_FILE": "/run/secrets/provider-token",
                "TANDEM_PROVIDER_INTERVAL": "${TANDEM_PROVIDER_INTERVAL:-3}",
                "TANDEM_PROVIDER_BATCH": "${TANDEM_PROVIDER_BATCH:-1}",
            },
            "volumes": [
                {"type": "volume", "source": f"{name}-state", "target": "/state"},
                {"type": "bind", "source": f"${{TANDEM_PROVIDER_CREDENTIALS_DIR:?Run tandem providers-setup}}/{name}.token",
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
