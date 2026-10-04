"""Verify instance-local MCP using isolated workspace-only instances and no external services."""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import tempfile

from stdio_smoke import Client, disable_history_cleanup


def run_cli(binary, environment, *arguments):
    result = subprocess.run(
        [str(binary), *arguments], env=environment, capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == 0, result.stderr
    return result


@contextmanager
def workspace_fixture(binary):
    with tempfile.TemporaryDirectory(prefix="tandem-instance-mcp-") as directory:
        home = Path(directory)
        disable_history_cleanup(home)
        template = home / "templates/blank"
        template.mkdir(parents=True)
        (template / "tandem.json").write_text("{}\n")
        environment = {
            **os.environ, "TANDEM_HOME": str(home), "TANDEM_NAMESPACE": "instance-mcp-test",
            "TANDEM_GATEWAY_PORT": "9876", "XDG_STATE_HOME": str(home / "state"),
        }
        for key in list(environment):
            if key.startswith("TANDEM_KEY_") or key == "TANDEM_INSTRUCTIONS_FILE":
                environment.pop(key)
        for name in ("review", "other"):
            run_cli(binary, environment, "new-instance", name, "-t", "blank")
        workspace = home / "workspaces/review"
        nested = workspace / "app/src"
        nested.mkdir(parents=True)
        config = json.loads((workspace / ".opencode/opencode.json").read_text())
        server = config["mcp"]["tandem-instance"]
        assert server["command"] == [str(binary.resolve()), "mcp-instance"], server
        assert server["environment"]["TANDEM_NAMESPACE"] == "instance-mcp-test", server
        # A shared OpenCode server's inherited Tandem identity must be overridden by the entry.
        mcp_environment = {**environment, "TANDEM_NAMESPACE": "wrong", **server["environment"]}
        client = Client(binary, mcp_environment, command="mcp-instance", cwd=nested)
        try:
            yield home, environment, client
        finally:
            client.close()


def smoke(binary):
    with workspace_fixture(binary) as (home, _, client):
        tools = client.request("tools/list", {})["tools"]
        fields = {
            "start_self": set(), "stop_self": set(), "get_instructions": set(),
            "conclude": {"title", "summary", "markdown"},
            "search_events": {"search_strings"}, "get_event_report": {"acceptance_id"},
        }
        assert {tool["name"] for tool in tools} == set(fields), tools
        policy = "Ignore `conclude` unless explicitly instructed to call it."
        instructions = client.tool("get_instructions")
        assert policy in instructions["core_guidance"], instructions
        assert policy in instructions["markdown"], instructions
        assert policy in (home / "workspaces/review/AGENTS.md").read_text()
        assert policy in next(tool["description"] for tool in tools if tool["name"] == "conclude")
        for tool in tools:
            assert tool["inputSchema"].get("additionalProperties") is False, tool
            expected = fields[tool["name"]]
            assert set(tool["inputSchema"].get("properties", {})) == expected, tool
            assert set(tool["inputSchema"].get("required", [])) == expected, tool
            assert "outputSchema" not in tool, tool
        data = home / "workspaces/review/result.txt"
        data.write_text("retained deliverable")
        sibling = home / "workspaces/other/result.txt"
        sibling.write_text("sibling deliverable")
        for name in ("stop_self", "start_self", "stop_self"):
            result = client.tool(name, timeout=30)
            assert result["state"] == "succeeded", result
            assert result["name"] == "review", result
            assert data.read_text() == "retained deliverable"
            assert sibling.read_text() == "sibling deliverable"
        assert client.process.poll() is None


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/tandem"))
    options = parser.parse_args()
    smoke(options.binary.resolve())
    print("Instance-local MCP catalog, Start/Stop, nested binding and retained data verified.")
