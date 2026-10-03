#!/usr/bin/env python3
"""Mutation tests: CI/local gate parity (asm-diff modes, hosted mirror, linker suite)."""
from __future__ import annotations

from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
import re
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
            and "--32" not in line
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

    def test_x64_gate_runs_the_whole_corpus(self) -> None:
        # No .casefile operands: asmdiff.py defaults to every
        # tests/asm-diff/*.casefile. A pinned subset would let new corpora
        # land ungated again.
        self.assertNotIn(".casefile", self.x64)

    def test_wrong_mode_compiler_corpus_or_jobs_cannot_satisfy_x64_gate(self) -> None:
        replacements = (
            self.x64.replace("--jobs 2", "--jobs 128", 1),
            self.x64.replace("--lccc target/fastbuild/lccc-x86", "--lccc target/fastbuild/lccc-i686", 1),
            self.x64 + " tests/asm-diff/other.casefile",
            self.x64.replace("python3 scripts/asmdiff.py", "echo python3 scripts/asmdiff.py", 1),
            self.x64.replace("--as \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as\"", "--as as", 1),
            # A depinned DISASSEMBLER must fail too: the betterok groups'
            # smaller-encoding verdicts are arbitrated by objdump, so a
            # PATH-resolved objdump is an unpinned verdict authority (the
            # PR #731 audit's M1 — 25 betterok groups across 7 casefiles
            # rode on the runner image's binutils).
            self.x64.replace("--objdump \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump\"", "--objdump objdump", 1),
            self.x64.replace(" --objdump \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump\"", "", 1),
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
        objdump_pinned = "--objdump \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump\""
        self.assertIn(pinned, self.i686)
        self.assertIn(objdump_pinned, self.i686)
        for replacement in (self.i686.replace(pinned, "", 1),
                            self.i686.replace(pinned, "--as as ", 1),
                            self.i686.replace(objdump_pinned, "--objdump objdump", 1),
                            self.i686.replace(f" {objdump_pinned}", "", 1)):
            with self.subTest(replacement=replacement):
                self.assert_rejected(self.hosted.replace(self.i686, replacement, 1))

    def test_local_gates_require_the_pinned_oracle_and_installer(self) -> None:
        pinned = '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"'
        objdump_pin = '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"'
        # Three local gates pin the 2.47 oracle: x86-asm-diff, i686-asm-diff
        # and (since S15) the offline encdiff-corpus gate. Since the PR #731
        # audit ALL THREE pin the 2.47 DISASSEMBLER too, not just encdiff:
        # asmdiff's betterok groups accept a smaller encoding only when
        # objdump proves the disassembly identical, so an unpinned objdump
        # is an unpinned VERDICT authority there exactly as in encdiff.
        # Both counts are INVARIANTS: a new gate would have to justify its
        # pins to this test, and a dropped pin anywhere trips the mutation
        # loops below.
        self.assertEqual(self.local.count(pinned), 3)
        self.assertEqual(self.local.count(objdump_pin), 3)
        # EVERY pin occurrence is behaviorally mutation-checked (the S16
        # audit response closed the range(2) hole; the S18 response made
        # the dispatch token-based — the bare-substring rfind also matched
        # `test_encdiff.py` and only worked because the asmdiff command
        # line happened to sit closer). The owner of occurrence i is the
        # differential program of the command the pin belongs to: the LAST
        # `python3 scripts/<prog>.py` token before the pin, asserted to be
        # one of the two differential drivers (test_* gates never own an
        # --as pin).
        parts = self.local.split(pinned)
        self.assertEqual(len(parts) - 1, 3)
        for i in range(3):
            with self.subTest(occurrence=i):
                programs = re.findall(
                    r"python3\s+(scripts/(?:enc|asm)diff\.py)", parts[i])
                self.assertTrue(programs, "no differential command before pin")
                owner = programs[-1]
                checker = (parity.check_encdiff_gate_parity
                           if owner == "scripts/encdiff.py"
                           else parity.check_asmdiff_gate_parity)
                local = pinned.join(parts[: i + 1]) + "--as as" + pinned.join(parts[i + 1 :])
                with redirect_stderr(StringIO()):
                    # The owning checker rejects the depinned gate ...
                    self.assertEqual(checker(local, self.hosted), 1)
                    # ... and the NON-owning checker accepts it — proving the
                    # dispatch is load-bearing: each mutation is caught by
                    # exactly its owner, so routing an occurrence to the
                    # wrong checker would let a depinned gate pass. (If a
                    # future checker learns to parse both programs, this
                    # assertion turns the overlap into a conscious update.)
                    other = (parity.check_asmdiff_gate_parity
                             if owner == "scripts/encdiff.py"
                             else parity.check_encdiff_gate_parity)
                    self.assertEqual(other(local, self.hosted), 0)
        # The un-mutated tree passes BOTH checkers (non-vacuity of the
        # 0-legs above: the non-owning checker is not simply always-1).
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_asmdiff_gate_parity(self.local, self.hosted), 0)
            self.assertEqual(parity.check_encdiff_gate_parity(self.local, self.hosted), 0)
        # EVERY objdump pin occurrence is mutation-checked through the same
        # token-based owner dispatch (three since the PR #731 audit: the two
        # asmdiff gates joined encdiff — their betterok verdicts are arbitrated
        # by the disassembler, so a depinned objdump must fail exactly its
        # owning gate's checker and no other).
        od_parts = self.local.split(objdump_pin)
        self.assertEqual(len(od_parts) - 1, 3)
        for i in range(3):
            with self.subTest(objdump_occurrence=i):
                programs = re.findall(
                    r"python3\s+(scripts/(?:enc|asm)diff\.py)", od_parts[i])
                self.assertTrue(programs, "no differential command before pin")
                owner = programs[-1]
                checker = (parity.check_encdiff_gate_parity
                           if owner == "scripts/encdiff.py"
                           else parity.check_asmdiff_gate_parity)
                local = objdump_pin.join(od_parts[: i + 1]) + \
                    "--objdump objdump" + objdump_pin.join(od_parts[i + 1:])
                with redirect_stderr(StringIO()):
                    self.assertEqual(checker(local, self.hosted), 1)
                    other = (parity.check_asmdiff_gate_parity
                             if owner == "scripts/encdiff.py"
                             else parity.check_encdiff_gate_parity)
                    self.assertEqual(other(local, self.hosted), 0)
        needle = "bash scripts/ensure_gas_247.sh x86_64-linux-gnu"
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_asmdiff_gate_parity(
                self.local.replace(needle, "echo installer removed", 1), self.hosted), 1)

    def test_encdiff_corpus_gate_pins_the_2_47_oracle(self) -> None:
        # The offline encdiff corpus is byte-truth-dependent the same way
        # the asm-diff corpora are (GAS 2.44 emits different data16-branch
        # bytes; the corpus verdicts would shift under an unpinned oracle).
        # VERDICT-truth-dependent too: objdump decides BEATS/ok through
        # decodes_same, so the disassembler is pinned to the same 2.47
        # build, and the verdict histogram baseline is part of the gate
        # (aggregate drift cannot hide behind the per-row exit contract).
        # This is the LOCAL block-level pin (exact text as ci_local spells
        # it); the behavioral both-sides contract is
        # test_hosted_encdiff_command_must_match_the_local_gate_exactly.
        block = (
            'gate "encdiff-corpus" fast \\\n'
            "    python3 scripts/encdiff.py --offline --quiet \\\n"
            "        --lccc target/fastbuild/lccc-x86 \\\n"
            '        --as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \\\n'
            '        --objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \\\n'
            "        --expect-histogram tests/encdiff-corpus/expected-verdicts.txt \\\n"
            "        --file tests/encdiff-corpus/index-fold-64.insn \\\n"
            "        --file tests/encdiff-corpus/data16-branches-64.insn"
        )
        self.assertIn(block, self.local)
        for mutation in (
            block.replace("--offline ", ""),
            block.replace(
                '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" ',
                "--as as "),
            block.replace(
                '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"',
                "--objdump objdump"),
            block.replace("--expect-histogram tests/encdiff-corpus/expected-verdicts.txt ", ""),
            # Not a suffix-drop (a prefix of the real block is always a
            # substring); wiring the WRONG corpus is the detectable wrong
            # thing: the 64-bit law corpus, not its 32-bit sibling.
            block.replace("data16-branches-64", "data16-branches-32"),
        ):
            with self.subTest(mutation=mutation[:60]):
                self.assertNotIn(mutation, self.local)

    @staticmethod
    def _encdiff_block(text: str) -> str:
        """The hosted encdiff command block, continuations included."""
        lines = text.splitlines(keepends=True)
        for i, line in enumerate(lines):
            if line.strip().startswith("python3 scripts/encdiff.py"):
                j = i
                while lines[j].rstrip("\n").endswith("\\"):
                    j += 1
                return "".join(lines[i : j + 1])
        raise AssertionError("no encdiff command in text")

    def test_hosted_encdiff_command_must_match_the_local_gate_exactly(self) -> None:
        # Both-sides behavioral parity for the encdiff corpus gate (the
        # S16 audit response; extended in S18 to the verdict chain). Path-
        # level mirroring only proves the script path appears on both
        # sides; a HOSTED-ONLY edit — depinned --as or --objdump, dropped
        # --offline or --expect-histogram, the wrong compiler, a swapped
        # corpus — passed every pre-existing check (the local block pin
        # never looked at the workflow). check_encdiff_gate_parity must
        # accept the real mirrors and reject each hosted mutation.
        self.assertEqual(
            parity.check_encdiff_gate_parity(self.local, self.hosted), 0)
        block = self._encdiff_block(self.hosted)
        mutations = (
            block.replace(
                '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"',
                "--as as"),
            block.replace(
                '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"',
                "--objdump objdump"),
            # The objdump flag dropped entirely: the gate falls back to the
            # runner image's objdump — the unpinned-oracle class the pin
            # exists to prevent. (Flag-to-newline, indentation-free, so the
            # mutation is workflow- and ci_local-layout-agnostic.)
            block.replace(
                '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \\\n', ""),
            block.replace("--expect-histogram tests/encdiff-corpus/expected-verdicts.txt", ""),
            block.replace("expected-verdicts.txt", "some-other-baseline.txt"),
            block.replace("--offline ", ""),
            block.replace("--quiet ", ""),
            block.replace("--lccc target/fastbuild/lccc-x86",
                          "--lccc target/fastbuild/lccc-i686"),
            block.replace("index-fold-64.insn", "index-fold-32.insn"),
            block.replace("data16-branches-64.insn", "data16-branches-32.insn"),
            # The whole step removed: scripts/encdiff.py still runs in
            # ci_local, so path parity stays green — only this checker
            # notices the hosted corpus verification is gone.
            "# encdiff corpus step removed",
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation[:60]):
                self.assertNotEqual(mutation, block)
                with redirect_stderr(StringIO()):
                    self.assertEqual(parity.check_encdiff_gate_parity(
                        self.local,
                        self.hosted.replace(block, mutation, 1)), 1)
        # Local-side mutations must be rejected too (the occurrence loop in
        # test_local_gates_require_the_pinned_oracle_and_installer covers
        # the --as/--objdump pins via the same checker; this asserts the
        # hosted side alone cannot satisfy a locally-mutated contract).
        local_block = self._encdiff_block(self.local)
        for needle, replacement in (
            ('--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"', "--as as"),
            ('--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"', "--objdump objdump"),
            ("--expect-histogram tests/encdiff-corpus/expected-verdicts.txt", ""),
        ):
            with self.subTest(local_pin=needle[:40]):
                with redirect_stderr(StringIO()):
                    self.assertEqual(parity.check_encdiff_gate_parity(
                        self.local.replace(local_block, local_block.replace(
                            needle, replacement), 1), self.hosted), 1)


