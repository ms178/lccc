#!/usr/bin/env python3
"""Mutation tests: distinct asm-diff mode/corpus gates must stay hosted."""
from __future__ import annotations

from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
import tempfile
import unittest

import check_ci_gate_parity as parity


class AsmDiffParityTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()
        cls.hosted = "\n".join(
            parity.run_script_bodies(p)
            for p in sorted(parity.WORKFLOWS.glob("*.yml"))
        )
        # Retain the literal YAML run lines (including shell quoting) so
        # each negative mutation changes exactly one hosted invocation.
        cls.x64 = next(
            line.strip() for line in cls.hosted.splitlines()
            if line.strip().startswith("python3 scripts/asmdiff.py")
            and "merged-pr629-followup.casefile" in line
        )
        cls.i686 = next(
            line.strip() for line in cls.hosted.splitlines()
            if line.strip().startswith("python3 scripts/asmdiff.py") and "--32" in line
        )

    def assert_rejected(self, hosted: str) -> None:
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_asmdiff_gate_parity(self.local, hosted), 1)

    def test_current_gates_are_distinct_and_present(self) -> None:
        self.assertIn(self.i686, self.hosted)
        self.assertIn(self.x64, self.hosted)
        self.assertEqual(parity.check_asmdiff_gate_parity(self.local, self.hosted), 0)

    def test_i686_gate_cannot_stand_in_for_missing_x64_corpus(self) -> None:
        hosted = self.hosted.replace(self.x64, "# x64 differential removed", 1)
        self.assertIn(self.i686, hosted)
        self.assert_rejected(hosted)

    def test_x64_gate_cannot_stand_in_for_missing_i686_mode(self) -> None:
        hosted = self.hosted.replace(self.i686, "# i686 differential removed", 1)
        self.assertIn(self.x64, hosted)
        self.assert_rejected(hosted)

    def test_wrong_mode_compiler_corpus_or_jobs_cannot_satisfy_x64_gate(self) -> None:
        replacements = (
            self.x64.replace("--jobs 2", "--jobs 128", 1),
            self.x64.replace("--lccc target/fastbuild/lccc-x86", "--lccc target/fastbuild/lccc-i686", 1),
            self.x64.replace("merged-pr629-followup.casefile", "other.casefile", 1),
            self.x64.replace("python3 scripts/asmdiff.py", "echo python3 scripts/asmdiff.py", 1),
            self.x64.replace("--as \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as\"", "--as as", 1),
        )
        for replacement in replacements:
            with self.subTest(replacement=replacement):
                self.assertNotEqual(replacement, self.x64)
                self.assert_rejected(self.hosted.replace(self.x64, replacement, 1))

    def test_installer_is_not_optional_when_hosted_oracle_is_pinned(self) -> None:
        needle = "bash scripts/ensure_gas_247.sh x86_64-linux-gnu"
        self.assertIn(needle, self.hosted)
        self.assert_rejected(self.hosted.replace(needle, "echo installer removed", 1))

    def test_i686_gate_requires_the_pinned_oracle(self) -> None:
        pinned = "--as \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as\" "
        self.assertIn(pinned, self.i686)
        for replacement in (self.i686.replace(pinned, "", 1),
                            self.i686.replace(pinned, "--as as ", 1)):
            with self.subTest(replacement=replacement):
                self.assert_rejected(self.hosted.replace(self.i686, replacement, 1))

    def test_local_gates_require_the_pinned_oracle_and_installer(self) -> None:
        pinned = "--as \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as\""
        self.assertEqual(self.local.count(pinned), 2)
        for i in range(2):
            with self.subTest(occurrence=i):
                parts = self.local.split(pinned)
                local = pinned.join(parts[: i + 1]) + "--as as" + pinned.join(parts[i + 1 :])
                with redirect_stderr(StringIO()):
                    self.assertEqual(parity.check_asmdiff_gate_parity(local, self.hosted), 1)
        needle = "bash scripts/ensure_gas_247.sh x86_64-linux-gnu"
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_asmdiff_gate_parity(
                self.local.replace(needle, "echo installer removed", 1), self.hosted), 1)


