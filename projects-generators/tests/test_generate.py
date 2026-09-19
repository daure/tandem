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
from generate import MINIMAL_FIXTURES, add_minimal, generate
from recipes import ASSETS, REPOSITORIES
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
        for name in FIXTURES:
            self.assertEqual(
                (self.root / ".tandem/templates" / name / "tandem-agents.md").read_text(encoding="utf-8"),
                (ASSETS / "guidance" / f"{name}.md").read_text(encoding="utf-8"))

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
            if name in ("repo-only", "guidance-only"):
                continue
            result = subprocess.run(["docker", "compose", "-f", str(template / "compose.yaml"),
                                     "config", "--format", "json"], env=environment,
                                    check=True, capture_output=True, text=True)
            self.assertEqual(result.stderr, "")
            model = json.loads(result.stdout)
            manifest = json.loads((template / "tandem.json").read_text()) if (template / "tandem.json").exists() else {}
            self.assertEqual(manifest.get("repositories", []),
                             [{"source": str(self.root / repo), "target": repo} for repo in fixture["repos"]])
            self.assertLessEqual(set(manifest.get("routes", {})) | set(manifest.get("one_shots", [])), set(model["services"]))
            for service in model["services"].values():
                self.assertNotIn("ports", service)
                self.assertNotIn("container_name", service)
            for role in ("web", "api"):
                if role in fixture:
                    service = model["services"][role]
                    self.assertEqual(service["volumes"][0]["source"], environment["TANDEM_WORKSPACE"])
            if name == "greetings":
                self.assertIn("db", model["services"])
                self.assertEqual(model["services"]["migrate"]["depends_on"]["db"]["condition"], "service_healthy")
            if name == "mailroom":
                self.assertIn("redis", model["services"])
                self.assertIn("worker", model["services"])
        for name in REPOSITORIES:
            if name == "repo-only":
                continue
            result = subprocess.run(["docker", "compose", "-f", str(self.root / name / "compose.yaml"),
                                     "config", "--quiet"], check=True, capture_output=True, text=True)
            self.assertEqual(result.stderr, "")

    def test_minimal_templates_cover_repository_and_service_only_workflows(self):
        self.generate()
        templates = self.root / ".tandem/templates"
        repo = templates / "repo-only"
        self.assertFalse((repo / "compose.yaml").exists())
        manifest = json.loads((repo / "tandem.json").read_text())
        self.assertEqual(manifest["repositories"], [{"source": str(self.root / "repo-only"), "target": "app"}])
        self.assertFalse((templates / "compose-only/tandem.json").exists())
        model = json.loads((templates / "compose-only/compose.yaml").read_text())
        self.assertEqual(set(model["services"]), {"redis"})
        self.assertEqual(model["services"]["redis"]["healthcheck"]["test"], ["CMD", "redis-cli", "ping"])
        guidance = templates / "guidance-only"
        self.assertEqual({path.name for path in guidance.iterdir()}, {"tandem-agents.md"})
        self.assertIn("## Scratch workspace", (guidance / "tandem-agents.md").read_text())
        subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests"],
                       cwd=self.root / "repo-only", check=True, capture_output=True)

    def test_minimal_addition_preserves_existing_fixtures_and_rejects_overwrites(self):
        self.generate()
        for path in [self.root / "repo-only", *(self.root / ".tandem/templates" / name for name in MINIMAL_FIXTURES)]:
            shutil.rmtree(path)
        source = self.root / "guestbook/README.md"
        source.write_text("User work")
        environment = (self.root / "env.sh").read_text()
        with contextlib.redirect_stdout(io.StringIO()):
            add_minimal(self.root, seed_commits=True)
        self.assertEqual(source.read_text(), "User work")
        self.assertEqual((self.root / "env.sh").read_text(), environment)
        self.assertEqual(git(self.root / "repo-only", "rev-list", "--count", "HEAD"), "1")
        self.assertEqual(set(json.loads((self.root / "fixtures.json").read_text())["templates"]), set(FIXTURES))
        with self.assertRaisesRegex(ValueError, "Refusing to overwrite"):
            add_minimal(self.root, seed_commits=True)

    def test_guidance_addition_preserves_existing_repositories_and_seed_status(self):
        self.generate()
        directory = self.root / ".tandem/templates/guidance-only"
        shutil.rmtree(directory)
        before = json.loads((self.root / "fixtures.json").read_text())
        head = git(self.root / "repo-only", "rev-parse", "HEAD")
        add_minimal(self.root, names=("guidance-only",))
        self.assertTrue((directory / "tandem-agents.md").is_file())
        self.assertEqual(json.loads((self.root / "fixtures.json").read_text()), before)
        self.assertEqual(git(self.root / "repo-only", "rev-parse", "HEAD"), head)


if __name__ == "__main__":
    unittest.main()
