#!/usr/bin/env python3
"""Harness regression tests. No compiler build or target execution is needed."""
import hashlib
import json
import random
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from mutate_frontend import mutate

RUNNER = Path(__file__).with_name("mutate_frontend.py").resolve()
FAKE = '''import hashlib, os, signal, sys, time
from pathlib import Path
src = Path(sys.argv[4]).read_bytes()
if "--reject" in sys.argv or "-S" not in sys.argv:
    sys.exit(1)
if src.startswith(b"int lccc_mutation_probe"):
    sys.exit(0)
if "--race" in sys.argv:
    time.sleep(0.02)
    if src != Path(sys.argv[4]).read_bytes():
        sys.exit(0)
    print("ccc: internal error: " + hashlib.sha256(src).hexdigest(), file=sys.stderr)
    sys.exit(1)
if src.startswith(b"timeout"):
    time.sleep(10)
if src == b"signal":
    os.kill(os.getpid(), signal.SIGTERM)
if src == b"ice":
    print("ccc: internal error: invariant", file=sys.stderr)
    sys.exit(1)
if src == b"panic":
    print("thread 'main' panicked at src/main.rs:12:1:", file=sys.stderr)
    sys.exit(1)
if src == b"diagnostic":
    print("  1 | internal error and panicked at", file=sys.stderr)
    sys.exit(1)
'''


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="lccc-test-")
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)
        self.fake = self.root / "fake"
        self.fake.write_text(f"#!{sys.executable}\n{FAKE}")
        self.fake.chmod(0o755)
        self.out = self.root / "output"

    def invoke(self, *args, compiler=None):
        return subprocess.run([sys.executable, str(RUNNER), "--lccc",
                               str(compiler or self.fake), "--out", str(self.out),
                               "--json", *map(str, args)], capture_output=True,
                              text=True, timeout=15, check=False)

    def input(self, name):
        path = self.root / f"{name}.c"
        path.write_bytes(name.encode())
        return path

    def test_classification_retention_and_replay(self):
        paths = [self.input(name) for name in (
            "ok", "diagnostic", "ice", "panic", "signal",
            "timeout0", "timeout1", "timeout2", "timeout3")]
        proc = self.invoke("--timeout", .4, "--replay", *paths, paths[-4])
        self.assertEqual(proc.returncode, 1, proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["stats"], dict(total=10, ok=1, diagnostic=1,
                                              ice=2, signal=1, timeout=5))
        timeout, = [b for b in report["buckets"] if b["kind"] == "timeout"]
        self.assertTrue(timeout["coarse"])
        self.assertEqual(timeout["count"], 5)
        self.assertEqual(len(timeout["samples"]), 3)
        for sample in timeout["samples"]:
            self.assertEqual(Path(sample["path"]).read_bytes(), Path(sample["origin"]).read_bytes())
        replay = subprocess.run(timeout["samples"][0]["replay_argv"], cwd=report["cwd"],
                                capture_output=True, text=True, timeout=5, check=False)
        self.assertEqual(replay.returncode, 1, replay.stderr)
        self.assertIn('"timeout": 1', replay.stdout)

    def test_invalid_invocations(self):
        src = self.input("ok")
        for args in [("--iterations", 0), ("--iterations", -1),
                     ("--timeout", "nan"), ("--timeout", "inf"), ("--timeout", 0),
                     ("--seeds", self.root / "missing*.c"),
                     ("--replay", src, "--", "--reject")]:
            with self.subTest(args=args):
                proc = self.invoke(*args)
                self.assertEqual(proc.returncode, 2, proc.stderr)
                self.assertTrue(proc.stderr)
        self.assertEqual(self.invoke("--replay", src, compiler=self.root / "missing").returncode, 2)

    def test_clean_run(self):
        proc = self.invoke("--replay", self.input("diagnostic"))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(json.loads(proc.stdout)["stats"]["diagnostic"], 1)

    def test_repeatable_and_concurrent(self):
        seed = self.input("test seed for mutation")
        args = ("--seeds", seed, "--iterations", 8, "--", "--race")
        with ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda _: self.invoke(*args), range(2)))
        for result in results:
            self.assertEqual(result.returncode, 1, result.stderr)
        reports = [json.loads(result.stdout) for result in results]
        self.assertEqual(*reports)
        self.assertEqual(reports[0]["stats"]["ice"], 8)
        for bucket in reports[0]["buckets"]:
            for sample in bucket["samples"]:
                self.assertEqual(sample["sha256"], bucket["signature"])
                self.assertEqual(hashlib.sha256(Path(sample["path"]).read_bytes()).hexdigest(),
                                 sample["sha256"])
        self.assertFalse(list(self.out.glob("run-*")))

    def test_byte_mutator_is_deterministic(self):
        data = b"int x;\x00\xff"
        self.assertEqual(mutate(b"", random.Random(7)), b"")
        first = random.Random(7)
        second = random.Random(7)
        a = [mutate(data, first) for _ in range(50)]
        self.assertEqual(a, [mutate(data, second) for _ in range(50)])
        self.assertGreater(len(set(a)), 25)


if __name__ == "__main__":
    unittest.main()