class FuzzDiscoveryParityTest(unittest.TestCase):
    def test_real_mirrors_and_negative_mutations(self):
        local = parity.LOCAL.read_text()
        hosted = "\n".join(parity.run_script_bodies(p)
                           for p in sorted(parity.WORKFLOWS.glob("*.yml")))
        self.assertEqual(parity.check_fuzz_test_gate_parity(local, hosted), 0)
        command = "python3 -m unittest discover -s tests/fuzz -p 'test_*.py'"
        self.assertIn(command, local)
        self.assertIn(command, hosted)
        for replacement in ("# " + command, "echo " + command,
                            command.replace("test_*.py", "test_phi_cfg_fuzz.py"),
                            command + " || true"):
            for side in ("local", "hosted"):
                with self.subTest(side=side, replacement=replacement), redirect_stderr(StringIO()):
                    self.assertEqual(parity.check_fuzz_test_gate_parity(
                        local.replace(command, replacement) if side == "local" else local,
                        hosted.replace(command, replacement) if side == "hosted" else hosted), 1)
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_fuzz_test_gate_parity(
                local.replace('gate "fuzz-harness-tests" fast', 'gate "fuzz-harness-tests" slow'), hosted), 1)


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


class LinkerSuiteParityTest(unittest.TestCase):
    """The whole linker suite, strictly, on both sides (review of PR #661)."""

    LOCAL = (
        'gate "kernel-relocs-tool" fast \\\n'
        '    bash tests/linker/setup_kernel_tools.sh --prefix "$HOME/.cache/k"\n'
        'gate "linker-suite" fast env \\\n'
        '    PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH" \\\n'
        "    LCCC_REQUIRE_I386=1 \\\n"
        '    LCCC_RELOCS_TOOL="$HOME/.cache/k/bin/relocs" \\\n'
        "    python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --strict\n"
    )
    HOSTED = (
        'bash tests/linker/setup_kernel_tools.sh --prefix "$HOME/.cache/k"\n'
        'PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH" \\\n'
        "LCCC_REQUIRE_I386=1 \\\n"
        'LCCC_RELOCS_TOOL="$HOME/.cache/k/bin/relocs" \\\n'
        "  python3 tests/linker/run_linker_tests.py \\\n"
        "    --lccc target/fastbuild/lccc --strict -v\n"
    )

    def check(self, local: str, hosted: str) -> int:
        with redirect_stderr(StringIO()):
            return parity.check_linker_suite_parity(local, hosted)

    def test_full_strict_suite_passes(self) -> None:
        self.assertEqual(self.check(self.LOCAL, self.HOSTED), 0)

    def test_repository_runs_the_full_suite(self) -> None:
        hosted = "\n".join(parity.run_script_bodies(p) for p in sorted(parity.WORKFLOWS.glob("*.yml")))
        self.assertEqual(self.check(parity.LOCAL.read_text(), hosted), 0)

    def test_every_weakening_fails_on_either_side(self) -> None:
        mutations = (
            ("--strict", ""),                                   # SKIPs would pass
            ("--strict", "--strict --filter i386_"),            # the old CI subset
            ("--strict", "--strict --tag dynamic"),
            ("--strict", "--strict --list"),                    # runs nothing
            ("LCCC_REQUIRE_I386=1", "LCCC_REQUIRE_I386=0"),
            ("LCCC_RELOCS_TOOL=", "LCCC_RELOCS_TOOX="),
            ("gas-2.47-x86_64-linux-gnu/bin:", "gas-2.42/bin:"),  # unpinned assembler
            ("python3 tests/linker/run_linker_tests.py", "echo python3 tests/linker/run_linker_tests.py"),
            ("bash tests/linker/setup_kernel_tools.sh", "echo tests/linker/setup_kernel_tools.sh"),
        )
        for old, new in mutations:
            for side in ("local", "hosted"):
                with self.subTest(side=side, old=old, new=new):
                    local = self.LOCAL.replace(old, new) if side == "local" else self.LOCAL
                    hosted = self.HOSTED.replace(old, new) if side == "hosted" else self.HOSTED
                    self.assertNotEqual((local, hosted), (self.LOCAL, self.HOSTED))
                    self.assertEqual(self.check(local, hosted), 1)

    def test_comment_is_not_an_invocation(self) -> None:
        hosted = "\n".join("# " + line for line in self.HOSTED.splitlines())
        self.assertEqual(self.check(self.LOCAL, hosted), 1)

    def test_linker_scripts_are_path_tracked(self) -> None:
        self.assertEqual(parity.COMMAND.findall("python3 tests/linker/fuzz_ld.py"),
                         ["tests/linker/fuzz_ld.py"])


