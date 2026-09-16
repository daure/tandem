import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from generate import generate
from recipes import REPOSITORIES
from templates import FIXTURES


def git(repo, *arguments):
    return subprocess.check_output(["git", "-C", str(repo), *arguments], text=True).strip()


class GeneratorTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="tandem-fixtures-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / "projects with spaces"

    def generate(self, root=None, seeded=True):
        with contextlib.redirect_stdout(io.StringIO()):
            return generate(root or self.root, seed_commits=seeded)

    def test_seeded_repositories_are_independent_clean_and_reproducible(self):
        self.generate()
        other = self.generate(Path(self.directory.name) / "second")
        for name in REPOSITORIES:
            repo = self.root / name
            self.assertEqual(git(repo, "rev-parse", "--show-toplevel"), str(repo))
            self.assertEqual(git(repo, "branch", "--show-current"), "main")
            self.assertEqual(git(repo, "rev-list", "--count", "HEAD"), "1")
            self.assertEqual(git(repo, "status", "--porcelain"), "")
            self.assertEqual(git(repo, "rev-parse", "HEAD"), git(other / name, "rev-parse", "HEAD"))
        self.assertFalse((self.root / "guestbook/frontend/.git").exists())
        self.assertEqual(set(FIXTURES), set(json.loads((self.root / "fixtures.json").read_text())["templates"]))

    def test_regeneration_refuses_to_touch_repositories_and_workspaces(self):
        self.generate()
        source = self.root / "guestbook/frontend/public/index.html"
        source.write_text("My uncommitted work")
        workspace = self.root / ".tandem/workspaces/review/notes.txt"
        workspace.parent.mkdir(parents=True)
        workspace.write_text("Instance edits")
        with self.assertRaisesRegex(ValueError, "Refusing to overwrite"):
            self.generate()
        self.assertEqual(source.read_text(), "My uncommitted work")
        self.assertEqual(workspace.read_text(), "Instance edits")

    def test_preflight_rejects_a_dangling_symlink_before_creating_anything(self):
        self.root.mkdir()
        (self.root / "postcard").symlink_to(self.root / "missing")
        with self.assertRaisesRegex(ValueError, "Refusing to overwrite"):
            self.generate()
        self.assertFalse((self.root / ".tandem").exists())
        self.assertFalse((self.root / "guestbook").exists())

    def test_commits_require_explicit_opt_in(self):
        self.generate(seeded=False)
        for name in REPOSITORIES:
            result = subprocess.run(["git", "-C", str(self.root / name), "rev-parse", "--verify", "HEAD"],
                                    capture_output=True)
            self.assertNotEqual(result.returncode, 0)

    def test_repo_sync_clones_the_requested_branch_for_every_repository(self):
        self.generate()
        for name in ("greetings-api", "greetings-ui"):
            git(self.root / name, "branch", "review")
        workspace = self.root / ".tandem/workspaces/review"
        workspace.mkdir(parents=True)
        script = self.root / ".tandem/templates/greetings/clone.sh"

        subprocess.run(["sh", str(script), "greetings-api", "greetings-ui"], check=True,
                       env={**os.environ, "SOURCE_ROOT": str(self.root), "TANDEM_BRANCH": "review",
                            "TANDEM_WORKSPACE_ROOT": str(workspace)},
                       cwd=workspace)

        for name in ("greetings-api", "greetings-ui"):
            self.assertEqual(git(workspace / name, "branch", "--show-current"), "review")

    def test_repo_sync_uses_the_source_repository_default_branch_without_a_branch_setting(self):
        self.generate()
        source = self.root / "guestbook"
        git(source, "switch", "-c", "source-default")
        workspace = self.root / ".tandem/workspaces/default"
        workspace.mkdir(parents=True)
        script = self.root / ".tandem/templates/guestbook/clone.sh"

        subprocess.run(["sh", str(script), "guestbook"], check=True,
                       env={**os.environ, "SOURCE_ROOT": str(self.root), "TANDEM_BRANCH": "",
                            "TANDEM_WORKSPACE_ROOT": str(workspace)}, cwd=workspace)

        self.assertEqual(git(workspace / "guestbook", "branch", "--show-current"), "source-default")

    def test_repo_sync_creates_missing_branches_from_each_repository_default(self):
        self.generate()
        names = ("greetings-api", "greetings-ui")
        git(self.root / names[0], "branch", "review")
        for name in names:
            git(self.root / name, "branch", "-m", f"{name}-default")
        workspace = self.root / ".tandem/workspaces/review"
        workspace.mkdir(parents=True)
        script = self.root / ".tandem/templates/greetings/clone.sh"

        subprocess.run(["sh", str(script), *names], check=True,
                       env={**os.environ, "SOURCE_ROOT": str(self.root), "TANDEM_BRANCH": "review",
                            "TANDEM_WORKSPACE_ROOT": str(workspace)}, cwd=workspace)

        for name in names:
            self.assertEqual(git(workspace / name, "branch", "--show-current"), "review")
            self.assertEqual(git(workspace / name, "rev-parse", "HEAD"),
                             git(self.root / name, "rev-parse", "HEAD"))
        self.assertEqual(git(workspace / names[0], "rev-parse", "--abbrev-ref", "@{upstream}"),
                         "origin/review")
        self.assertEqual(git(workspace / names[1], "for-each-ref", "--format=%(upstream)",
                             "refs/heads/review"), "")
        self.assertEqual(git(self.root / names[1], "branch", "--list", "review"), "")

    def test_repo_sync_preserves_existing_branch_and_edits_on_retry(self):
        self.generate()
        workspace = self.root / ".tandem/workspaces/review"
        workspace.mkdir(parents=True)
        script = self.root / ".tandem/templates/guestbook/clone.sh"
        environment = {**os.environ, "SOURCE_ROOT": str(self.root), "TANDEM_BRANCH": "review",
                       "TANDEM_WORKSPACE_ROOT": str(workspace)}
        subprocess.run(["sh", str(script), "guestbook"], check=True, env=environment, cwd=workspace)
        checkout = workspace / "guestbook"
        git(checkout, "switch", "-c", "my-work")
        edited = checkout / "frontend/public/index.html"
        edited.write_text("staged edits")
        git(checkout, "add", "frontend/public/index.html")
        edited.write_text("working edits")
        (checkout / "notes.txt").write_text("untracked notes")
        before = git(checkout, "status", "--porcelain")

        subprocess.run(["sh", str(script), "guestbook"], check=True, env=environment, cwd=workspace)

        self.assertEqual(git(checkout, "branch", "--show-current"), "my-work")
        self.assertEqual(git(checkout, "status", "--porcelain"), before)
        self.assertEqual(edited.read_text(), "working edits")
        self.assertEqual(git(checkout, "show", ":frontend/public/index.html"), "staged edits")
        self.assertEqual((checkout / "notes.txt").read_text(), "untracked notes")

    def test_git_overrides_are_rejected_before_the_output_is_created(self):
        script = Path(__file__).resolve().parents[1] / "generate.py"
        result = subprocess.run([sys.executable, str(script), "--output", str(self.root), "--seed-commits"],
                                env={**os.environ, "GIT_DIR": str(Path(self.directory.name) / "foreign.git")},
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("Unset Git repository override variables", result.stderr)
        self.assertFalse(self.root.exists())

    @unittest.skipUnless(shutil.which("docker"), "Docker Compose is required for model validation")
    def test_compose_models_resolve_to_isolated_workspaces_and_owned_infrastructure(self):
        self.generate()
        environment = {**os.environ, "TANDEM_WORKSPACE": str(self.root / ".tandem/workspaces/check"),
                       "TANDEM_UID": str(os.getuid()), "TANDEM_GID": str(os.getgid())}
        for name, fixture in FIXTURES.items():
            template = self.root / ".tandem/templates" / name
            result = subprocess.run(["docker", "compose", "-f", str(template / "compose.yaml"),
                                     "config", "--format", "json"], env=environment,
                                    check=True, capture_output=True, text=True)
            self.assertEqual(result.stderr, "")
            model = json.loads(result.stdout)
            manifest = json.loads((template / "tandem.json").read_text())
            self.assertLessEqual(set(manifest["routes"]) | set(manifest["one_shots"]), set(model["services"]))
            for service in model["services"].values():
                self.assertNotIn("ports", service)
                self.assertNotIn("container_name", service)
            for role in ("web", "api"):
                if role in fixture:
                    service = model["services"][role]
                    self.assertEqual(service["volumes"][0]["source"], environment["TANDEM_WORKSPACE"])
                    self.assertEqual(service["depends_on"]["repo-sync"]["condition"], "service_completed_successfully")
            if name == "greetings":
                self.assertIn("db", model["services"])
                self.assertEqual(model["services"]["migrate"]["depends_on"]["db"]["condition"], "service_healthy")
            if name == "mailroom":
                self.assertIn("redis", model["services"])
                self.assertIn("worker", model["services"])
            self.assertEqual(model["services"]["repo-sync"]["environment"]["TANDEM_BRANCH"], "")
        for name in REPOSITORIES:
            result = subprocess.run(["docker", "compose", "-f", str(self.root / name / "compose.yaml"),
                                     "config", "--quiet"], check=True, capture_output=True, text=True)
            self.assertEqual(result.stderr, "")


if __name__ == "__main__":
    unittest.main()
