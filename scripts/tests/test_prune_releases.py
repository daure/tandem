from datetime import datetime, timedelta, timezone
import importlib.util
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("prune_releases", Path(__file__).resolve().parents[1] / "prune-releases.py")
retention = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retention)


def release(number, **overrides):
    return {
        "id": number,
        "tag_name": f"v1.0.{number}",
        "draft": False,
        "prerelease": False,
        "published_at": (datetime(2026, 1, 1, tzinfo=timezone.utc) + timedelta(days=number)).isoformat(),
        **overrides,
    }


class RetentionTests(unittest.TestCase):
    def setUp(self):
        silence = patch("builtins.print")
        silence.start()
        self.addCleanup(silence.stop)

    def test_keeps_newest_thirty_by_publication_time_and_ignores_unpublished_releases(self):
        values = [release(number) for number in range(31, 0, -1)]
        values.extend([release(100, draft=True, published_at=None), release(101, prerelease=True)])
        self.assertEqual([item["id"] for item in retention.select_deletions(values)], [1])
        self.assertEqual(retention.select_deletions(values[:30]), [])

    def test_dry_run_fetches_all_pages_without_mutations(self):
        pages = [[release(n) for n in range(131, 31, -1)], [release(n) for n in range(31, 0, -1)]]
        with patch.object(retention.subprocess, "run", side_effect=[subprocess.CompletedProcess([], 0, json.dumps(pages)), subprocess.CompletedProcess([], 0, json.dumps(release(131)))]) as run:
            self.assertEqual(retention.prune(), 101)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0].args[0], ["gh", "api", "--hostname", "github.com", "repos/daure/tandem/releases?per_page=100", "--paginate", "--slurp"])
        self.assertTrue(all("DELETE" not in call.args[0] for call in run.call_args_list))

    def test_apply_rechecks_candidates_and_deletes_only_release_ids(self):
        pages = [[release(n) for n in range(1, 32)]]
        with patch.object(retention, "gh_api", side_effect=[pages, release(31), release(1), None]) as api:
            self.assertEqual(retention.prune(published_tag="v1.0.31", apply=True), 1)
        self.assertEqual(api.call_args_list[-1].args, ("repos/daure/tandem/releases/1", "--method", "DELETE"))


if __name__ == "__main__":
    unittest.main()
