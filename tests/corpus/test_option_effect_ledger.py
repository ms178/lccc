#!/usr/bin/env python3
"""Regression tests for the executable GP-22 option-effect ledger.

The ledger in `src/driver/option_table.rs` is a *claim* about the compiler;
`scripts/option_effect_ledger.py` turns it into a measurement.  These tests
pin both halves: the static invariants (so the parser and registry cannot
rot), the negative controls (so each check has teeth), and, when the built
compiler is present, the dynamic agreement between claim and observable.

No test here asserts a version string or a spelling that the gate does not
itself execute: a decorative assertion would be exactly the failure the
ledger was created to prevent.
"""

from __future__ import annotations

import hashlib
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))

from scripts import option_effect_ledger as ledger  # noqa: E402

LCCC = REPO / "target/fastbuild/lccc"


class LedgerParsing(unittest.TestCase):
    def test_every_declarative_row_is_parsed(self) -> None:
        rows = ledger.parse_rows(ledger.LEDGER.read_text())
        self.assertEqual(len(rows), 17, "a new row must be accounted for in the registry")
        self.assertTrue(all(r.spelling.startswith("-") for r in rows))

    def test_generated_family_rows_are_not_declarative_entries(self) -> None:
        rows = ledger.parse_rows(ledger.LEDGER.read_text())
        self.assertNotIn("option family", [r.spelling for r in rows])

    def test_inline_generated_contract_is_not_parsed_as_a_row(self) -> None:
        synthetic = (
            '    Contract { spelling: "-declared", class: Class::Inert,\n'
            '        rationale: "x", probe_id: Some("probe-lto-no-artifact") },\n'
            '    Some(Contract { spelling: "option family", class: Class::Refused,\n'
            '        rationale: "generated", probe_id: None });\n'
        )
        # The first is inline too, so neither is an array element: the parser
        # must not invent ledger rows out of generated code paths.
        self.assertEqual(ledger.parse_rows(synthetic), [])

    def test_comments_between_fields_do_not_break_parsing(self) -> None:
        synthetic = (
            "    Contract {\n"
            '        spelling: "-fexample",\n'
            "        class: Class::Implemented,\n"
            "        // a measured provenance comment between fields\n"
            '        rationale: "effect",\n'
            '        probe_id: Some("probe-fopenmp-serial"),\n'
            "    },\n"
        )
        rows = ledger.parse_rows(synthetic)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0].cls, "Implemented")
        self.assertEqual(rows[0].probe, "probe-fopenmp-serial")


class StaticInvariants(unittest.TestCase):
    def setUp(self) -> None:
        self.rows = ledger.parse_rows(ledger.LEDGER.read_text())
        self.cli = ledger.CLI.read_text()

    def test_live_tree_has_no_static_problems(self) -> None:
        self.assertEqual(ledger.static_problems(self.rows, self.cli), [])

    def test_every_accepted_row_has_a_registered_probe(self) -> None:
        for row in self.rows:
            if row.cls in ("Inert", "Implemented"):
                self.assertIsNotNone(row.probe, row.spelling)
                self.assertIn(row.probe, ledger.PROBES, row.spelling)
            else:
                self.assertIsNone(row.probe, row.spelling)

    def test_probe_registry_has_no_orphans_or_duplicates(self) -> None:
        probes = [r.probe for r in self.rows if r.probe]
        self.assertEqual(len(probes), len(set(probes)))
        self.assertEqual(set(probes), set(ledger.PROBES))

    def test_priority_set_is_fully_ledgered(self) -> None:
        spellings = {r.spelling for r in self.rows}
        self.assertTrue(set(ledger.PRIORITY_SET) <= spellings)

    def test_five_corrected_rows_are_classified_by_measurement(self) -> None:
        by_name = {r.spelling: r for r in self.rows}
        for refused in ("-ftest-coverage", "-fprofile-arcs", "-static-pie"):
            self.assertEqual(by_name[refused].cls, "Refused", refused)
        self.assertEqual(by_name["-Ofast"].cls, "Implemented")
        self.assertEqual(by_name["-Og"].cls, "Implemented")
        self.assertEqual(by_name["-Ofast"].probe, "probe-Ofast-fast-math")
        self.assertEqual(by_name["-Og"].probe, "probe-Og-debug-opt")

    def test_blanket_catch_all_absent(self) -> None:
        self.assertNotIn('arg if arg.starts_with("-f") => {}',
                         "\n".join(l.split("//", 1)[0] for l in self.cli.splitlines()))

    def test_each_static_rule_fires_on_a_mutation(self) -> None:
        """Independently of the gate's own --self-test, prove each rule."""
        source = ledger.LEDGER.read_text()
        mutations = {
            "duplicate": (source.replace('spelling: "-fno-lto"',
                                         'spelling: "-fsanitize=none"', 1), "duplicate"),
            "missing probe": (source.replace('probe_id: Some("probe-lto-no-artifact")',
                                             "probe_id: None", 1), "no probe_id"),
            "bad class": (source.replace("class: Class::Inert,\n        rationale: \"Objects contain native code",
                                          "class: Class::Bogus,\n        rationale: \"Objects contain native code", 1),
                          "invalid class"),
            "priority dropped": (source.replace('spelling: "-pedantic"',
                                                'spelling: "-pedantic-REMOVED"', 1),
                                 "priority option"),
            "unregistered probe": (source.replace('probe_id: Some("probe-lto-no-artifact")',
                                                  'probe_id: Some("probe-nope")', 1),
                                   "no implementation"),
        }
        for name, (mutated, needle) in mutations.items():
            with self.subTest(name=name):
                problems = ledger.static_problems(ledger.parse_rows(mutated), self.cli)
                self.assertTrue(any(needle in p for p in problems), problems)

    def test_self_test_has_no_failures(self) -> None:
        self.assertEqual(ledger.self_test(None), [])


