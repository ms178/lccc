#!/usr/bin/env python3
"""Executable red-team checks for PR826 fixtures, parsers, and preload oracle.

Host-compiled drivers validate the test oracle, not LCCC's generated runtime.
LCCC integration and the production Rust ULEB implementation have separate gates.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import runpy
import re
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
PGO = runpy.run_path(str(ROOT / "tests/regression/check_pgo_io.py"), run_name="audit_pgo")
RV = runpy.run_path(str(ROOT / "tests/linker/check_riscv_relocation_contract.py"), run_name="audit_rv")

# A reference runtime deliberately offers the original ordering bug as a mutant.
# The same interposer used by the integration gate must distinguish the two.
DRIVER = r'''
#include <stdio.h>
#include <stdlib.h>
#include <sys/file.h>
#include <unistd.h>
int main(void) {
    const char *mode = getenv("DRIVER_NO_CLOEXEC") ? "a" : "ae";
    FILE *f = fopen(getenv("LCCC_PROFILE_FILE"), mode);
    if (!f) { perror("lccc: PGO: cannot open profile"); return 23; }
    int fd = fileno(f);
    if (fd < 0 || flock(fd, LOCK_EX)) {
        perror("lccc: PGO: cannot lock profile");
        fclose(f); return 23;
    }
    for (int i = 0; i < 1000; ++i) fprintf(f, "record-%04d-abcdefgh\n", i);
    if (ferror(f)) {
        static const char msg[] = "lccc: PGO: profile write failed\n";
        ssize_t written = write(2, msg, sizeof(msg)-1);
        if (written != (ssize_t)(sizeof(msg)-1)) {
            if (fclose(f)) perror("lccc: PGO: profile close/flush failed");
            return 23;
        }
    }
    if (getenv("DRIVER_EARLY_UNLOCK")) flock(fd, LOCK_UN);
    if (fclose(f)) perror("lccc: PGO: profile close/flush failed");
    return 23;
}
'''


def compile_checked(command):
    result = subprocess.run(command, capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise AssertionError(f"compiler command {command} failed ({result.returncode}):\n"
                             + result.stdout + result.stderr)


_SANITIZER_FLAGS: dict[str, list[str]] = {}


def sanitizer_flags(cc: str) -> list[str]:
    """`-fsanitize=undefined -fno-sanitize-recover=all` when `cc` can link it.

    The pinned GCC 16.2 shipped by the tag cache is a compiler-only prefix: it
    carries no libubsan, so `-fsanitize=undefined` fails at *link* time with
    `cannot find -lubsan`.  That is a property of the distribution, not of the
    harness, and it says nothing about the claim this case proves -- the
    generated program prints the same value at every optimisation level.  The
    campaign therefore runs either way and says which mode it used, instead of
    dying on a runtime it does not control (which is how the lane went red with
    an assertion about `gcc` rather than about the property under test).
    """
    cached = _SANITIZER_FLAGS.get(cc)
    if cached is not None:
        return cached
    flags = ["-fsanitize=undefined", "-fno-sanitize-recover=all"]
    with tempfile.TemporaryDirectory() as tmp:
        probe = Path(tmp) / "probe.c"
        probe.write_text("int main(void) { return 0; }\n")
        result = subprocess.run([cc, *flags, str(probe), "-o", str(Path(tmp) / "probe")],
                                capture_output=True, text=True, timeout=60)
    if result.returncode:
        detail = (result.stderr.strip().splitlines() or ["no diagnostics"])[-1]
        print(f"pr826 redteam: {cc} cannot link -fsanitize=undefined ({detail}); "
              "running the opt-level campaign without UB instrumentation",
              file=sys.stderr)
        flags = []
    _SANITIZER_FLAGS[cc] = flags
    return flags


class RedTeam(unittest.TestCase):
    def test_profile_parser_resets_record_identity(self):
        text = "lccc-pgo-v1 1 2 3 4\nfunc f\nf 1\nlccc-pgo-v1 5 6 7 8\nf 1\n"
        with self.assertRaises(AssertionError):
            PGO["counters"](text)

    def test_profile_parser_rejects_blank_and_malformed_records(self):
        for record in ("\n", "x\n", "func f\nf 1\n", "lccc-pgo-v1 1 2 3 4\nfunc f\nf x\n"):
            with self.subTest(record=record), self.assertRaises((AssertionError, ValueError)):
                PGO["counters"](record)

    def test_failure_produces_structured_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "failure.json"
            missing = Path(tmp) / "missing-compiler"
            p = subprocess.run([sys.executable, str(ROOT / "tests/regression/check_pgo_io.py"),
                                "--lccc", str(missing), "--cc", str(missing), "--json", str(report)],
                               capture_output=True, text=True, timeout=10)
            self.assertEqual(p.returncode, 1)
            result = json.loads(report.read_text())
            self.assertEqual((result["passed"], result["failed"], result["skipped"]), (0, 1, 0))
            self.assertIn("missing-compiler", result["error"])

    def test_profile_counts_preserve_header_identity(self):
        one = "lccc-pgo-v1 1 2 3 4\nfunc f\nf 1\ne 0 1 1\n"
        counts = PGO["counters"](one * 2)
        self.assertEqual(set(counts.values()), {2})
        self.assertEqual(len(counts), 2)

    def test_assertion_disabled_invocations_fail_closed(self):
        for path, args in (("tests/regression/check_pgo_io.py", ["--lccc", "unused"]),
                           ("tests/linker/check_riscv_relocation_contract.py", ["--lccc", "unused"])):
            p = subprocess.run([sys.executable, "-O", str(ROOT / path), *args],
                               capture_output=True, text=True, timeout=10)
            self.assertEqual(p.returncode, 2)
            self.assertIn("disables test assertions", p.stderr)

    def test_elf_fixtures_have_unique_names_and_valid_tables(self):
        cases = list(RV["cases"]())
        self.assertEqual(len(cases), len({case[0] for case in cases}))
        with tempfile.TemporaryDirectory() as tmp:
            for name, options, _, _ in cases:
                with self.subTest(name=name):
                    path = Path(tmp) / "fixture.o"
                    path.write_bytes(RV["fixture"](**options))
                    secs = RV["sections"](path)
                    self.assertEqual(secs[".rela.data"][0][5] % 24, 0)
                    self.assertEqual(secs[".data.more"][1], b"\xa5" * 16)

    def test_relocation_extent_metadata_matches_dispatcher(self):
        source = (ROOT / "src/backend/riscv/linker/reloc.rs").read_text()
        table = source.split("fn relocation_extent", 1)[1].split("/// Validate", 1)[0]
        dispatcher = source.split("fn apply_one_reloc", 1)[1].split("/// Find the hi20", 1)[0]
        kinds = lambda text: set(re.findall(r"R_RISCV_[A-Z0-9_]+", text))
        self.assertEqual(kinds(table), kinds(dispatcher))
        self.assertEqual(len(kinds(table)), 38)  # Plus numeric NONE, checked separately.

    def test_generated_c_has_independent_output_at_all_opts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for count in (1, 96):
                src = root / "source.c"
                src.write_text(PGO["source"](count))
                for opt in ("-O0", "-O2", "-O3"):
                    exe = root / "source"
                    compile_checked([CC, opt, *sanitizer_flags(CC),
                                     str(src), "-o", str(exe)])
                    p = subprocess.run([str(exe)], capture_output=True, text=True, timeout=10)
                    self.assertEqual(p.returncode, 23)
                    self.assertEqual(p.stdout, f"{count - count * (count - 1) // 2}\n")
                    self.assertEqual(p.stderr, "")

    def test_preload_detects_original_unlock_mutant_and_faults(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "driver.c"
            src.write_text(DRIVER)
            exe, so = root / "driver", root / "oracle.so"
            compile_checked([CC, "-D_FORTIFY_SOURCE=3", "-O1", "-Wall", "-Wextra", "-Werror",
                             str(src), "-o", str(exe)])
            compile_checked([CC, "-shared", "-fPIC", "-O1", "-Wall", "-Wextra", "-Werror",
                             str(ROOT / "tests/regression/fixtures/pgo_io_interpose.c"),
                             "-ldl", "-o", str(so)])
            base = {k: v for k, v in os.environ.items()
                    if not k.startswith(("PGO_IO_", "LCCC_PROFILE_", "DRIVER_")) and k != "LD_PRELOAD"}
            env = dict(base, LD_PRELOAD=str(so))
            golden = root / "golden"
            p = subprocess.run([str(exe)], env=dict(env, LCCC_PROFILE_FILE=str(golden)),
                               capture_output=True, timeout=10)
            self.assertEqual((p.returncode, p.stderr), (23, b""))
            raw = golden.read_bytes()
            self.assertGreater(len(raw), 8192)
            p = subprocess.run([str(exe)], env=dict(env, LCCC_PROFILE_FILE=str(root / "inherited"),
                                                  DRIVER_NO_CLOEXEC="1"), capture_output=True, timeout=10)
            self.assertEqual(p.returncode, 99, "oracle failed to detect inheritable profile descriptor")
            for fault in ("fopen", "fileno", "flock", "eintr", "ferror", "ferror-zero", "ferror-stale", "fclose"):
                path = root / fault
                p = subprocess.run([str(exe)], env=dict(env, LCCC_PROFILE_FILE=str(path), PGO_IO_FAULT=fault),
                                   capture_output=True, text=True, timeout=10)
                self.assertEqual(p.returncode, 23)
                self.assertIn("lccc: PGO:", p.stderr)
                if fault in ("fopen", "fileno", "flock", "eintr"):
                    self.assertTrue(not path.exists() or path.stat().st_size == 0)
                if fault.startswith("ferror"):
                    self.assertEqual(p.stderr, "lccc: PGO: profile write failed\n")
            for mutant in (False, True):
                sync = root / f"sync-{mutant}"
                sync.mkdir()
                path = root / f"profile-{mutant}"
                pair_env = dict(env, LCCC_PROFILE_FILE=str(path), PGO_IO_SYNC=str(sync))
                if mutant:
                    pair_env["DRIVER_EARLY_UNLOCK"] = "1"
                a = subprocess.Popen([str(exe)], env=dict(pair_env, PGO_IO_ROLE="A"),
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                b = None
                try:
                    deadline = time.monotonic() + 10
                    while not (sync / "A-held").exists():
                        self.assertIsNone(a.poll(), "A exited before acquiring lock")
                        self.assertLess(time.monotonic(), deadline, "A lock acquisition timeout")
                        time.sleep(.001)
                    b = subprocess.Popen([str(exe)], env=dict(pair_env, PGO_IO_ROLE="B"),
                                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                    for process in (a, b):
                        stdout, stderr = process.communicate(timeout=15)
                        self.assertEqual((process.returncode, stdout, stderr), (23, b"", b""))
                finally:
                    for process in (a, b):
                        if process is None:
                            continue
                        if process.poll() is None:
                            process.kill()
                        process.wait()
                self.assertEqual((sync / "unlocked-close").exists(), mutant)
                self.assertEqual(path.read_bytes() == raw * 2, not mutant)
                self.assertTrue((sync / "B-attempting").exists())
                self.assertTrue((sync / "B-blocked").exists())


def main():
    global CC
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--cc", default="gcc")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    CC = args.cc
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RedTeam)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    for case, trace in result.failures + result.errors:
        for offset in range(0, len(trace), 3000):
            chunk = trace[offset:offset + 3000]
            escaped = chunk.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
            print(f"::error title=redteam {case.id()} chunk {offset}::" + escaped)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        version = subprocess.run([CC, "--version"], capture_output=True, text=True, timeout=10)
        args.json.write_text(json.dumps(dict(tests=result.testsRun, failures=len(result.failures),
                                            errors=len(result.errors), skipped=len(result.skipped),
                                            compiler=version.stdout.splitlines()[0]), indent=2) + "\n")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
