#!/usr/bin/env python3
"""Mutation tests: CI/local gate parity (universal invocation contracts).

Every guard in check_ci_gate_parity.py has at least one test here that
FAILS when the guard is deleted — that convention is what keeps a guard
from rotting: remove the mention rule and test_decoys_are_flagged fails,
remove the universal loop and test_a_second_unregistered_invocation_fails
fails, remove the job-level semantics and test_job_level_soft_fails fails.
The map:

  guard (function/behaviour)                          -> tests
  ------------------------------------------------------------------
  _reduce_head wrapper grammar                        -> BenignWrapperTest
  _find_invocation mention rule                       -> UniversalContractTest
                                                         .test_decoys_...
  _Cmd.quote_problem (single-quote/backslash $)       -> .test_single_quoted_pin...
                                                         .test_backslash_...
  _canonical brace form                               -> .test_brace_home_...
  _contract_status option/rest multisets              -> .test_duplicate_option...
                                                         .test_swapped_pins...
                                                         .test_lookalike_pins...
  universal loop (every invocation conforms)          -> .test_a_second_unregis...
                                                         (R1/R2/R3/R4 repros)
  per-contract coverage                               -> .test_every_contract_...
  suite env channels (W6)                             -> .test_suite_channels...
  gate_registrations parsed, speed pinned             -> GateRegistrationTest
  job/workflow-level if/continue-on-error/env (W2)    -> StepSemanticsGuardTest
  per-program env channels (W3)                       -> .test_hidden_channels...
  duplicate-key refusing loader (W9)                  -> .test_duplicate_keys...
  aggregated main (all checks run)                    -> .test_main_reports_all
"""
from __future__ import annotations

from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
import tempfile
import unittest

import check_ci_gate_parity as parity


def hosted_bodies() -> str:
    return "\n".join(
        parity.run_script_bodies(p)
        for p in sorted(parity.WORKFLOWS.glob("*.yml"))
    )