class DynamicAgreement(unittest.TestCase):
    @unittest.skipUnless(LCCC.is_file(), "fastbuild binary not present")
    def test_ledger_agrees_with_the_built_compiler(self) -> None:
        rows = ledger.parse_rows(ledger.LEDGER.read_text())
        self.assertEqual(ledger.dynamic_problems(rows, str(LCCC)), [])

    @unittest.skipUnless(LCCC.is_file(), "fastbuild binary not present")
    def test_a_reclassified_row_is_caught_dynamically(self) -> None:
        """Negative control: claiming -Ofast is inert must be detected, even
        though every static invariant still holds."""
        source = ledger.LEDGER.read_text().replace(
            'spelling: "-Ofast",\n        class: Class::Implemented,',
            'spelling: "-Ofast",\n        class: Class::Inert,', 1,
        )
        rows = ledger.parse_rows(source)
        self.assertEqual(ledger.static_problems(rows, ledger.CLI.read_text()), [])
        self.assertTrue(ledger.dynamic_problems(rows, str(LCCC)))

    @unittest.skipUnless(LCCC.is_file(), "fastbuild binary not present")
    def test_gate_cli_passes_and_reports_json(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(REPO / "scripts/option_effect_ledger.py"),
             "--lccc", str(LCCC), "--self-test", "--json"],
            capture_output=True, text=True, timeout=180,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["problems"], [])
        self.assertTrue(report["dynamic"])
        self.assertEqual(report["rows"], 17)

    @unittest.skipUnless(LCCC.is_file(), "fastbuild binary not present")
    def test_offline_mode_never_needs_the_binary(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(REPO / "scripts/option_effect_ledger.py"),
             "--offline", "--json"],
            capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertFalse(json.loads(proc.stdout)["dynamic"])


