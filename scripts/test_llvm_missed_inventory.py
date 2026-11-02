#!/usr/bin/env python3
"""Offline adversarial contracts: no credentials/network needed."""

import csv
import io
import json
import hashlib
from pathlib import Path
import tempfile
from contextlib import redirect_stdout
import unittest
from unittest.mock import Mock, patch
from llvm_missed_inventory import (
    comments,
    enumerate_items,
    inventory_csv,
    classify,
    main,
)


def item(n, **extra):
    return dict(
        number=n,
        title=f"test {n}",
        state="closed",
        html_url=f"https://github.com/llvm/llvm-project/issues/{n}",
        updated_at="2026-10-20T00:00:00Z",
        labels=[{"name": "missed-optimization"}],
        comments=0,
        **extra,
    )


class InventoryTests(unittest.TestCase):
    def test_paginates_past_search_limit_and_keeps_prs(self):
        rows = [item(n) for n in range(1, 1202)]
        rows[-1]["pull_request"] = {"url": "pr"}
        get = Mock(side_effect=[rows[n : n + 100] for n in range(0, len(rows), 100)])
        found = enumerate_items(get)
        self.assertEqual(len(found), 1201)
        self.assertIn("page=13", get.call_args.args[0])
        csv_rows = list(csv.DictReader(io.StringIO(inventory_csv(found.values()))))
        self.assertEqual(csv_rows[-1]["kind"], "pr")
        self.assertEqual(csv_rows[0]["review_status"], "unreviewed")

    def test_exact_page_multiple_fetches_terminating_page(self):
        get = Mock(side_effect=[[item(n) for n in range(100)], []])
        self.assertEqual(len(enumerate_items(get)), 100)
        self.assertEqual(get.call_count, 2)

    def test_empty_duplicate_wrong_label_and_error_fail_closed(self):
        for replies in (
            [[]],
            [[item(1), item(1)]],
            [[dict(item(1), labels=[])]],
            [{"message": "bad"}],
        ):
            with self.subTest(replies=replies), self.assertRaises(ValueError):
                enumerate_items(Mock(side_effect=replies))

    def test_csv_preserves_quoting_and_never_claims_verified(self):
        x = item(1)
        x["title"] = '  =danger("x"),\nnext'
        row = next(csv.DictReader(io.StringIO(inventory_csv([x]))))
        self.assertEqual(row["title"], "'" + x["title"])
        self.assertEqual(row["review_status"], "unreviewed")
        reviewed = next(
            csv.DictReader(io.StringIO(inventory_csv([x], {1: "covered-limited"})))
        )
        self.assertEqual(reviewed["review_status"], "covered-limited")

    def test_comments_paginate_and_reject_incomplete_or_duplicate(self):
        x = item(1)
        x["comments"] = 101
        result = comments(
            Mock(side_effect=[[{"id": n} for n in range(100)], [{"id": 100}]]), x
        )
        self.assertEqual(len(result["comments"]), 101)
        gap = comments(Mock(return_value=[{"id": 1}]), x)
        self.assertEqual(gap["counter_gap"], 100)
        with self.assertRaises(ValueError):
            comments(Mock(return_value=[{"id": 1}, {"id": 1}]), x)

    def run_resume(self, folder, cases, corrupt=None):
        root = Path(folder)
        posts = json.dumps(item(1)) + "\n"
        conversations = (
            json.dumps({"number": 1, "comments": [], "counter_gap": 0}) + "\n"
        )
        manifest = {
            "posts_sha256": hashlib.sha256(posts.encode()).hexdigest(),
            "comments_sha256": hashlib.sha256(conversations.encode()).hexdigest(),
        }
        (root / "posts.jsonl").write_text(posts + (" " if corrupt == "posts" else ""))
        (root / "comments.jsonl").write_text(
            conversations + (" " if corrupt == "comments" else "")
        )
        (root / "manifest.json").write_text(json.dumps(manifest))
        (root / "ledger.json").write_text(json.dumps({"cases": cases}))
        argv = [
            "inventory",
            "--output",
            str(root),
            "--csv",
            str(root / "inventory.csv"),
            "--ledger",
            str(root / "ledger.json"),
            "--comments",
            "--resume",
        ]
        with patch("sys.argv", argv), patch(
            "llvm_missed_inventory.subprocess.run"
        ) as network, redirect_stdout(io.StringIO()):
            main()
            network.assert_not_called()
        return json.loads((root / "manifest.json").read_text())

    def test_hash_checked_resume_derives_coverage_without_network(self):
        with tempfile.TemporaryDirectory() as folder:
            manifest = self.run_resume(
                folder, [{"number": 1, "status": "covered-limited"}]
            )
            self.assertEqual(manifest["status"], "complete")
            self.assertEqual(manifest["requests_this_invocation"], 0)
            self.assertEqual(
                manifest["csv_sha256"],
                hashlib.sha256(
                    (Path(folder) / "inventory.csv").read_bytes()
                ).hexdigest(),
            )
            self.assertIn(
                "covered-limited", (Path(folder) / "inventory.csv").read_text()
            )

    def test_resume_rejects_tampered_snapshots(self):
        for corrupt in ("posts", "comments"):
            with self.subTest(
                corrupt=corrupt
            ), tempfile.TemporaryDirectory() as folder, self.assertRaisesRegex(
                ValueError, "hash mismatch"
            ):
                self.run_resume(folder, [], corrupt)

    def test_ledger_rejects_unknown_duplicate_and_overclaimed_status(self):
        for cases in (
            [{"number": 9, "status": "unreviewed"}],
            [{"number": 1, "status": "unreviewed"}] * 2,
            [{"number": 1, "status": "optimal"}],
        ):
            with self.subTest(
                cases=cases
            ), tempfile.TemporaryDirectory() as folder, self.assertRaises(ValueError):
                self.run_resume(folder, cases)

    def test_classifier_is_routing_not_validation(self):
        x = item(1)
        self.assertEqual(classify(x)[0], "unclassified-review")
        x["labels"] += [{"name": "lld:ELF"}]
        self.assertEqual(classify(x)[0], "linker-review")
        x["labels"] = [{"name": "backend:X86"}]
        self.assertEqual(classify(x)[0], "supported-target-review")
        x["labels"] = [{"name": "backend:AMDGPU"}]
        self.assertEqual(classify(x)[0], "unsupported-target-review")


if __name__ == "__main__":
    unittest.main()