class UniversalContractTest(unittest.TestCase):
    """Every command-position invocation of a contracted program conforms.

    The audit's central finding, pinned from both directions: a conforming
    invocation existing proves NOTHING about the invocations around it (the
    old checker's `any()' accepted a second unpinned asmdiff line, a second
    unpinned step, an extra encdiff --32, and a filtered linker run next to
    the strict one), and a contract nothing satisfies is a gate that no
    longer exists.
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()
        cls.hosted = hosted_bodies()
        cls.x64 = next(
            line.strip() for line in cls.hosted.splitlines()
            if line.strip().startswith("python3 scripts/asmdiff.py")
            and "--32" not in line
        )
        cls.i686 = next(
            line.strip() for line in cls.hosted.splitlines()
            if line.strip().startswith("python3 scripts/asmdiff.py") and "--32" in line
        )

    def check(self, local: str | None = None, hosted: str | None = None) -> int:
        with redirect_stderr(StringIO()):
            return parity.check_invocation_contracts(
                self.local if local is None else local,
                self.hosted if hosted is None else hosted,
            )

    def test_real_mirrors_pass(self) -> None:
        self.assertEqual(self.check(), 0)

    def test_every_contract_is_satisfied_on_each_declared_side(self) -> None:
        # The coverage direction: removing a contract's one conforming
        # invocation from EITHER side is a failure — the gate is gone.
        for i, contract in enumerate(parity.INVOCATION_CONTRACTS):
            for side in sorted(contract.sides):
                with self.subTest(contract=contract.name, side=side):
                    if side == "local":
                        local = self.local.replace(contract.program, "true", 1)
                        self.assertNotEqual(local, self.local)
                        self.assertEqual(self.check(local=local), 1)
                    else:
                        hosted = self.hosted.replace(contract.program, "true", 1)
                        self.assertNotEqual(hosted, self.hosted)
                        self.assertEqual(self.check(hosted=hosted), 1)

    # ── the audit's four existential bypasses, as compositions ──────────

    def test_a_second_unregistered_asmdiff_line_fails(self) -> None:
        # R1: the old checker proved *a* conforming line exists; the
        # unpinned twin beside it was invisible. Universal: both must
        # conform or neither runs.
        local = self.local.replace(
            'gate "i686-tls-ie-relax" fast',
            "python3 scripts/asmdiff.py --jobs 2 --as as --objdump objdump"
            " --lccc target/fastbuild/lccc-x86\n"
            'gate "i686-tls-ie-relax" fast', 1)
        self.assertEqual(self.check(local=local), 1)

    def test_a_second_unregistered_hosted_step_fails(self) -> None:
        # R2: a GENUINE second workflow step (active, hard, no env) with
        # the unpinned command — invisible to the existential check.
        hosted = self.hosted.replace(
            self.i686,
            "python3 scripts/asmdiff.py --jobs 2 --as as --objdump objdump"
            " --lccc target/fastbuild/lccc-x86\n" + self.i686, 1)
        self.assertEqual(self.check(hosted=hosted), 1)

    def test_an_extra_unpinned_encdiff_step_fails(self) -> None:
        # R3: encdiff --32 against unpinned tools next to the real gate.
        hosted = self.hosted.replace(
            "python3 scripts/encdiff.py --offline --quiet",
            "python3 scripts/encdiff.py --32 --objdump objdump --as as\n"
            "python3 scripts/encdiff.py --offline --quiet", 1)
        self.assertEqual(self.check(hosted=hosted), 1)

    def test_an_extra_filtered_linker_run_fails(self) -> None:
        # R4 and the abbreviation variant: a second linker invocation that
        # runs a --filter subset (full env prefix or none) fails even
        # though the strict full-suite invocation beside it conforms.
        # argparse resolves `--filt' to `--filter' (allow_abbrev), so a
        # denylist of full spellings never sees it — the exact-token
        # contract does.
        anchor = "python3 tests/linker/run_linker_tests.py"
        for extra in (
            "python3 tests/linker/run_linker_tests.py --strict --filter i386_",
            "python3 tests/linker/run_linker_tests.py --strict --filt i386_",
            "python3 tests/linker/run_linker_tests.py --strict --tag dynamic",
        ):
            with self.subTest(extra=extra.split()[-1]):
                hosted = self.hosted.replace(
                    anchor, extra + "\n" + anchor, 1)
                self.assertEqual(self.check(hosted=hosted), 1)

    def test_a_local_filtered_linker_gate_fails(self) -> None:
        # The local mirror's own weakening, same class.
        local = self.local.replace(
            "python3 tests/linker/run_linker_tests.py --lccc"
            " target/fastbuild/lccc --strict",
            "python3 tests/linker/run_linker_tests.py --lccc"
            " target/fastbuild/lccc --strict --filter i386_", 1)
        self.assertEqual(self.check(local=local), 1)

    # ── token-level contract pins (unchanged doctrine, kept from the
    #    previous suite: each mutation must still fail) ──────────────────

    def test_duplicate_option_override_cannot_satisfy_a_gate(self) -> None:
        # argparse `store' is LAST-occurrence-wins: the second --objdump
        # is what runs while the first spells the pin.
        for side, text, other in (
            ("local", self.local, self.hosted),
            ("hosted", self.hosted, self.local),
        ):
            with self.subTest(side=side):
                mutated = text.replace(
                    '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"',
                    '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"'
                    " --objdump objdump", 1)
                self.assertNotEqual(mutated, text)
                if side == "local":
                    self.assertEqual(self.check(local=mutated), 1)
                else:
                    self.assertEqual(self.check(hosted=mutated), 1)

    def test_swapped_pin_values_cannot_satisfy_a_gate(self) -> None:
        # Same flat token bag, different pairing: --objdump <as> --as <objdump>.
        swapped = self.hosted.replace(
            '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"'
            ' --objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"',
            '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"'
            ' --objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"', 1)
        self.assertNotEqual(swapped, self.hosted)
        self.assertEqual(self.check(hosted=swapped), 1)

    def test_lookalike_pin_paths_are_rejected(self) -> None:
        # Exact tokens, not containment: suffixed/prefixed/sibling paths
        # spelled next to the pin are unregistered invocations.
        pin = "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"
        for lookalike in (
            pin + "-untrusted",
            "/untrusted/" + pin,
            pin + "s",
        ):
            with self.subTest(lookalike=lookalike):
                mutated = self.hosted.replace(self.x64, self.x64.replace(
                    '"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"',
                    lookalike), 1)
                self.assertEqual(self.check(hosted=mutated), 1)

    def test_injected_mode_flag_cannot_satisfy_the_64_bit_gate(self) -> None:
        # `--32' on the x86-64 invocation runs the 32-bit corpus under a
        # 64-bit-validated contract.
        mutated = self.hosted.replace(self.x64, self.x64 + " --32", 1)
        self.assertEqual(self.check(hosted=mutated), 1)

    def test_decoys_are_flagged_not_ignored(self) -> None:
        # echo/false&&/xargs/if-false lines that name a contracted program
        # are unverifiable mentions and FAIL — the mirrors' gate files have
        # no legitimate way to name these programs except to run them.
        provision = "bash scripts/ensure_gas_247.sh x86_64-linux-gnu"
        for decoy in (
            "echo " + provision,
            "false && " + provision,
            "if false; then " + provision + "; fi",
            "sh -c 'xargs " + provision + "'",
        ):
            with self.subTest(decoy=decoy.split()[0]):
                local = self.local.replace(provision, decoy, 1)
                self.assertEqual(self.check(local=local), 1)

    def test_suite_env_channels_are_part_of_the_contract(self) -> None:
        # W6: the contract suites' real-toolchain legs run against the
        # provisioned 2.47 pair, and a missing pin is an error, not a
        # skip. The env channels are pinned like every other token. Each
        # mutation is anchored to the invocation's own continuation line
        # (the module's comments legitimately mention the channel names).
        for old, new in (
            # require-mode turned off on the encdiff suite: silent skips return
            ("LCCC_REQUIRE_PINNED_ORACLE=1 \\\n    python3 scripts/test_encdiff.py",
             "LCCC_REQUIRE_PINNED_ORACLE=0 \\\n    python3 scripts/test_encdiff.py"),
            ("LCCC_REQUIRE_PINNED_ORACLE=1 \\\n    python3 scripts/test_asmdiff.py",
             "LCCC_REQUIRE_PINNED_ORACLE=0 \\\n    python3 scripts/test_asmdiff.py"),
            # the legs' tool channels redirected to the distro binaries
            ('ENCDIFF_TEST_OBJDUMP="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"',
             'ENCDIFF_TEST_OBJDUMP="/usr/bin/objdump"'),
            ('ASMDIFF_TEST_AS="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"',
             'ASMDIFF_TEST_AS="/usr/bin/as"'),
        ):
            with self.subTest(new=new.splitlines()[0][:60]):
                local = self.local.replace(old, new, 1)
                self.assertNotEqual(local, self.local)
                self.assertEqual(self.check(local=local), 1)

    def test_single_quoted_pin_is_refused(self) -> None:
        # R6: '$HOME/...' passes a LITERAL path (no expansion) — the tool
        # gets an unopenable string while the checker's token compared
        # equal. The raw spelling is checked now: a $ behind single quotes
        # can never be the expanding pin.
        local = self.local.replace(
            '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"',
            "--as '$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as'", 1)
        self.assertEqual(self.check(local=local), 1)

    def test_backslash_escaped_dollar_pin_is_refused(self) -> None:
        # "\$HOME/..." is the same literal-path defect with escaped dollars.
        local = self.local.replace(
            '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"',
            '--as "\\$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"', 1)
        self.assertEqual(self.check(local=local), 1)

    def test_brace_home_spelling_is_accepted(self) -> None:
        # R7: ${HOME} and $HOME expand identically — a working gate must
        # not be rejected for the spelling.
        local = self.local.replace(
            '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"',
            '--as "${HOME}/.cache/gas-2.47-x86_64-linux-gnu/bin/as"', 1)
        self.assertNotEqual(local, self.local)
        self.assertEqual(self.check(local=local), 0)

    def test_bash_c_payload_with_expansions_is_judged_at_payload_level(self) -> None:
        # A `bash -c' line's outer token is a single-quoted string that may
        # legitimately CONTAIN dollar references — the quoting that matters
        # is the payload's own, where the invocation resolves. Wrapping the
        # whole conforming x64 gate in bash -c must still pass.
        inner = (
            'python3 scripts/asmdiff.py --jobs 2 '
            '--as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" '
            '--objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" '
            '--lccc target/fastbuild/lccc-x86')
        hosted = self.hosted.replace(self.x64, "bash -c '" + inner + "'", 1)
        self.assertNotEqual(hosted, self.hosted)
        self.assertEqual(self.check(hosted=hosted), 0)
        # But a literal-dollar pin INSIDE the payload is still refused
        # (single quotes within the payload make the path literal).
        bad = ('bash -c "python3 scripts/asmdiff.py --jobs 2 '
               "--as '$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as'\"")
        hosted = self.hosted.replace(self.x64, bad, 1)
        self.assertEqual(self.check(hosted=hosted), 1)

    def test_local_gate_registrations_are_speed_pinned(self) -> None:
        # The registration checks live in the contract walk: name AND
        # speed class, parsed (comment-immune). Demoting a fast gate to
        # slow drops it from --fast runs, which is a coverage change.
        for contract in parity.INVOCATION_CONTRACTS:
            if contract.local_gate is None:
                continue
            name, speed = contract.local_gate
            with self.subTest(gate=name):
                local = self.local.replace(f'gate "{name}" {speed}',
                                           f'gate "{name}" slow', 1)
                self.assertEqual(self.check(local=local), 1)


class BenignWrapperTest(unittest.TestCase):
    """Execution-transparent wrappers are resolved, decoys are not.

    W5: `timeout 300 bash ...' is a legitimate refactor the old grammar
    rejected as a false alarm; the wrapper list is a fixed grammar (any
    unknown spelling still fails) so tolerance cannot become a hole.
    """

    PROVISION = "bash scripts/ensure_gas_247.sh x86_64-linux-gnu"
    local: str

    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()
        cls.hosted = hosted_bodies()

    def wrapped(self, spelling: str) -> int:
        local = self.local.replace(self.PROVISION, spelling, 1)
        self.assertNotEqual(local, self.local, "mutation did not apply")
        with redirect_stderr(StringIO()):
            return parity.check_invocation_contracts(local, self.hosted)

    def test_execution_transparent_wrappers_satisfy_the_contract(self) -> None:
        # Each spelling REPLACES the provision invocation and must still
        # satisfy the exact contract (no env perturbation: the wrappers
        # are execution-transparent, and env additions are coverage
        # changes that fail by design — see the contract test).
        for spelling in (
            "timeout 300 " + self.PROVISION,
            "timeout 10m " + self.PROVISION,
            "nice " + self.PROVISION,
            "nice -n 5 " + self.PROVISION,
            "command " + self.PROVISION,
            "setsid " + self.PROVISION,
            "setsid -w " + self.PROVISION,
            "time " + self.PROVISION,
            "nohup " + self.PROVISION,
            "env -u GAS_CACHE " + self.PROVISION,
            "/usr/bin/env " + self.PROVISION,
            "bash -c '" + self.PROVISION + "'",
            "timeout 300 env -u GAS_CACHE nice bash -c '" + self.PROVISION + "'",
        ):
            with self.subTest(spelling=spelling.split()[0]):
                self.assertEqual(self.wrapped(spelling), 0)

    def test_env_additions_are_contract_changes_not_refactors(self) -> None:
        # The grammar RESOLVES an assignment prefix (command position),
        # but the contract pins the env exactly: an added variable is a
        # conscious contract update, never a silent wrapper.
        self.assertEqual(self.wrapped("VAR=x=1 " + self.PROVISION), 1)


    def test_wrapper_grammar_is_closed(self) -> None:
        # Unknown flags and malformed arguments are not guessed at: a
        # spelling outside the grammar fails until it is registered.
        for spelling in (
            "timeout " + self.PROVISION,            # missing duration
            "timeout 300 --kill-after 10 " + self.PROVISION,  # unregistered flag
            "nice -N 5 " + self.PROVISION,          # unknown nice flag
            "env --split-output " + self.PROVISION, # unknown env flag
            "command -v " + self.PROVISION,         # -v prints, does not run
            "exec -a fake " + self.PROVISION,       # unregistered wrapper
        ):
            with self.subTest(spelling=spelling.split()[0]):
                self.assertEqual(self.wrapped(spelling), 1)

    def test_program_args_resolves_the_grammar(self) -> None:
        toks = "gate x fast env A=1 timeout 300 nice -n 2 python3 prog.py --x".split()
        self.assertEqual(parity.program_args(toks, "prog.py"), ["--x"])
        self.assertIsNone(parity.program_args("echo prog.py".split(), "prog.py"))
        self.assertIsNone(parity.program_args(
            "xargs python3 prog.py".split(), "prog.py"))


def _invocation_block(text: str, program: str) -> str:
    """The full continuation block of the FIRST invocation of `program'.

    Mutations are anchored to the invocation's own lines: a bare
    `--strict' or `LCCC_REQUIRE_I386=1' replace would hit a comment or an
    unrelated step first, silently mutating nothing that runs.
    """
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if program in line:
            start = i
            while start > 0 and lines[start - 1].rstrip().endswith("\\"):
                start -= 1
            end = i
            while lines[end].rstrip().endswith("\\") and end + 1 < len(lines):
                end += 1
            return "\n".join(lines[start:end + 1])
    raise AssertionError(f"no invocation of {program}")


class LinkerContractTest(unittest.TestCase):
    """The whole linker suite, strictly, on both sides — now as exact
    per-side invocation contracts (env prefix + options + rest), verified
    by mutating the REAL mirror texts with invocation-anchored blocks
    (the universal check requires every contract to be satisfied, so
    isolated fixtures could only ever fail)."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()
        cls.hosted = hosted_bodies()

    def check(self, local: str | None = None, hosted: str | None = None) -> int:
        with redirect_stderr(StringIO()):
            return parity.check_invocation_contracts(
                self.local if local is None else local,
                self.hosted if hosted is None else hosted)

    def test_real_mirrors_satisfy_the_linker_contracts(self) -> None:
        self.assertEqual(self.check(), 0)

    def test_every_weakening_fails_on_either_side(self) -> None:
        mutations = (
            ("--strict", ""),                                   # SKIPs would pass
            ("--strict", "--strict --filter i386_"),            # the old CI subset
            ("--strict", "--strict --filt i386_"),              # abbreviation bypass
            ("--strict", "--strict --tag dynamic"),
            ("--strict", "--strict --list"),                    # runs nothing
            ("LCCC_REQUIRE_I386=1", "LCCC_REQUIRE_I386=0"),
            ("LCCC_RELOCS_TOOL=", "LCCC_RELOCS_TOOX="),
            ("gas-2.47-x86_64-linux-gnu/bin:", "gas-2.42/bin:"),  # unpinned assembler
            ("python3 tests/linker/run_linker_tests.py",
             "echo python3 tests/linker/run_linker_tests.py"),
        )
        for side in ("local", "hosted"):
            text = self.local if side == "local" else self.hosted
            block = _invocation_block(text, "tests/linker/run_linker_tests.py")
            for old, new in mutations:
                with self.subTest(side=side, old=old, new=new):
                    mutated_block = block.replace(old, new, 1)
                    self.assertNotEqual(mutated_block, block, "mutation did not land")
                    mutated = text.replace(block, mutated_block, 1)
                    self.assertEqual(self.check(**{side: mutated}), 1)
        # The relocs-tool provisioning invocation, weakened on either side.
        for side in ("local", "hosted"):
            text = self.local if side == "local" else self.hosted
            with self.subTest(side=side, old="setup_kernel_tools"):
                mutated = text.replace(
                    'bash tests/linker/setup_kernel_tools.sh --prefix',
                    "echo tests/linker/setup_kernel_tools.sh --prefix", 1)
                self.assertNotEqual(mutated, text)
                self.assertEqual(self.check(**{side: mutated}), 1)

    def test_a_second_filtered_invocation_beside_the_strict_one_fails(self) -> None:
        # The audit's R4 as a composition: the strict invocation stays,
        # the filtered twin appears beside it.
        extra = "python3 tests/linker/run_linker_tests.py --strict --filter i386_"
        self.assertEqual(self.check(local=self.local + "\n" + extra), 1)
        self.assertEqual(self.check(hosted=self.hosted + "\n" + extra), 1)

    def test_comment_is_not_an_invocation(self) -> None:
        # The linker invocation commented out of the hosted mirror: the
        # contract loses its conforming invocation.
        block = _invocation_block(self.hosted, "tests/linker/run_linker_tests.py")
        commented = "\n".join("# " + line for line in block.splitlines())
        hosted = self.hosted.replace(block, commented, 1)
        self.assertEqual(self.check(hosted=hosted), 1)

    def test_linker_scripts_are_path_tracked(self) -> None:
        self.assertEqual(parity.COMMAND.findall("python3 tests/linker/fuzz_ld.py"),
                         ["tests/linker/fuzz_ld.py"])