class GateRobustness(unittest.TestCase):
    """A CI gate must fail with a diagnosis, never with a traceback, and must
    never hash output left behind by a previous invocation. These cases were
    found by reading the gate line by line; none is covered by the happy path.
    """

    def cli(self, *args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(REPO / "scripts/option_effect_ledger.py"),
             *args, "--json"],
            cwd=cwd or REPO, capture_output=True, text=True, timeout=180,
        )

    def test_a_missing_binary_is_a_named_problem_not_a_traceback(self) -> None:
        proc = self.cli("--lccc", "/definitely/not/a/compiler")
        self.assertEqual(proc.returncode, 1)
        self.assertNotIn("Traceback", proc.stderr)
        report = json.loads(proc.stdout)
        self.assertFalse(report["dynamic"])
        self.assertTrue(any("no such file" in p for p in report["problems"]))

    def test_a_nonexecutable_binary_is_a_named_problem(self) -> None:
        with tempfile.TemporaryDirectory(prefix="lccc-ledger-noexec-") as tmp:
            path = Path(tmp) / "lccc"
            path.write_text("not an executable\n")
            path.chmod(path.stat().st_mode & ~stat.S_IXUSR)
            proc = self.cli("--lccc", str(path))
        self.assertEqual(proc.returncode, 1)
        self.assertNotIn("Traceback", proc.stderr)
        self.assertTrue(any("not executable" in p
                            for p in json.loads(proc.stdout)["problems"]))

    @unittest.skipUnless(LCCC.is_file(), "fastbuild binary not present")
    def test_a_relative_binary_path_works_from_a_foreign_cwd(self) -> None:
        with tempfile.TemporaryDirectory(prefix="lccc-ledger-foreign-cwd-") as tmp:
            proc = self.cli("--lccc", "target/fastbuild/lccc", cwd=Path(tmp))
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue(json.loads(proc.stdout)["dynamic"])

    def test_stale_assembly_is_removed_before_hashing(self) -> None:
        """`opt.s` is reused for all 17 rows. If a compile fails without
        writing it, the previous row's bytes must not be hashed and reported
        as a codegen change."""
        with tempfile.TemporaryDirectory(prefix="lccc-ledger-stale-") as tmp:
            work = Path(tmp)
            src = work / "fixture.c"
            src.write_text("int f(void){return 1;}\n")
            dest = work / "opt.s"
            dest.write_text("STALE FROM A PREVIOUS ROW\n")
            stub = work / "compiler"
            stub.write_text("#!/bin/sh\nexit 7\n")
            stub.chmod(0o755)

            proc, digest = ledger.compile_asm(str(stub), work, src, [], "opt.s")
            self.assertNotEqual(proc.returncode, 0)
            self.assertEqual(digest, "", "a failed compile hashed stale bytes")
            self.assertFalse(dest.exists())

            # Positive control: a compiler that DOES write the output must be
            # hashed, so the empty digest above is not a vacuous assertion.
            writer = work / "writer"
            writer.write_text(
                "#!/bin/sh\n"
                'while [ $# -gt 0 ]; do\n'
                '  if [ "$1" = "-o" ]; then shift; printf "FRESH\\n" > "$1"; exit 0; fi\n'
                "  shift\n"
                "done\n"
                "exit 0\n",
            )
            writer.chmod(0o755)
            proc2, digest2 = ledger.compile_asm(str(writer), work, src, [], "opt.s")
            self.assertEqual(proc2.returncode, 0)
            self.assertEqual(digest2, hashlib.sha256(b"FRESH\n").hexdigest())

    def test_resolve_binary_accepts_the_repo_root_fallback(self) -> None:
        resolved, problem = ledger.resolve_binary("target/fastbuild/lccc")
        if problem:
            self.skipTest("fastbuild binary not present")
        self.assertTrue(Path(resolved).is_absolute())
        self.assertEqual(Path(resolved), (REPO / "target/fastbuild/lccc").resolve())

    def test_resolve_binary_accepts_a_bare_command_on_path(self) -> None:
        with tempfile.TemporaryDirectory(prefix="lccc-ledger-path-") as tmp:
            link = Path(tmp) / "lccc-on-path"
            link.write_text("#!/bin/sh\nexit 0\n")
            link.chmod(0o755)
            old_path = os.environ.get("PATH", "")
            os.environ["PATH"] = tmp + os.pathsep + old_path
            try:
                resolved, problem = ledger.resolve_binary("lccc-on-path")
            finally:
                os.environ["PATH"] = old_path
        self.assertIsNone(problem)
        self.assertEqual(Path(resolved), link.resolve())


class ArchitectureParity(unittest.TestCase):
    """The driver option table is architecture-independent, but the gate only
    ever runs against the primary driver. A per-arch CLI could diverge (a
    different `cli.rs` arm, a target-specific refusal) and stay invisible.

    Measured on the rebuilt fastbuild tree: lccc, lccc-i686, lccc-arm and
    lccc-riscv all report 17 rows and zero problems. Each is a host executable
    that emits a different target's assembly, so the same probes exercise
    four back ends. Skip only when the binaries have not been built.
    """

    DRIVERS = ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv")

    def test_every_supported_arch_driver_agrees_with_the_ledger(self) -> None:
        present = [REPO / "target/fastbuild" / name for name in self.DRIVERS]
        if not all(path.is_file() for path in present):
            self.skipTest("one or more fastbuild arch drivers were not built")
        rows = ledger.parse_rows(ledger.LEDGER.read_text())
        for path in present:
            with self.subTest(driver=path.name):
                problems = ledger.dynamic_problems(rows, str(path))
                self.assertEqual(problems, [], f"{path.name}: {problems}")

    def test_the_arch_driver_set_is_pinned(self) -> None:
        """A new target must be added to the parity sweep deliberately."""
        self.assertEqual(self.DRIVERS, ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv"))


if __name__ == "__main__":
    unittest.main()
