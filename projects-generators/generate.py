#!/usr/bin/env python3
"""Generate five local Git repositories and four isolated Tandem templates."""

import argparse
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import subprocess

from recipes import REPOSITORIES, create_repositories, write, write_json
from templates import FIXTURES, create_templates


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


def generate(root, seed_commits=False, port=9886):
    overrides = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY")
    if any(name in os.environ for name in overrides):
        raise ValueError("Unset Git repository override variables before generating fixtures")
    root = root.expanduser().resolve()
    if any(character in str(root) for character in "$\n\r"):
        raise ValueError("Output path cannot contain dollar signs or line breaks")
    if not 1 <= port <= 65535:
        raise ValueError("Gateway port must be between 1 and 65535")
    if not shutil.which("git"):
        raise ValueError("Git is required")
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[1] / "projects")
    parser.add_argument("--seed-commits", action="store_true", help="Create the initial commits needed for cloning")
    parser.add_argument("--gateway-port", type=int, default=9886)
    options = parser.parse_args()
    try:
        root = generate(options.output, options.seed_commits, options.gateway_port)
    except (ValueError, OSError, RuntimeError, subprocess.CalledProcessError) as error:
        detail = error.stderr if isinstance(error, subprocess.CalledProcessError) else str(error)
        parser.exit(1, f"Generation failed: {detail}\n")
    print(f"Created five repositories and four templates under {root}")
    if not options.seed_commits:
        print("Repositories have no commits. Commit their source before starting instances.")
    print(f"Load the fixture environment: source {shlex.quote(str(root / 'env.sh'))}")


if __name__ == "__main__":
    main()