class StepSemanticsGuardTest(unittest.TestCase):
    """Contracted programs must be real, failing, unperturbed steps.

    A workflow can display a gate while not running it as one: a step- or
    JOB-level `if:` that keeps it off the green-path run, step- or
    job-level `continue-on-error:` that swallows its red, a hidden env
    channel (workflow/job/step) for the program's override variables, or
    an unverifiable mention in a run body. None are visible to any
    text-level parity check — the walker itself must carry the semantics.
    """

    GATE_STEP = (
        "      - name: Verify x86-64 assembly differential\n"
        "        run: python3 scripts/asmdiff.py --jobs 2 --as p --objdump q"
        " --lccc target/fastbuild/lccc-x86\n"
    )

    @classmethod
    def setUpClass(cls) -> None:
        import yaml  # noqa: F401  (the checker requires it anyway)
        cls.dir = tempfile.TemporaryDirectory()
        cls.wf = Path(cls.dir.name) / "ci.yml"

    @classmethod
    def tearDownClass(cls) -> None:
        cls.dir.cleanup()

    def workflow(self, extra: str = "", step_attrs: str = "",
                 env: str = "", job_env: str = "", job_attrs: str = "",
                 wf_env: str = "") -> Path:
        self.wf.write_text(
            "name: CI\n"
            "on: [push, pull_request]\n"
            + (f"env:\n{wf_env}" if wf_env else "")
            + "jobs:\n"
            "  test:\n"
            "    runs-on: ubuntu-latest\n"
            + job_attrs
            + (f"    env:\n{job_env}" if job_env else "")
            + "    steps:\n"
            + self.GATE_STEP.replace("        run:", step_attrs + "        run:")
            .replace("      - name:", extra + "      - name:")
            + (env or "")
        )
        return self.wf

    def test_active_gate_step_passes(self) -> None:
        path = self.workflow()
        self.assertEqual(parity.check_step_guards([path]), 0)
        self.assertIn("python3 scripts/asmdiff.py", parity.run_script_bodies(path))

    def test_conditional_gate_step_is_not_coverage(self) -> None:
        # `if:` anything-but-always/success: the gate never runs on the
        # green-path PR run, so its body is excluded from parity AND the
        # guard reports it by name. `failure()` is the classic shape: a
        # gate that only runs once something else already failed.
        for cond in ("failure()", "runner.os == 'Windows'",
                     "${{ github.event_name == 'schedule' }}"):
            with self.subTest(cond=cond):
                path = self.workflow(
                    step_attrs=f"        if: {cond}\n")
                self.assertEqual(parity.check_step_guards([path]), 1)
                self.assertNotIn("python3 scripts/asmdiff.py",
                                 parity.run_script_bodies(path))

    def test_always_and_success_conditions_stay_active(self) -> None:
        for cond in ("always()", "${{ always() }}", "success()"):
            with self.subTest(cond=cond):
                path = self.workflow(step_attrs=f"        if: {cond}\n")
                self.assertEqual(parity.check_step_guards([path]), 0)
                self.assertIn("python3 scripts/asmdiff.py",
                              parity.run_script_bodies(path))

    def test_continue_on_error_gate_is_not_a_gate(self) -> None:
        # The step runs, but its failure cannot fail the build: a red
        # gate that CI reports green is not coverage. (The body still
        # counts as EXECUTED for path parity — the guard is what refuses
        # the soft-fail semantics.)
        path = self.workflow(step_attrs="        continue-on-error: true\n")
        self.assertEqual(parity.check_step_guards([path]), 1)
        self.assertIn("python3 scripts/asmdiff.py", parity.run_script_bodies(path))

    def test_job_level_semantics_are_visible(self) -> None:
        # W2: one word at the JOB level used to make an entire gate set
        # non-gating while the checker read only step fields.
        path = self.workflow(job_attrs="    continue-on-error: true\n")
        self.assertEqual(parity.check_step_guards([path]), 1)
        path = self.workflow(job_attrs="    if: github.event_name == 'schedule'\n")
        self.assertEqual(parity.check_step_guards([path]), 1)
        # A job-level active condition keeps the step coverage.
        path = self.workflow(job_attrs="    if: always()\n")
        self.assertEqual(parity.check_step_guards([path]), 0)
        self.assertIn("python3 scripts/asmdiff.py", parity.run_script_bodies(path))

    def test_workflow_level_env_reaches_the_guard(self) -> None:
        # W2/W3: workflow-level `env:` merges into every step's
        # environment; the old walker never read it.
        path = self.workflow(wf_env="  LCCC_OBJDUMP: /usr/bin/objdump\n")
        self.assertEqual(parity.check_step_guards([path]), 1)

    def test_hidden_tool_override_env_is_rejected(self) -> None:
        # Per-program channels: LCCC/LCCC_GAS/LCCC_OBJCOPY/LCCC_OBJDUMP
        # for the differential tools (the argparse-default override
        # channels), GAS_DL_DIR/GAS_CACHE for the provisioner, the
        # suite-test channels, the linker reference compilers. Other
        # variables are not channels for these programs and must not
        # trip the guard.
        for env in (
            "        env:\n          LCCC_GAS: /untrusted/as\n",
            "        env:\n          LCCC: /untrusted/lccc\n",
            "        env:\n          LCCC_OBJDUMP: /usr/bin/objdump\n",
        ):
            with self.subTest(env=env.strip().splitlines()[-1]):
                path = self.workflow(env=env)
                self.assertEqual(parity.check_step_guards([path]), 1)
        # Job-level env is inherited by every step of the job.
        path = self.workflow(job_env="      LCCC_OBJDUMP: /untrusted/objdump\n")
        self.assertEqual(parity.check_step_guards([path]), 1)
        # Not channels: CCC (unrelated), PATH, the linker's scratch root.
        for env in ("        env:\n          CCC: target/fastbuild/lccc\n",
                    "        env:\n          PATH: /usr/bin\n"):
            with self.subTest(env=env.strip().splitlines()[-1]):
                path = self.workflow(env=env)
                self.assertEqual(parity.check_step_guards([path]), 0)

    def test_provisioner_cache_redirects_are_channels(self) -> None:
        # W3: GAS_DL_DIR/GAS_CACHE can point provisioning at a doctored
        # tarball or source tree — hidden channels for a guarded program.
        for var in ("GAS_DL_DIR", "GAS_CACHE"):
            with self.subTest(var=var):
                self.wf.write_text(
                    "name: CI\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
                    "    steps:\n      - name: g\n"
                    f"        env:\n          {var}: /tmp/doctored\n"
                    "        run: bash scripts/ensure_gas_247.sh x86_64-linux-gnu\n")
                self.assertEqual(parity.check_step_guards([self.wf]), 1)

    def test_linker_reference_compilers_are_channels(self) -> None:
        # LINKTEST_CC/CXX pick the reference side of the linker
        # comparisons; LCCC_I386_SCRATCH_ROOT only redirects scratch
        # space (the hosted step sets it legitimately) and must pass.
        self.wf.write_text(
            "name: CI\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
            "    steps:\n      - name: g\n"
            "        env:\n          LINKTEST_CC: /untrusted/gcc\n"
            "        run: python3 tests/linker/run_linker_tests.py --strict\n")
        self.assertEqual(parity.check_step_guards([self.wf]), 1)
        self.wf.write_text(
            "name: CI\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
            "    steps:\n      - name: g\n"
            "        env:\n          LCCC_I386_SCRATCH_ROOT: /scratch\n"
            "        run: python3 tests/linker/run_linker_tests.py --strict\n")
        self.assertEqual(parity.check_step_guards([self.wf]), 0)

    def test_unverifiable_mention_in_a_step_body_is_reported(self) -> None:
        # A run body that names a contracted program without running it
        # (echo, xargs) is a problem in its own right — gate files name
        # these programs to run them.
        for body in ("echo bash scripts/ensure_gas_247.sh x86_64-linux-gnu\n",
                     "find . -name x | xargs python3 scripts/asmdiff.py\n"):
            with self.subTest(body=body.split()[0]):
                self.wf.write_text(
                    "name: CI\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
                    "    steps:\n      - name: g\n"
                    f"        run: |\n          {body}")
                self.assertEqual(parity.check_step_guards([self.wf]), 1)

    def test_duplicate_yaml_keys_are_refused(self) -> None:
        # W9: PyYAML silently keeps the LAST of two same-named keys, so a
        # workflow edit that adds a second `run:` to a step parses as
        # something other than what a reader sees. The loader refuses.
        self.wf.write_text(
            "name: CI\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
            "    steps:\n      - name: g\n"
            "        run: echo first\n"
            "        run: python3 scripts/asmdiff.py\n")
        with redirect_stderr(StringIO()) as err:
            self.assertEqual(parity.check_step_guards([self.wf]), 1)
        self.assertIn("duplicate", err.getvalue())
        with self.assertRaises(ValueError):
            parity.run_script_bodies(self.wf)

    def test_all_registered_programs_are_guarded_like_the_gates(self) -> None:
        # W8: the guarded set is derived from the contract registry — a
        # program added to INVOCATION_CONTRACTS is automatically held to
        # the execution semantics, no hand-maintained list to forget.
        for program in parity.GUARDED_PROGRAMS:
            with self.subTest(program=program):
                interp = "bash" if program.endswith(".sh") else "python3"
                self.wf.write_text(
                    "name: CI\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
                    "    steps:\n      - name: g\n"
                    f"        if: failure()\n        run: {interp} {program}\n")
                self.assertEqual(parity.check_step_guards([self.wf]), 1)