if __name__ == "__main__":
    unittest.main()


class GateNameUniquenessTest(unittest.TestCase):
    """A gate registered twice re-runs, inflates PASSED, and can silently drift.

    Four such duplicates shipped before this check existed: the rest of the
    module reduced gate commands to a `set`, which erases exactly the
    multiplicity a duplicate is made of.
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()

    def test_real_tree_has_no_duplicate_gate_names(self) -> None:
        self.assertEqual(parity.check_gate_name_uniqueness(self.local), 0)

    def test_a_repeated_registration_is_rejected(self) -> None:
        line = next(
            line
            for line in self.local.splitlines()
            if line.startswith('gate "') and line.endswith("\\")
        )
        name = line.split('"')[1]
        mutated = self.local + f'\n{line}\n    true\n'
        with redirect_stderr(StringIO()) as err:
            self.assertEqual(parity.check_gate_name_uniqueness(mutated), 1)
        self.assertIn(name, err.getvalue())
        self.assertIn("registered 2x", err.getvalue())

    def test_the_same_script_under_different_arguments_is_not_a_duplicate(self) -> None:
        # Pins the design decision, because the obvious alternative is wrong.
        # Comparing command-path multiplicity (a Counter over COMMAND.findall)
        # fails the CLEAN tree: asmdiff.py and fuzz_diff.py legitimately run
        # once per mode, check_volatile_destructuring.py runs as --self-test and
        # then for real, and COMMAND.findall also matches paths named in prose.
        # Uniqueness is therefore asserted on gate names only.
        text = (
            'gate "asmdiff-x64" fast \\\n'
            "    python3 scripts/asmdiff.py --corpus a\n"
            'gate "asmdiff-i686" fast \\\n'
            "    python3 scripts/asmdiff.py --32 --corpus a\n"
            'gate "destructuring-selftest" fast \\\n'
            "    python3 scripts/check_volatile_destructuring.py --self-test\n"
            'gate "destructuring" fast \\\n'
            "    python3 scripts/check_volatile_destructuring.py\n"
        )
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_gate_name_uniqueness(text), 0)
