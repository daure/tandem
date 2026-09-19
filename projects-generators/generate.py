#!/usr/bin/env python3
"""Generate six local Git repositories and seven isolated Tandem templates."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess

from recipes import REPOSITORIES, create_repositories, local_repository, write, write_json
from templates import FIXTURES, create_templates

MINIMAL_FIXTURES = ("repo-only", "compose-only", "guidance-only")


def git(repo, *arguments, check=True, environment=None):
    return subprocess.run(["git", "-C", str(repo), *arguments], check=check,
                          env=environment, text=True, capture_output=True)


def initialize_git(repo, seed_commits):
    git(repo, "init", "--initial-branch=main")
    if not seed_commits:
        return
    print(git(repo, "status", "--short").stdout, end="")
    print(git(repo, "diff").stdout, end="")
    history = git(repo, "log", "--oneline", "-10", check=False)
    if history.returncode not in (0, 128):
        raise RuntimeError(history.stderr)
    files = sorted(str(path.relative_to(repo)) for path in repo.rglob("*")
                   if path.is_file() and ".git" not in path.relative_to(repo).parts)
    git(repo, "add", "--", *files)
    environment = {**os.environ, "GIT_AUTHOR_NAME": "Tandem Fixtures",
                   "GIT_AUTHOR_EMAIL": "fixtures@tandem.invalid",
                   "GIT_COMMITTER_NAME": "Tandem Fixtures",
                   "GIT_COMMITTER_EMAIL": "fixtures@tandem.invalid",
                   "GIT_AUTHOR_DATE": "2026-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2026-01-01T00:00:00Z"}
    git(repo, "commit", "-m", f"Seed {repo.name} development fixture", environment=environment)
    if git(repo, "status", "--porcelain").stdout:
        raise RuntimeError(f"Seed repository is not clean: {repo}")


def validate_environment(root, git_required=True):
    overrides = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY")
    if any(name in os.environ for name in overrides):
        raise ValueError("Unset Git repository override variables before generating fixtures")
    root = root.expanduser().resolve()
    if any(character in str(root) for character in "$\n\r"):
        raise ValueError("Output path cannot contain dollar signs or line breaks")
    if git_required and not shutil.which("git"):
        raise ValueError("Git is required")
    return root


def generate(root, seed_commits=False, port=9886):
    root = validate_environment(root)
    if not 1 <= port <= 65535:
        raise ValueError("Gateway port must be between 1 and 65535")
    targets = [root / name for name in (*REPOSITORIES, ".tandem", "env.sh", "fixtures.json")]
    occupied = [str(path) for path in targets if path.exists() or path.is_symlink()]
    if occupied:
        raise ValueError("Refusing to overwrite fixture paths: " + ", ".join(occupied))
    root.mkdir(parents=True, exist_ok=True)
    (root / ".tandem").mkdir()
    create_repositories(root)
    for name in REPOSITORIES:
        initialize_git(root / name, seed_commits)
    create_templates(root)
    suffix = hashlib.sha256(str(root).encode()).hexdigest()[:8]
    environment = {"TANDEM_HOME": str(root / ".tandem"), "TANDEM_NAMESPACE": f"tandem-fixtures-{suffix}",
                   "TANDEM_GATEWAY_PORT": str(port)}
    write(root / "env.sh", "# Source this file from your shell before starting Tandem.\n"
          + "".join(f"export {key}={shlex.quote(value)}\n" for key, value in environment.items()))
    write_json(root / "fixtures.json", {"environment": environment, "repositories": list(REPOSITORIES),
                                       "templates": list(FIXTURES), "seeded": seed_commits})
    return root


def add_minimal(root, seed_commits=False, names=MINIMAL_FIXTURES):
    with_repository = "repo-only" in names
    root = validate_environment(root, git_required=with_repository)
    metadata_path = root / "fixtures.json"
    if metadata_path.is_symlink() or not metadata_path.is_file():
        raise ValueError("Fixture inventory must be an existing regular file")
    inventory = json.loads(metadata_path.read_text(encoding="utf-8"))
    if (not isinstance(inventory.get("repositories"), list)
            or not isinstance(inventory.get("templates"), list)
            or not isinstance(inventory.get("seeded"), bool)):
        raise ValueError("Invalid fixture inventory")
    if Path(inventory["environment"]["TANDEM_HOME"]) != root / ".tandem":
        raise ValueError("Fixture inventory belongs to a different root")
    templates = root / ".tandem/templates"
    if not templates.is_dir() or templates.is_symlink() or (root / ".tandem").is_symlink():
        raise ValueError("Fixture templates must be an existing real directory")
    targets = [templates / name for name in names]
    if with_repository:
        targets.append(root / "repo-only")
    if any(path.exists() or path.is_symlink() for path in targets):
        raise ValueError("Refusing to overwrite minimal fixture paths")
    if with_repository:
        local_repository(root)
        initialize_git(root / "repo-only", seed_commits)
        inventory["repositories"] = list(dict.fromkeys([*inventory["repositories"], "repo-only"]))
        inventory["seeded"] = inventory["seeded"] and seed_commits
    create_templates(root, names)
    inventory["templates"] = list(dict.fromkeys([*inventory["templates"], *names]))
    write_json(metadata_path, inventory)
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[1] / "projects")
    parser.add_argument("--seed-commits", action="store_true", help="Create the initial commits needed for cloning")
    parser.add_argument("--gateway-port", type=int, default=9886)
    additions = parser.add_mutually_exclusive_group()
    additions.add_argument("--add-minimal", action="store_true", help="Append repo-only, compose-only and guidance-only fixtures; refuse occupied paths")
    additions.add_argument("--add-guidance", action="store_true", help="Append only guidance-only to an existing fixture root; requires no Git")
    options = parser.parse_args()
    try:
        if options.add_minimal:
            root = add_minimal(options.output, options.seed_commits)
        elif options.add_guidance:
            root = add_minimal(options.output, names=("guidance-only",))
        else:
            root = generate(options.output, options.seed_commits, options.gateway_port)
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.CalledProcessError) as error:
        detail = error.stderr if isinstance(error, subprocess.CalledProcessError) else str(error)
        parser.exit(1, f"Generation failed: {detail}\n")
    print(f"Added requested fixtures under {root}" if options.add_minimal or options.add_guidance else f"Created {len(REPOSITORIES)} repositories and {len(FIXTURES)} templates under {root}")
    if not options.seed_commits and not options.add_guidance:
        print("Repositories have no commits. Commit their source before starting instances.")
    print(f"Load the fixture environment: source {shlex.quote(str(root / 'env.sh'))}")


if __name__ == "__main__":
    main()