class GateRegistrationTest(unittest.TestCase):
    """Registrations are parsed from comment-stripped `gate NAME fast'
    lines: a comment can never satisfy a contract (W4), a duplicated name
    re-runs a gate and inflates the PASSED count, and a malformed speed
    class silently changes --fast behaviour."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()

    def test_real_tree_has_no_duplicate_or_malformed_registrations(self) -> None:
        self.assertEqual(parity.check_gate_name_uniqueness(self.local), 0)
        registrations, malformed = parity.gate_registrations(self.local)
        self.assertEqual(malformed, [])
        self.assertGreater(len(registrations), 50)

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

    def test_a_malformed_speed_class_is_rejected(self) -> None:
        # `gate NAME medium' runs as a fast gate (the runner only tests
        # for "slow") — a typo silently changes --fast behaviour.
        mutated = self.local + '\ngate "typo-demo" medium \\\n    true\n'
        with redirect_stderr(StringIO()) as err:
            self.assertEqual(parity.check_gate_name_uniqueness(mutated), 1)
        self.assertIn("malformed", err.getvalue())

    def test_a_comment_cannot_satisfy_a_gate_registration(self) -> None:
        # W4: demote the real registration to slow, leave the required
        # spelling in a comment — the old substring check passed while
        # the gate left the fast set.
        mutated = self.local.replace('gate "encdiff-corpus" fast',
                                     'gate "encdiff-corpus" slow', 1)
        mutated = mutated.replace(
            "# The encdiff corpus (index-fold-64",
            '# gate "encdiff-corpus" fast (documentation)\n'
            "# The encdiff corpus (index-fold-64", 1)
        self.assertIn('gate "encdiff-corpus" slow', mutated)
        with redirect_stderr(StringIO()) as err:
            self.assertEqual(
                parity.check_invocation_contracts(mutated, hosted_bodies()), 1)
        self.assertIn('gate "encdiff-corpus" must be registered', err.getvalue())

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


class FuzzDiscoveryParityTest(unittest.TestCase):
    """Executable discovery in both mirrors, and a FAST local gate."""

    def test_real_mirrors_and_negative_mutations(self):
        local = parity.LOCAL.read_text()
        hosted = hosted_bodies()
        self.assertEqual(parity.check_fuzz_test_gate_parity(local, hosted), 0)
        for mutated_local, mutated_hosted in (
            (local.replace("-p 'test_*.py'", "-p 'test_broken.py'", 1), hosted),
            (local.replace('gate "fuzz-harness-tests" fast',
                           'gate "fuzz-harness-tests" slow', 1), hosted),
            (local, hosted.replace("tests/fuzz", "tests/other", 1)),
        ):
            with self.subTest():
                self.assertNotEqual((mutated_local, mutated_hosted), (local, hosted))
                self.assertEqual(
                    parity.check_fuzz_test_gate_parity(mutated_local, mutated_hosted),
                    1)


class HostedStepsMirroredTest(unittest.TestCase):
    """The reverse direction: hosted-only steps are coverage CI can fail
    on that ci_local never sees."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()
        cls.hosted = hosted_bodies()

    def test_mirrored_tree_passes(self) -> None:
        self.assertEqual(
            parity.check_hosted_steps_mirrored(self.local, self.hosted), 0)

    def test_hosted_only_script_fails(self) -> None:
        hosted = self.hosted + "\npython3 scripts/check_new_gate.py\n"
        with redirect_stderr(StringIO()):
            self.assertEqual(
                parity.check_hosted_steps_mirrored(self.local, hosted), 1)

    def test_comment_does_not_mirror_a_script_or_a_build_mode(self) -> None:
        local = self.local.replace("cargo build", "# cargo build", 1)
        self.assertNotEqual(local, self.local)
        with redirect_stderr(StringIO()):
            self.assertEqual(
                parity.check_hosted_steps_mirrored(local, self.hosted), 1)

    def test_missing_build_mode_fails_in_every_spelling(self) -> None:
        for old, new in (
            ("cargo build", "cargo test"),
            ("--config profile=dev", "--config profile=frobnicate"),
        ):
            local = self.local.replace(old, new, 1)
            if local == self.local:
                continue
            with self.subTest(new=new):
                with redirect_stderr(StringIO()):
                    self.assertEqual(
                        parity.check_hosted_steps_mirrored(local, self.hosted), 1)

    def test_missing_cargo_subcommand_fails(self) -> None:
        # `cargo doc' is not one of ci_local's build/fmt/clippy/test runs.
        self.assertNotIn("cargo doc", self.local)
        hosted = self.hosted + "\ncargo doc\n"
        with redirect_stderr(StringIO()):
            self.assertEqual(
                parity.check_hosted_steps_mirrored(self.local, hosted), 1)

    def test_allowlist_exempts_and_must_shrink(self) -> None:
        allow = "scripts/check_hosted_only_demo.py"
        hosted = self.hosted + f"python3 {allow}\n"
        with redirect_stderr(StringIO()):
            self.assertEqual(
                parity.check_hosted_steps_mirrored(self.local, hosted), 1)
        allowlist = parity.HOSTED_ONLY
        original = allowlist.read_text()
        try:
            allowlist.write_text(original + f"{allow}  # test entry\n")
            with redirect_stderr(StringIO()):
                self.assertEqual(
                    parity.check_hosted_steps_mirrored(self.local, hosted), 0)
            # An entry ci_local now mirrors is stale and must be deleted.
            local = self.local + f"python3 {allow}\n"
            with redirect_stderr(StringIO()):
                self.assertEqual(
                    parity.check_hosted_steps_mirrored(local, hosted), 1)
        finally:
            allowlist.write_text(original)

    def test_repository_is_mirrored(self) -> None:
        self.assertEqual(
            parity.check_hosted_steps_mirrored(self.local, self.hosted), 0)