class HostedStepsMirroredTest(unittest.TestCase):
    """check_hosted_steps_mirrored: every hosted command must run locally."""

    HOSTED = (
        "python3 scripts/check_inline_asm_utf8.py --lccc x\n"
        "cargo test --profile fastbuild --config 'profile.fastbuild.debug-assertions=true'\n"
        "python3 .github/scripts/ci-bench.py\n"
    )
    LOCAL = (
        "gate utf8 fast python3 scripts/check_inline_asm_utf8.py\n"
        "CFG=(--config 'profile.fastbuild.debug-assertions=true')\n"
        "cargo test \"${CFG[@]}\"\n"
    )

    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.saved = parity.HOSTED_ONLY
        parity.HOSTED_ONLY = Path(self.dir.name) / "hosted_only.txt"
        self.allow(".github/scripts/ci-bench.py  # measurement")

    def tearDown(self) -> None:
        parity.HOSTED_ONLY = self.saved
        self.dir.cleanup()

    def allow(self, *entries: str) -> None:
        parity.HOSTED_ONLY.write_text("# header\n" + "".join(e + "\n" for e in entries))

    def check(self, local: str, hosted: str | None = None) -> tuple[int, str]:
        err = StringIO()
        with redirect_stderr(err):
            rc = parity.check_hosted_steps_mirrored(local, self.HOSTED if hosted is None else hosted)
        return rc, err.getvalue()

    def test_mirrored_tree_passes(self) -> None:
        self.assertEqual(self.check(self.LOCAL), (0, ""))

    def test_hosted_only_script_fails(self) -> None:
        rc, err = self.check(self.LOCAL.replace("scripts/check_inline_asm_utf8.py", "x"))
        self.assertEqual(rc, 1)
        self.assertIn("script scripts/check_inline_asm_utf8.py", err)

    def test_comment_does_not_mirror_a_script_or_a_build_mode(self) -> None:
        local = (
            "# python3 scripts/check_inline_asm_utf8.py\n"
            "cargo test  # --config 'profile.fastbuild.debug-assertions=true'\n"
        )
        rc, err = self.check(local)
        self.assertEqual(rc, 1)
        self.assertIn("script scripts/check_inline_asm_utf8.py", err)
        self.assertIn("--config profile.fastbuild.debug-assertions=true", err)

    def test_missing_build_mode_fails_in_every_spelling(self) -> None:
        local = self.LOCAL.replace("debug-assertions=true", "debug-assertions=false")
        for hosted in (
            "cargo test --config 'profile.fastbuild.debug-assertions=true'",
            'cargo test --config "profile.fastbuild.debug-assertions=true"',
            "cargo test --config=profile.fastbuild.debug-assertions=true",
        ):
            rc, err = self.check(local, hosted)
            self.assertEqual(rc, 1, hosted)
            self.assertIn("--config profile.fastbuild.debug-assertions=true", err)

    def test_missing_cargo_subcommand_fails(self) -> None:
        rc, err = self.check(self.LOCAL, self.HOSTED + "cargo build --bin lccc\n")
        self.assertEqual(rc, 1)
        self.assertIn("cargo build", err)

    def test_allowlist_exempts_and_must_shrink(self) -> None:
        self.allow(".github/scripts/ci-bench.py", "scripts/check_inline_asm_utf8.py")
        rc, err = self.check(self.LOCAL)
        self.assertEqual(rc, 1)
        self.assertIn("now mirrors (delete them)", err)
        self.allow()
        rc, err = self.check(self.LOCAL)
        self.assertEqual(rc, 1)
        self.assertIn("script .github/scripts/ci-bench.py", err)

    def test_repository_is_mirrored(self) -> None:
        local = parity.LOCAL.read_text()
        hosted = "\n".join(parity.run_script_bodies(p) for p in sorted(parity.WORKFLOWS.glob("*.yml")))
        parity.HOSTED_ONLY = self.saved
        self.assertEqual(self.check(local, hosted), (0, ""))


if __name__ == "__main__":
    unittest.main()