class AggregatedDiagnosticsTest(unittest.TestCase):
    """main() runs EVERY check and reports all failures (W10): one run,
    every violation — not first-fail with the rest silently unreported."""

    def test_main_reports_every_failing_check(self) -> None:
        import subprocess
        import sys as _sys
        # Mutate both a contract (unregistered invocation) and the step
        # semantics (job-level continue-on-error) in one tree copy.
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "scripts").mkdir(parents=True)
            (root / ".github" / "workflows").mkdir(parents=True)
            local = parity.LOCAL.read_text().replace(
                "bash scripts/ensure_gas_247.sh x86_64-linux-gnu",
                "echo bash scripts/ensure_gas_247.sh x86_64-linux-gnu", 1)
            (root / "scripts" / "ci_local.sh").write_text(local)
            wf = (parity.WORKFLOWS / "ci.yml").read_text().replace(
                "jobs:\n  test:\n", "jobs:\n  test:\n    continue-on-error: true\n", 1)
            (root / ".github" / "workflows" / "ci.yml").write_text(wf)
            checker = Path(parity.__file__).read_text()
            (root / "scripts" / "check_ci_gate_parity.py").write_text(checker)
            p = subprocess.run(
                [_sys.executable, str(root / "scripts" / "check_ci_gate_parity.py")],
                capture_output=True, text=True, cwd=root)
            self.assertEqual(p.returncode, 1)
            # BOTH violation families are reported in one run.
            self.assertIn("invocation contracts", p.stderr)
            self.assertIn("workflow step guards", p.stderr)


# The __main__ block lives at the very END of the module on purpose: it
# sat above the last test class for its whole life, so direct execution
# (`python3 scripts/test_ci_gate_parity.py`) collected tests only from
# the classes defined ABOVE it — three gate-uniqueness tests were dead
# code in the very execution mode CI uses, and the suite reported 24/24
# while they never ran. Discovery imports were unaffected; direct
# execution was the blind spot.
if __name__ == "__main__":
    unittest.main()
