#!/usr/bin/env python3
"""Unit tests for encoding-diff semantics and casefile input handling."""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts import encdiff, insndiff  # noqa: E402

# ── The real-objdump leg's tool resolution ──────────────────────────────────────────────────────────
# The round-trip verdicts (ok-best / BEATS with "round-trip verified") are
# decided by encdiff._OBJDUMP — which defaults to the LCCC_OBJDUMP env or
# bare "objdump", i.e. whatever distro binary happens to be on PATH. The
# listing grammar the corpus round-trips against is the PINNED 2.47 pair's,
# so the leg resolves its objdump the way test_asmdiff does: an explicit
# test channel (ENCDIFF_TEST_OBJDUMP, set by both CI mirrors to the
# provisioned pair), PATH only as a bare-run fallback. Under
# LCCC_REQUIRE_PINNED_ORACLE=1 a missing pin is an ERROR, not a skip — a
# silently-skipped round-trip test is a silently-untested verdict path —
# and a set-but-invalid channel never falls back to PATH either (the
# all-or-nothing rule test_asmdiff's resolver documents).
def _leg_objdump() -> str:
    require = os.environ.get("LCCC_REQUIRE_PINNED_ORACLE") == "1"
    pinned = os.environ.get("ENCDIFF_TEST_OBJDUMP") or ""
    if require:
        if not (Path(pinned).is_file() and os.access(pinned, os.X_OK)):
            raise AssertionError(
                "LCCC_REQUIRE_PINNED_ORACLE=1 but ENCDIFF_TEST_OBJDUMP="
                f"{pinned!r} is not an executable file — provision the "
                "pinned pair first (bash scripts/ensure_gas_247.sh "
                "x86_64-linux-gnu)")
        return pinned
    if pinned:
        # An EXPLICIT channel is never completed from PATH: a set-but-
        # invalid ENCDIFF_TEST_OBJDUMP is a wiring error, and quietly
        # arbitrating the round-trip verdicts against a distro objdump
        # would paper over exactly the misconfiguration the channel
        # exists to make visible.
        if not (Path(pinned).is_file() and os.access(pinned, os.X_OK)):
            raise unittest.SkipTest(
                f"ENCDIFF_TEST_OBJDUMP={pinned!r} is not an executable "
                "file — an explicit channel is never completed from "
                "PATH; fix the path (or unset it for a bare PATH run)")
        return pinned
    found = shutil.which("objdump")
    if found:
        return found
    raise unittest.SkipTest(
        "objdump unavailable (set ENCDIFF_TEST_OBJDUMP to the pinned pair; "
        "CI sets LCCC_REQUIRE_PINNED_ORACLE=1 to refuse the skip)")


class LegObjdumpResolverTests(unittest.TestCase):
    """The explicit objdump channel never falls back to PATH when broken.

    A set-but-invalid ENCDIFF_TEST_OBJDUMP is a wiring error; the
    pre-hardening resolver quietly arbitrated the round-trip verdicts
    against whatever distro objdump was on PATH — exactly the
    unpinned-oracle defect the channel exists to prevent. Require mode
    stays an error, bare mode stays runnable (skip with the reason, never
    a silent fallback).
    """

    # A real executable INDEPENDENT of the ambient channels: capturing
    # os.environ here would poison the "valid channel" cell the moment a
    # human runs the suite with a broken ENCDIFF_TEST_OBJDUMP exported —
    # the resolver checks existence+executability, not identity, and the
    # Python interpreter is an executable every run of this suite has.
    GOOD = sys.executable

    def _env(self, **extra):
        base = {k: v for k, v in os.environ.items()
                if k not in ("ENCDIFF_TEST_OBJDUMP",
                             "LCCC_REQUIRE_PINNED_ORACLE")}
        base.update(extra)
        return mock.patch.dict(os.environ, base, clear=True)

    def test_invalid_explicit_channel_skips_never_falls_back(self) -> None:
        with self._env(ENCDIFF_TEST_OBJDUMP="/nonexistent/objdump"):
            with self.assertRaises(unittest.SkipTest) as ctx:
                _leg_objdump()
        self.assertIn("is not an executable file", str(ctx.exception))
        self.assertIn("never completed from PATH", str(ctx.exception))

    def test_valid_explicit_channel_wins_in_both_modes(self) -> None:
        for extra in ({}, {"LCCC_REQUIRE_PINNED_ORACLE": "1"}):
            with self.subTest(mode=extra or "bare"):
                with self._env(ENCDIFF_TEST_OBJDUMP=self.GOOD, **extra):
                    self.assertEqual(_leg_objdump(), self.GOOD)

    def test_require_mode_rejects_invalid_or_missing_pin(self) -> None:
        for pinned in ("/nonexistent/objdump", ""):
            with self.subTest(pinned=pinned or "<unset>"):
                with self._env(LCCC_REQUIRE_PINNED_ORACLE="1",
                               **({"ENCDIFF_TEST_OBJDUMP": pinned}
                                  if pinned else {})):
                    with self.assertRaises(AssertionError):
                        _leg_objdump()

    def test_bare_mode_without_channel_uses_path_or_skips(self) -> None:
        with self._env():
            try:
                got = _leg_objdump()
            except unittest.SkipTest as exc:
                self.assertIn("objdump unavailable", str(exc))
            else:
                self.assertTrue(Path(got).is_file())


def encoded(data: bytes) -> encdiff.Encoding:
    return encdiff.Encoding(True, data)


def row_with(lccc: bytes, **oracles: bytes) -> encdiff.Row:
    return row_as("probe %eax,%xmm1,%xmm2", lccc, **oracles)


def row_as(insn: str, lccc: bytes, **oracles: bytes) -> encdiff.Row:
    return encdiff.Row(
        insn,
        encoded(lccc),
        {name: encoded(data) for name, data in oracles.items()},
    )


def objdump_result(byte_column: str, instruction: str, returncode: int = 0):
    # The REAL pinned-2.47 byte column: 21 characters wide (three per
    # byte, space-padded — see asmdiff's parser notes). The canned shape
    # must match it, because the parser validates column consistency.
    line = f"   0:\t{byte_column:<21}\t{instruction}\n"
    return subprocess.CompletedProcess(
        args=["objdump"], returncode=returncode, stdout=line, stderr="")


def objdump_listing(*lines: str, returncode: int = 0):
    """A canned multi-line objdump stdout, verbatim."""
    return subprocess.CompletedProcess(
        args=["objdump"], returncode=returncode,
        stdout="".join(line + "\n" for line in lines), stderr="")


class EncDiffSemanticTests(unittest.TestCase):
    def test_agreed_shorter_candidate_is_a_beat_only_after_roundtrip(self):
        candidate = row_with(b"L", gas=b"GAS", clang=b"GAS")
        with mock.patch.object(encdiff, "decodes_same", return_value=True) as check:
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "BEATS")
        check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS", bits32=False,
                                      seg_dead64=False)

    def test_disagreeing_shortest_forms_are_all_roundtrip_checked(self):
        candidate = row_with(b"L", gas=b"GA", icx=b"IC")
        with mock.patch.object(
                encdiff, "decodes_same", side_effect=[True, True]) as check:
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "BEATS")
        self.assertEqual(check.call_count, 2)
        self.assertEqual(
            {call.args[2] for call in check.call_args_list}, {b"GA", b"IC"})

    def test_shorter_candidate_that_decodes_differently_is_wrong_not_a_beat(self):
        candidate = row_with(b"L", gas=b"GA", icx=b"IC")
        with mock.patch.object(
                encdiff, "decodes_same", side_effect=[True, False]):
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "WRONG-BYTES")

    def test_shorter_candidate_without_disassembler_evidence_is_unverified(self):
        candidate = row_with(b"L", gas=b"GAS", clang=b"GAS")
        with mock.patch.object(encdiff, "decodes_same", return_value=None):
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "UNVERIFIED-BEATS")

    def test_equal_length_oracle_disagreement_is_not_assumed_correct(self):
        candidate = row_with(b"AA", gas=b"AA", icx=b"BB")
        with mock.patch.object(
                encdiff, "decodes_same", side_effect=[True, False]):
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "WRONG-BYTES")

    def test_longer_candidate_must_still_match_shortest_oracle_semantically(self):
        candidate = row_with(b"LONG", gas=b"S", clang=b"S")
        with mock.patch.object(encdiff, "decodes_same", return_value=False):
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "WRONG-BYTES")

    def test_disassembler_requires_two_valid_successful_decodes(self):
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("90", "nop"),
                objdump_result("90", "nop")]):
            self.assertIs(encdiff.decodes_same("objdump", b"\x90", b"\x90"), True)

        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("90", "nop"),
                objdump_result("cc", "int3")]):
            self.assertIs(encdiff.decodes_same("objdump", b"\x90", b"\xcc"), False)

        # Objdump's `{vex}` selector marks a legal encoding of the same
        # EVEX-only mnemonic; it is not an architectural operand or opcode.
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("c4 e2 71 51 10", "{vex} vpdpbusds (%rax),%xmm1,%xmm2"),
                objdump_result("62 f2 75 08 51 10", "vpdpbusds (%rax),%xmm1,%xmm2")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\xc4\xe2\x71\x51\x10",
                b"\x62\xf2\x75\x08\x51\x10"), True)

        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("0f", ".byte 0xf"),
                objdump_result("0f", ".byte 0xf")]):
            self.assertIs(encdiff.decodes_same("objdump", b"\x0f", b"\x0f"), None)

        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("90", "nop"),
                objdump_result("90", "nop", returncode=1)]):
            self.assertIs(encdiff.decodes_same("objdump", b"\x90", b"\x90"), None)

        with mock.patch.object(encdiff.subprocess, "run", side_effect=FileNotFoundError):
            self.assertIs(encdiff.decodes_same("missing-objdump", b"\x90", b"\x90"), None)

        self.assertIs(encdiff.decodes_same("objdump", b"", b""), None)

    def test_disassembler_admits_real_continuations_and_refuses_orphans(self):
        # The pinned 2.47 wraps instructions wider than the 7-byte column
        # onto continuation lines (running address, no instruction
        # column). A real 12-byte wrap is semantic evidence and compares
        # equal to itself; every malformed fragment class is None — the
        # old parser silently SKIPPED non-matching lines, so two streams
        # differing only in skipped bytes could compare equal.
        wrap = objdump_listing(
            "   0:\t48 81 84 cb 88 77 66 \taddq   $0x11223344,0x55667788(%rbx,%rcx,8)",
            "   7:\t55 44 33 22 11 ",
            "   c:\t90                   \tnop")
        with mock.patch.object(encdiff.subprocess, "run",
                               side_effect=[wrap, wrap]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x48\x81", b"\x48\x81"), True)
        # THE orphan shapes — each must refuse the whole stream:
        orphan = objdump_listing(
            "   0:\t90                   \tnop",
            "   1:\tff ")                     # column was not full: no wrap
        with mock.patch.object(encdiff.subprocess, "run",
                               side_effect=[orphan, orphan]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x90", b"\x90"), None)
        for bad in (
            # an unknown line shape is never a silent skip:
            objdump_listing("   0:\t90                   \tnop",
                            "garbage"),
            # a `...` gap of undecoded bytes is not evidence:
            objdump_listing("   0:\t90                   \tnop",
                            "\t..."),
            # an anchor byte column inconsistent with the listing width:
            objdump_listing("   0:\t90                   \tnop",
                            "   1:\t31 c0             \txor    %eax,%eax"),
            # an address-chain break between anchors:
            objdump_listing("   0:\t31 c0                \txor    %eax,%eax",
                            "   3:\t90                   \tnop"),
            # a continuation after a non-full continuation:
            objdump_listing(
                "   0:\t48 81 84 cb 88 77 66 \taddq   $0x11223344,0x55667788(%rbx,%rcx,8)",
                "   7:\t55 ",
                "   8:\t44 "),
        ):
            with self.subTest(bad=bad.stdout.splitlines()[-1]):
                with mock.patch.object(encdiff.subprocess, "run",
                                       side_effect=[bad, bad]):
                    self.assertIs(encdiff.decodes_same(
                        "objdump", b"\x90", b"\x90"), None)

    def test_wrapped_branch_target_uses_the_true_instruction_length(self):
        # A branch wider than the byte column wraps; its target marker
        # must use the TRUE length (anchor + continuations), not the
        # anchor-line byte count. Two encodings of the same transfer —
        # one wrapped 8-byte near jmp (7+1) and one short — compare
        # equal exactly when they transfer to the same place.
        wrapped_near = objdump_listing(
            "   0:\t2e 3e 26 36 2e 3e e9 \tjmp    0x6",
            "   7:\t00 ")
        short = objdump_listing("   0:\t74 02                \tje     0x2")
        # (Same-transfer check is exercised through real rows elsewhere;
        # here the pin is that the wrapped listing itself is admissible
        # evidence — the old parser skipped its continuation line and
        # computed the end address from 7 anchor bytes only.)
        with mock.patch.object(encdiff.subprocess, "run",
                               side_effect=[wrapped_near, wrapped_near]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x2e", b"\x2e"), True)

    def test_i686_roundtrip_classification_uses_32bit_disassembly(self):
        candidate = row_with(b"L", gas=b"GAS", clang=b"GAS")
        with mock.patch.object(encdiff, "decodes_same", return_value=True) as check:
            encdiff.classify(candidate, bits32=True)
        self.assertEqual(candidate.verdict, "BEATS")
        self.assertTrue(check.call_args_list)
        self.assertTrue(all(call.kwargs["bits32"]
                            for call in check.call_args_list))

    def test_i686_disassembler_uses_i386_machine_mode(self):
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("90", "nop"),
                objdump_result("90", "nop")]) as run:
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x90", b"\x90", bits32=True), True)
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(len(commands), 2)
        self.assertTrue(all(cmd[cmd.index("-m") + 1] == "i386"
                            for cmd in commands))

    def test_insndiff_roundtrip_verifies_vex_marker_and_rejects_undecodable_bytes(self):
        lccc = insndiff.Encoding(True, b"\xc4\xe2\x71\x51\x10", "")
        gas = insndiff.Encoding(True, b"\x62\xf2\x75\x08\x51\x10", "")
        with tempfile.TemporaryDirectory(prefix="insndiff-dis-") as td:
            tmp = Path(td)
            with mock.patch.object(insndiff.subprocess, "run", side_effect=[
                    objdump_result("c4 e2 71 51 10", "{vex} vpdpbusds (%rax),%xmm1,%xmm2"),
                    objdump_result("62 f2 75 08 51 10", "vpdpbusds (%rax),%xmm1,%xmm2")]):
                self.assertTrue(insndiff.verify_shorter("objdump", lccc, gas, tmp))

            with mock.patch.object(insndiff.subprocess, "run", side_effect=[
                    objdump_result("0f", ".byte 0xf"),
                    objdump_result("0f", ".byte 0xf")]):
                self.assertFalse(insndiff.verify_shorter(
                    "objdump", insndiff.Encoding(True, b"\x0f", ""),
                    insndiff.Encoding(True, b"\x0f", ""), tmp))

    def test_casefile_reader_skips_reject_groups_and_deduplicates(self):
        content = """;;; accepted
.text
vaddps %xmm1,%xmm2,%xmm3
;;; expected_failure reject
vaddps %xmm8,%xmm2,%xmm3
;;; another_positive betterok
.text
vaddps %xmm1,%xmm2,%xmm3
vmovaps %xmm1,%xmm2
"""
        with tempfile.TemporaryDirectory(prefix="encdiff-casefile-") as td:
            path = Path(td) / "probe.casefile"
            path.write_text(content)
            self.assertEqual(
                encdiff.read_casefiles([str(path)]),
                ["vaddps %xmm1,%xmm2,%xmm3", "vmovaps %xmm1,%xmm2"],
            )


class BranchMarkerTests(unittest.TestCase):
    """Direct-branch comparison: two encodings of one branch are the same
    program when EITHER the absolute target matches (backward labels sit at
    a fixed address) OR the end-relative displacement does (forward labels
    ride at a fixed distance from the instruction end)."""

    def test_backward_short_and_near_same_target_are_equivalent(self):
        # `jmp 1b` scaffolds the label before the instruction: both the
        # 2-byte short and the 5-byte near row transfer to address 0 even
        # though their end-relative displacements differ (-2 vs -5).
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("eb fe", "jmp 0x0"),
                objdump_result("e9 fb ff ff ff", "jmp 0x0")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\xeb\xfe", b"\xe9\xfb\xff\xff\xff"), True)

    def test_forward_next_insn_short_and_near_are_equivalent(self):
        # A forward label lands right after the instruction: absolute
        # targets differ (2 vs 5) because the label MOVES with the length;
        # the end-relative arm (+0 both) is the invariant.
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("eb 00", "jmp 0x2"),
                objdump_result("e9 00 00 00 00", "jmp 0x5")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\xeb\x00", b"\xe9\x00\x00\x00\x00"), True)

    def test_branches_to_different_places_are_not_equivalent(self):
        # Neither invariant matches: different absolute targets AND
        # different end-relative displacements.
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("eb fe", "jmp 0x0"),
                objdump_result("e9 fc ff ff ff", "jmp 0x1")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\xeb\xfe", b"\xe9\xfc\xff\xff\xff"), False)

    def test_callw_and_call_are_never_equivalent(self):
        # P1 regression: the 16-bit call pushes a 2-byte return address
        # where the 32-bit call pushes 4 — a real semantic difference in
        # every mode, so `callw` is never unified with `call`. (In 64-bit
        # mode the callw rendering is a hardware-invalid truncated row;
        # in 32-bit mode it is a valid but different instruction.)
        for bits32 in (False, True):
            with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                    objdump_result("66 e8 01 00", "callw 0x5"),
                    objdump_result("e8 01 00 00 00", "call 0x6")]):
                self.assertIs(encdiff.decodes_same(
                    "objdump", b"\x66\xe8\x01\x00",
                    b"\xe8\x01\x00\x00\x00", bits32=bits32), False)

    def test_jmpw_unifies_with_jmp_only_in_32bit(self):
        # Same absolute target: in 32-bit mode `jmpw` (66 e9 rel16, EIP
        # truncated to 16 bits — unobservable for in-payload targets) is
        # the same transfer as `jmp` (e9 rel32). In 64-bit mode the
        # callw/jmpw renderings are truncated hardware-invalid rows and
        # never equivalence partners.
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("66 e9 00 00", "jmpw 0x4"),
                objdump_result("e9 ff ff ff ff", "jmp 0x4")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x66\xe9\x00\x00",
                b"\xe9\xff\xff\xff\xff", bits32=True), True)
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("66 e9 00 00", "jmpw 0x4"),
                objdump_result("e9 ff ff ff ff", "jmp 0x4")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x66\xe9\x00\x00",
                b"\xe9\xff\xff\xff\xff", bits32=False), False)

    def test_data16_loop_unifies_in_both_modes(self):
        # In 64-bit mode the 66 is dead on E0-E3 (hardware-verified: `66
        # e2` still decrements the full RCX). In 32-bit mode it is NOT
        # the counter width that 66 selects (the CX/ECX counter is
        # ADDRESS-SIZE selected -- objdump renders `67 e2` as `loopw`);
        # 66 only truncates IP to 16 bits on the taken branch, which no
        # in-payload target can observe. Both modes therefore unify
        # `data16 loop` with the plain short row.
        for bits32 in (False, True):
            with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                    objdump_result("66 e2 01", "data16 loop 0x4"),
                    objdump_result("e2 01", "loop 0x3")]):
                self.assertIs(encdiff.decodes_same(
                    "objdump", b"\x66\xe2\x01", b"\xe2\x01",
                    bits32=bits32), True)
        # The genuinely different instruction stays distinct: `67 e2` is
        # LOOPW (16-bit CX counter, objdump spells it out) and must NEVER
        # unify with the plain ECX row.
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("67 e2 01", "loopw 0x4"),
                objdump_result("e2 01", "loop 0x3")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x67\xe2\x01", b"\xe2\x01",
                bits32=True), False)


class Invalid64Data16BranchTests(unittest.TestCase):
    """The architectural rule: a 66-prefixed near branch in 64-bit mode
    still consumes a FULL 4-byte displacement, so a 2-byte field (payload
    length 4 for e8/e9, 5 for 0f 8x) desynchronises the decoder."""

    def test_truncated_near_rows_are_invalid(self):
        self.assertTrue(encdiff.invalid_64_data16_branch(b"\x66\xe9\x2c\x01"))
        self.assertTrue(encdiff.invalid_64_data16_branch(b"\x66\xe8\x2c\x01"))
        self.assertTrue(encdiff.invalid_64_data16_branch(b"\x66\x0f\x84\x2c\x01"))

    def test_dead_prefix_rel32_rows_are_valid(self):
        self.assertFalse(encdiff.invalid_64_data16_branch(
            b"\x66\xe9\x2c\x01\x00\x00"))
        self.assertFalse(encdiff.invalid_64_data16_branch(
            b"\x66\xe8\x2c\x01\x00\x00"))
        self.assertFalse(encdiff.invalid_64_data16_branch(
            b"\x66\x0f\x84\x2c\x01\x00\x00"))

    def test_short_and_unrelated_rows_are_valid(self):
        self.assertFalse(encdiff.invalid_64_data16_branch(b"\x66\xeb\x01"))
        self.assertFalse(encdiff.invalid_64_data16_branch(b"\x66\x74\x01"))
        self.assertFalse(encdiff.invalid_64_data16_branch(b"\x66\xe2\x01"))
        self.assertFalse(encdiff.invalid_64_data16_branch(b"\x66\x90"))
        self.assertFalse(encdiff.invalid_64_data16_branch(b"\xe9\x2c\x01\x00\x00"))

    def test_truncated_oracle_form_is_oracle_invalid(self):
        row = row_with(b"\x66\xe9\x2c\x01\x00\x00", gas=b"\x66\xe9\x2c\x01")
        encdiff.classify(row)
        self.assertEqual(row.verdict, "ORACLE-INVALID")
        self.assertIn("shortest valid encoding of the", row.note)

    def test_truncated_lccc_form_is_wrong_bytes(self):
        row = row_with(b"\x66\xe9\x2c\x01", gas=b"\x66\xe9\x2c\x01\x00\x00")
        encdiff.classify(row)
        self.assertEqual(row.verdict, "WRONG-BYTES")
        self.assertIn("architecturally invalid", row.note)

    def test_invalid_oracle_form_is_excluded_from_comparison(self):
        valid = b"\x66\xe9\x2c\x01\x00\x00"
        row = row_with(valid, gas=b"\x66\xe9\x2c\x01", clang=valid)
        encdiff.classify(row)
        self.assertEqual(row.verdict, "ok")
        self.assertIn("invalid oracle bytes excluded", row.note)

    def test_32bit_mode_never_flags_the_rel16_rows(self):
        # In 32-bit mode the 2-byte field is architecturally correct; the
        # partition must not run at all.
        row = row_as("data16 jmp foo", b"\x66\xe9\x2c\x01",
                     gas=b"\x66\xe9\x2c\x01")
        encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "ok")


class OracleCommutativeAndSelectorViewTests(unittest.TestCase):
    """Remote-oracle divergences that are provably encoding choices, not bugs.

    Two classes surfaced by the gcc16.2/clang23.1/icx/icc run over the
    whole corpus (chunked, .godbolt-cache):
      * ICC's -O0 encoder emits the COMMUTED modrm form of TEST
        (`40 84 c5` for `test %bpl,%al`) where every other encoder, and
        lccc, emit `40 84 e8`; clang/ICC/ICX likewise commute XCHG
        (`40 86 c5` for `xchgb %bpl,%al`) where GAS/GCC/lccc emit
        `40 86 e8`. TEST's sources are read-only and order-independent;
        XCHG's exchange is its own inverse. Both unify by sorting.
      * clang/icx take the `REX.W + 8C/8E` and REX.W SLDT/STR rows where
        GAS/GCC/ICC/lccc take the 32-bit rows for the same q-spelled
        request (`d5 18 8c e0` vs `d5 10 8c e0` for `movq %fs,%r16`;
        `48 8c e0` vs `8c e0` for `movq %fs,%rax`). In 64-bit mode a
        32-bit GPR write zero-fills the upper half, so both forms leave
        the destination holding the same zero-extended selector — the
        SDM lists the rows separately but the architectural state is
        identical, and the no-W row is one byte SHORTER on the legacy
        path. Only the 32-bit view unifies with the 64-bit view; 16-bit
        views and 32-bit mode stay distinct (a 16-bit write preserves the
        upper bits).
    """

    def test_test_xchg_commutation_is_register_pair_only(self):
        # The commuted-orientation unification applies to the REGISTER
        # pair only. A memory operand (both source orders assemble to
        # the same bytes -- one encoding shape) must pass through the
        # canon UNTOUCHED: the old ([^,]+),(.+) split tore SIB operands
        # apart at their commas and sorted the debris.
        mem = "xchg %eax,0x10(%rbx,%rcx,4)"
        self.assertEqual(encdiff._canon_insn(mem), mem)
        mem2 = "test 0x8(%rbx,%rcx,8),%edx"
        self.assertEqual(encdiff._canon_insn(mem2), mem2)
        # Register pairs still unify in both orientations:
        self.assertEqual(encdiff._canon_insn("test %eax,%ebx"),
                         encdiff._canon_insn("test %ebx,%eax"))
        self.assertEqual(encdiff._canon_insn("xchg %al,%bl"),
                         encdiff._canon_insn("xchg %bl,%al"))

    def test_canon_dead_segment_law_is_fold_row_scoped(self):
        # 64-bit mode ignores ES/DS/SS overrides on data accesses, so an
        # encoding that dropped the dead byte is the same PROGRAM as one
        # that kept it — but the unification is OPT-IN (seg_dead64), set
        # per row for exactly the index-fold family whose folded view can
        # legitimately differ from GAS's raw bytes (see _ROW_SEG_FOLD).
        # Every other comparison keeps full prefix discrimination: a
        # wrongly-dropped or swapped dead byte on a non-fold row must
        # compare UNEQUAL and fail the gate, not be laundered equal.
        self.assertNotEqual(
            encdiff._canon_insn("ds mov 0x4(%rax),%rax"),
            encdiff._canon_insn("mov 0x4(%rax),%rax"))
        self.assertNotEqual(
            encdiff._canon_insn("ss mov 0x0(%rbp),%eax"),
            encdiff._canon_insn("mov 0x0(%rbp),%eax"))
        self.assertNotEqual(
            encdiff._canon_insn("es mov (%r10),%xmm0"),
            encdiff._canon_insn("mov (%r10),%xmm0"))
        # Opted in (the fold rows), the dead token is stripped so lccc's
        # shorter folded form compares equal to GAS's SIB form:
        self.assertEqual(
            encdiff._canon_insn("ds mov 0x4(%rax),%rax", seg_dead64=True),
            encdiff._canon_insn("mov 0x4(%rax),%rax", seg_dead64=True))
        self.assertEqual(
            encdiff._canon_insn("ss mov 0x0(%rbp),%eax", seg_dead64=True),
            encdiff._canon_insn("mov 0x0(%rbp),%eax", seg_dead64=True))
        self.assertEqual(
            encdiff._canon_insn("es mov (%r10),%xmm0", seg_dead64=True),
            encdiff._canon_insn("mov (%r10),%xmm0", seg_dead64=True))
        # The spellings that must NOT unify — even opted in — are spelled
        # differently by objdump, so the regex cannot touch them: NOTRACK
        # (semantic under CET), branch hints, FS/GS (rendered inline), and
        # every CS row (0x2e is the hint partner on branches).
        self.assertNotEqual(
            encdiff._canon_insn("notrack jmp *%rax", seg_dead64=True),
            encdiff._canon_insn("jmp *%rax", seg_dead64=True))
        self.assertNotEqual(
            encdiff._canon_insn("cs mov 0x4(%rax),%rax", seg_dead64=True),
            encdiff._canon_insn("mov 0x4(%rax),%rax", seg_dead64=True))
        self.assertNotEqual(
            encdiff._canon_insn("mov %fs:0x4(%rax),%rax", seg_dead64=True),
            encdiff._canon_insn("mov 0x4(%rax),%rax", seg_dead64=True))
        self.assertNotEqual(
            encdiff._canon_insn("mov %gs:0x4(%rax),%rax", seg_dead64=True),
            encdiff._canon_insn("mov 0x4(%rax),%rax", seg_dead64=True))
        # Segment operands inside the operand text are not leading tokens:
        self.assertNotEqual(
            encdiff._canon_insn("mov %ds,%eax"),
            encdiff._canon_insn("mov %eax,%eax"))
        # 32-bit mode strips nothing — even opted in: every override
        # selects a real descriptor there.
        self.assertNotEqual(
            encdiff._canon_insn("ds mov 0x4(%eax),%eax", bits32=True,
                                seg_dead64=True),
            encdiff._canon_insn("mov 0x4(%eax),%eax", bits32=True,
                                seg_dead64=True))

    def test_row_seg_fold_matches_exactly_the_view_flip_rows(self):
        # The flip set is {rbp, ebp}: an index-only scale-1 operand under
        # a segment override whose fold moves the register into the base
        # slot and changes the default-segment class. Everything else —
        # explicit bases, scale != 1, symbol displacements, other index
        # registers (rax/r10 fold but keep the DS default; rsp/esp cannot
        # be an index; r12/r13 fold to non-SS-default names) — decides
        # identically on both views and must NOT opt in.
        def row(s: str) -> bool:
            return encdiff._ROW_SEG_FOLD.search(s) is not None
        self.assertTrue(row("mov %ss:4(,%rbp,1), %rax"))
        self.assertTrue(row("mov %ds:4(,%rbp,1), %rax"))
        self.assertTrue(row("movl %es:(,%ebp,1), %eax"))
        self.assertTrue(row("mov %ss:(,%rbp,1), %rax"))   # no displacement
        self.assertTrue(row("mov %ds:0x10(,%rbp,1), %rax"))
        self.assertFalse(row("mov %ss:4(%rbp), %rax"))    # explicit base
        self.assertFalse(row("mov %ss:4(,%rbp,2), %rax"))  # scale != 1
        self.assertFalse(row("mov %ds:sym(,%rbp,1), %rax"))  # symbols never fold
        self.assertFalse(row("mov %ss:4(,%rax,1), %rax"))  # rax: no flip
        self.assertFalse(row("mov %ss:4(,%r10,1), %rax"))  # r10: no flip
        self.assertFalse(row("mov %ss:4(,%r13,1), %rax"))  # r13 folds to a
        #                       non-SS-default name (GAS 2.47 elision is
        #                       name-based rbp/ebp/rsp/esp — byte-probed)
        self.assertFalse(row("mov 4(,%rbp,1), %rax"))     # no segment override

    def test_fold_row_threads_the_dead_segment_law_into_the_roundtrip(self):
        # classify() derives the opt-in from the ROW SOURCE and hands it to
        # the round-trip comparison: the flip rows get the unification, a
        # plain fold row does not, and a non-fold row never does.
        flip = row_as("mov %ds:4(,%rbp,1), %rax", b"L", gas=b"GAS")
        with mock.patch.object(encdiff, "decodes_same", return_value=True) as check:
            encdiff.classify(flip)
        self.assertEqual(flip.verdict, "BEATS")
        check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS", bits32=False,
                                      seg_dead64=True)
        plain = row_as("mov 4(,%rax,1), %rcx", b"L", gas=b"GAS")
        with mock.patch.object(encdiff, "decodes_same", return_value=True) as check:
            encdiff.classify(plain)
        check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS", bits32=False,
                                      seg_dead64=False)

    def test_dead_segment_bytes_compare_unequal_outside_the_fold_rows(self):
        # End to end through decodes_same (the gate's actual comparison):
        # two encodings of one instruction differing only by the dead DS
        # byte are NOT the same comparison unit outside the fold rows —
        # the corpus gate fails on such a divergence — and compare equal
        # exactly when the fold-row law is opted in.
        withds = bytes.fromhex("3e488b4504")   # ds mov 0x4(%rbp),%rax
        without = bytes.fromhex("488b4504")    # mov 0x4(%rbp),%rax
        self.assertFalse(encdiff.decodes_same(
            encdiff._OBJDUMP, without, withds))
        self.assertTrue(encdiff.decodes_same(
            encdiff._OBJDUMP, without, withds, seg_dead64=True))

    def test_fold_row_law_ignores_trailing_comments(self):
        # The opt-in is decided on the COMMENT-STRIPPED source: corpus
        # rows may carry trailing `#' comments, and a comment
        # quoting a flip-shaped operand — exactly the documentation style
        # this corpus uses — must not opt its row into the dead-segment
        # strip. Before the fix, a non-flip row with such a comment had a
        # dropped-prefix regression laundered into a pass (proven with the
        # shipped predicate: the regex matched the comment text, and
        # decodes_same(seg_dead64=True) returned True for wrong bytes).
        adversarial = ("mov %ss:4(%rax), %rax"
                       "  # unlike %ds:4(,%rbp,1), this row keeps its prefix")
        # Non-vacuous both ways: the regex DOES match the raw line...
        self.assertIsNotNone(encdiff._ROW_SEG_FOLD.search(adversarial))
        # ...the comment IS legal assembler input (the row itself is a
        # normal, non-flip instruction)...
        clean = "mov %ss:4(%rax), %rax"
        self.assertIsNone(encdiff._ROW_SEG_FOLD.search(clean))
        # ...and classify derives the opt-in from the stripped source, so
        # the adversarial row threads seg_dead64=False exactly like the
        # clean one. A dropped 0x36 on this row compares UNEQUAL and fails
        # the gate (asserted end-to-end below).
        for insn in (adversarial, clean):
            row = row_as(insn, b"L", gas=b"GAS")
            with mock.patch.object(encdiff, "decodes_same",
                                   return_value=True) as check:
                encdiff.classify(row)
            check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS",
                                          bits32=False, seg_dead64=False)
        withss = bytes.fromhex("36488b4004")   # ss kept (correct for this row)
        dropped = bytes.fromhex("488b4004")    # regression: prefix dropped
        self.assertFalse(encdiff.decodes_same(
            encdiff._OBJDUMP, dropped, withss))
        # And the real flip row still opts in (the law is not over-fixed):
        flip = row_as("mov %ds:4(,%rbp,1), %rax", b"L", gas=b"GAS")
        with mock.patch.object(encdiff, "decodes_same",
                               return_value=True) as check:
            encdiff.classify(flip)
        check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS",
                                      bits32=False, seg_dead64=True)

    def test_verdict_histogram_counts_and_zero_projection_compare(self):
        # The aggregate baseline check: counts AND the rows-sha256 row
        # identity digest, compared through the NONZERO projection on
        # both sides (a class recorded at 0 is documentation — the actual
        # dict has no key for it, and raw dict equality would flag every
        # zero row as drift). Drift, a new nonzero class, and a class
        # missing from the baseline all fail. The parse is strict: every
        # verdict class exactly once, nonnegative integer counts, exactly
        # one digest line.
        rows = [row_as("a", b"L", gas=b"S"), row_as("b", b"LS", gas=b"S"),
                row_as("c", b"L", gas=b"G")]
        for r, verdict in zip(rows, ("BEATS", "LONGER", "BEATS")):
            r.verdict = verdict
        self.assertEqual(encdiff.verdict_histogram(rows),
                         {"BEATS": 2, "LONGER": 1})
        with tempfile.TemporaryDirectory() as td:
            base = Path(td) / "hist.txt"

            def baseline(text: str) -> None:
                base.write_text(text)

            def classes(overrides: dict[str, int]) -> str:
                # every verdict class exactly once, zeros included
                return "\n".join(
                    f"{cls} {overrides.get(cls, 0)}"
                    for cls in encdiff.SEVERITY) + "\n"

            good = ("# comment line\n"
                    f"# rows-sha256: {encdiff.rows_digest(rows)}\n"
                    + classes({"BEATS": 2, "LONGER": 1}))
            baseline(good)
            self.assertTrue(encdiff.check_verdict_histogram(rows, base))
            # Count drift fails:
            rows[0].verdict = "ok-best"
            self.assertFalse(encdiff.check_verdict_histogram(rows, base))
            rows[0].verdict = "BEATS"
            # A class appearing from zero fails:
            rows[0].verdict = "ok"
            self.assertFalse(encdiff.check_verdict_histogram(rows, base))
            rows[0].verdict = "BEATS"
            # An unparseable baseline fails loudly (not as a silent pass):
            baseline("BEATS two\n")
            self.assertFalse(encdiff.check_verdict_histogram(rows, base))
            # A MISSING baseline fails cleanly with accurate advice (a
            # missing file is not drift — no traceback, and no misleading
            # "update the baseline" hint):
            self.assertFalse(
                encdiff.check_verdict_histogram(rows, Path(td) / "nope.txt"))
            # An EMPTY baseline is not a contract:
            baseline("# only comments\n")
            self.assertFalse(encdiff.check_verdict_histogram(rows, base))
            # A baseline without the rows-sha256 digest line fails: counts
            # alone are a NET contract.
            baseline("# comment line\n" + classes({"BEATS": 2, "LONGER": 1}))
            self.assertFalse(encdiff.check_verdict_histogram(rows, base))
            # The digest grammar is case-insensitive, and an UPPERCASE
            # spelling of the SAME digest is accepted (normalised at the
            # parse boundary): before, it parsed fine and then compared
            # verbatim against hashlib's lowercase hexdigest — a correct
            # baseline that reported drift forever.
            baseline("# ROWS-SHA256: " + encdiff.rows_digest(rows).upper()
                     + "\n" + classes({"BEATS": 2, "LONGER": 1}))
            self.assertTrue(encdiff.check_verdict_histogram(rows, base))

    def test_rows_digest_pins_row_identity_not_verdicts(self):
        # The L1 blind spot, closed: two row sets with IDENTICAL verdict
        # counts but different row texts (a compensating delete+add) have
        # different digests, so the gate fails where the counts-only
        # contract passed. Verdict-only changes leave the digest alone
        # (BEATS staying BEATS through better bytes needs no re-record).
        left = [row_as("mov %rax, %rbx", b"L", gas=b"S"),
                row_as("mov %rcx, %rdx", b"L", gas=b"S")]
        right = [row_as("mov %rax, %rbx", b"L", gas=b"S"),
                 row_as("mov %rsi, %rdi", b"L", gas=b"S")]  # swapped row
        for r in left + right:
            r.verdict = "BEATS"
        self.assertEqual(encdiff.verdict_histogram(left),
                         encdiff.verdict_histogram(right))
        self.assertNotEqual(encdiff.rows_digest(left),
                            encdiff.rows_digest(right))
        # A verdict flip on identical rows keeps the digest:
        left[0].verdict = "ok-best"
        self.assertEqual(encdiff.rows_digest(left),
                         encdiff.rows_digest([*left[:1],
                                              row_as("mov %rcx, %rdx", b"L", gas=b"S")]))
        # And a byte-exact pin is part of the row's identity: weakening a
        # pin changes the digest even though the instruction text (and
        # every count) is unchanged.
        pinned = encdiff.Row("mov %ds:4(,%ebp,1), %rax", encoded(b"L"),
                             byte_pin=bytes.fromhex("3e67488b4504"))
        weakened = encdiff.Row("mov %ds:4(,%ebp,1), %rax", encoded(b"L"),
                               byte_pin=bytes.fromhex("67488b4504"))
        self.assertNotEqual(encdiff.rows_digest([pinned]),
                            encdiff.rows_digest([weakened]))

    def test_rows_digest_detects_a_compensating_swap_end_to_end(self):
        # End to end through check_verdict_histogram: same counts, one
        # row swapped for another of the same verdict — the counts-only
        # comparison is blind to exactly this; the digest fails it.
        rows = [row_as("mov %r8, %r9", b"L", gas=b"S"),
                row_as("mov %r10, %r11", b"L", gas=b"S")]
        for r in rows:
            r.verdict = "BEATS"
        with tempfile.TemporaryDirectory() as td:
            base = Path(td) / "hist.txt"
            base.write_text(
                "# rows-sha256: " + encdiff.rows_digest(rows) + "\n"
                + "\n".join(f"{c} {2 if c == 'BEATS' else 0}"
                            for c in encdiff.SEVERITY) + "\n")
            self.assertTrue(encdiff.check_verdict_histogram(rows, base))
            swapped = [rows[0], row_as("mov %r12, %r13", b"L", gas=b"S")]
            swapped[1].verdict = "BEATS"
            self.assertEqual(encdiff.verdict_histogram(swapped),
                             encdiff.verdict_histogram(rows))
            self.assertFalse(encdiff.check_verdict_histogram(swapped, base))

    def test_canon_insn_sorts_test_operands(self):
        self.assertEqual(
            encdiff._canon_insn("test %bpl,%al"),
            encdiff._canon_insn("test %al,%bpl"),
        )
        self.assertEqual(
            encdiff._canon_insn("test %bpl,%al"), "test %al,%bpl")

    def test_canon_insn_sorts_xchg_operands(self):
        # XCHG writes each operand with the other's value — the exchange
        # is its own inverse, so the commuted modrm orientation (clang/
        # ICC/ICX) and lccc's agree after sorting.
        self.assertEqual(
            encdiff._canon_insn("xchg %bpl,%al"),
            encdiff._canon_insn("xchg %al,%bpl"),
        )
        self.assertEqual(
            encdiff._canon_insn("xchg %bpl,%al"), "xchg %al,%bpl")

    def test_canon_insn_does_not_sort_anything_else(self):
        # CMP order is observable (a<b vs b<a); every two-operand ALU op
        # writes a destination — sorting any of those would corrupt the
        # comparison, so only TEST is touched.
        for insn in ("cmp %al,%bl", "and %al,%bl", "add %eax,%ebx",
                     "ucomiss %xmm1,%xmm2", "sub %r16,%r17",
                     "xadd %eax,%ebx"):
            self.assertEqual(encdiff._canon_insn(insn), insn)

    def test_canon_insn_unifies_selector_32bit_view_with_64bit(self):
        self.assertEqual(
            encdiff._canon_insn("mov %fs,%eax"), "mov %fs,%rax")
        self.assertEqual(
            encdiff._canon_insn("mov %fs,%r16d"), "mov %fs,%r16")
        self.assertEqual(
            encdiff._canon_insn("mov %r16d,%fs"), "mov %r16,%fs")
        self.assertEqual(
            encdiff._canon_insn("sldt %eax"), "sldt %rax")
        self.assertEqual(
            encdiff._canon_insn("str %r24d"), "str %r24")
        # Already-64-bit spellings and the W-row spellings agree.
        self.assertEqual(
            encdiff._canon_insn("mov %fs,%rax"), "mov %fs,%rax")
        self.assertEqual(
            encdiff._canon_insn("mov %r16,%fs"), "mov %r16,%fs")

    def test_canon_insn_keeps_16bit_and_memory_forms_distinct(self):
        # A 16-bit write preserves bits 63:16 — a real difference, never
        # unified with the 64-bit view.
        self.assertEqual(encdiff._canon_insn("mov %fs,%ax"), "mov %fs,%ax")
        self.assertNotEqual(
            encdiff._canon_insn("mov %fs,%ax"), "mov %fs,%rax")
        # Memory-destination/source forms carry no register size view.
        self.assertEqual(
            encdiff._canon_insn("mov %fs,(%rax)"), "mov %fs,(%rax)")
        self.assertEqual(
            encdiff._canon_insn("mov (%rax),%fs"), "mov (%rax),%fs")
        # Non-selector GPR moves are untouched.
        self.assertEqual(encdiff._canon_insn("mov %eax,%ebx"), "mov %eax,%ebx")

    def test_canon_insn_does_not_unify_selector_views_in_32bit_mode(self):
        # In 32-bit mode a 16-bit selector write preserves bits 31:16 —
        # nothing unifies there.
        self.assertEqual(
            encdiff._canon_insn("mov %fs,%eax", bits32=True), "mov %fs,%eax")
        self.assertNotEqual(
            encdiff._canon_insn("mov %fs,%eax", bits32=True),
            encdiff._canon_insn("mov %fs,%rax", bits32=True))

    def test_icc_commuted_test_row_verifies_ok_best(self):
        # The byteregs shape: same length, different bytes; with the
        # commutative canonicalisation the round-trip must verify the row
        # as ok-best instead of WRONG-BYTES.
        od = _leg_objdump()
        row = row_as("testb %bpl, %al", b"\x40\x84\xe8",
                     gas=b"\x40\x84\xe8", clang=b"\x40\x84\xe8",
                     gcc=b"\x40\x84\xe8", icc=b"\x40\x84\xc5",
                     icx=b"\x40\x84\xe8")
        with mock.patch.object(encdiff, "_OBJDUMP", od):
            encdiff.classify(row)
        self.assertEqual(row.verdict, "ok-best")
        self.assertIn("round-trip verified", row.note)

    def test_clang_w_row_selector_move_verifies_ok_best(self):
        # The apx shape: same length (4B), one REX2 bit apart; with the
        # selector-view canonicalisation the round-trip must verify.
        od = _leg_objdump()
        row = row_as("movq %fs, %r16", b"\xd5\x10\x8c\xe0",
                     gas=b"\xd5\x10\x8c\xe0", clang=b"\xd5\x18\x8c\xe0",
                     gcc=b"\xd5\x10\x8c\xe0", icc=b"\xd5\x10\x8c\xe0",
                     icx=b"\xd5\x18\x8c\xe0")
        with mock.patch.object(encdiff, "_OBJDUMP", od):
            encdiff.classify(row)
        self.assertEqual(row.verdict, "ok-best")
        self.assertIn("round-trip verified", row.note)

    def test_genuinely_different_selector_move_stays_wrong(self):
        # A control: different target register is a real difference — the
        # rule must not launder it.
        od = _leg_objdump()
        row = row_as("movq %fs, %r16", b"\xd5\x10\x8c\xe0",
                     gas=b"\xd5\x10\x8c\xe0", clang=b"\xd5\x10\x8c\xe1")
        with mock.patch.object(encdiff, "_OBJDUMP", od):
            encdiff.classify(row)
        self.assertEqual(row.verdict, "WRONG-BYTES")


class ByteExactPinTests(unittest.TestCase):
    """The `# byte-exact <hex>' row annotation.

    The dead-segment canon deliberately tolerates any ES/DS/SS choice on
    the flip rows — both views are the same 64-bit flat-mode program —
    so the flip rows' VERDICT cannot see the segment byte: before the
    pins, a dropped 0x3e on the ds fold row classified BEATS straight
    through the strip (reproduced here with real GAS bytes). The pin is
    checked before any law, canonicaliser or round-trip, which makes the
    corpus rows themselves the CI record of the prefix policy.
    """

    def test_split_byte_pin_parses_strips_and_rejects_malformed(self):
        text = "mov %ds:4(,%ebp,1), %rax   # byte-exact 3e67488b4504"
        insn, pin = encdiff.split_byte_pin(text)
        self.assertEqual(insn, "mov %ds:4(,%ebp,1), %rax")
        self.assertEqual(pin, bytes.fromhex("3e67488b4504"))
        # No annotation: the line passes through unchanged.
        self.assertEqual(encdiff.split_byte_pin("mov %eax, %ebx"),
                         ("mov %eax, %ebx", None))
        # A prose comment is not an annotation (it stays part of the
        # instruction text, exactly as before the mechanism existed)...
        insn, pin = encdiff.split_byte_pin("mov %eax, %ebx # a comment")
        self.assertEqual(insn, "mov %eax, %ebx # a comment")
        self.assertIsNone(pin)
        # ...but anything byte-exact-SHAPED that fails the strict grammar
        # is a hard error: a typo'd pin must never silently weaken the
        # row's contract back to verdict-only.
        for bad in ("mov %eax, %ebx # byte-exact 3e6",   # odd hex
                    "mov %eax, %ebx # byte-exact xyz",   # not hex
                    "mov %eax, %ebx # byte-exact",       # empty
                    "mov %eax, %ebx # byte exact 3e"):   # wrong spelling
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    encdiff.split_byte_pin(bad)

    def test_exactly_one_pin_shaped_tag_per_line(self):
        # The audit regression: a `$`-anchored regex found the LAST valid
        # tag, so a conflicting earlier pin was silently ignored (and a
        # malformed one too). The whole comment is scanned for tag-shaped
        # occurrences first: exactly one, then strict grammar.
        for bad in (
            # two valid pins, conflicting values (was: accepted, pin=91):
            "mov %eax, %ebx # byte-exact 90 # byte-exact 91",
            # two valid pins, IDENTICAL values (ambiguity is the error,
            # not the value):
            "mov %eax, %ebx # byte-exact 90 # byte-exact 90",
            # malformed first, valid second (was: accepted, pin=91):
            "mov %eax, %ebx # byte exact 90 # byte-exact 91",
            # valid first, malformed second (the shadowed case):
            "mov %eax, %ebx # byte-exact 91 # byte exact 90",
            # near-miss spellings are tag-shaped too, in any case:
            "mov %eax, %ebx # BYTE_EXACT 90 # byte-exact 91",
            "mov %eax, %ebx # byteexact 90 # byte-exact 91",
        ):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError) as raised:
                    encdiff.split_byte_pin(bad)
                self.assertIn("byte-exact", str(raised.exception))
        # A single tag in ANY case spelling is the same pin:
        insn, pin = encdiff.split_byte_pin(
            "mov %eax, %ebx # BYTE-EXACT 89d8")
        self.assertEqual((insn, pin), ("mov %eax, %ebx", b"\x89\xd8"))
        # A prose mention that is not tag-shaped (prose between the `#'
        # and the word) stays a comment, not an error:
        insn, pin = encdiff.split_byte_pin(
            "mov %eax, %ebx # this row stays byte-exact by policy")
        self.assertEqual(insn,
                         "mov %eax, %ebx # this row stays byte-exact by policy")
        self.assertIsNone(pin)

    def test_dropped_ds_prefix_on_the_ds_flip_row_is_not_a_beat(self):
        # THE adversarial row of the pin mechanism, end to end
        # through the shipped classify with REAL GAS 2.47 reference
        # bytes: lccc regresses and drops the 0x3e the pin demands
        # (emitting the 5-byte fold without the override). Without the
        # pin this row classifies BEATS with "round-trip verified" —
        # the dead-segment strip launders the missing byte — and the
        # corpus gate stays green on a segment-byte regression.
        gas_raw = bytes.fromhex("67488b042d04000000")    # GAS raw SIB view
        for lccc_bytes, expect in (
            (bytes.fromhex("3e67488b4504"), "BEATS"),       # correct: kept
            (bytes.fromhex("67488b4504"), "WRONG-BYTES"),   # regress: dropped
        ):
            with self.subTest(lccc=lccc_bytes.hex()):
                row = encdiff.Row(
                    "mov %ds:4(,%ebp,1), %rax", encoded(lccc_bytes),
                    {"gas": encoded(gas_raw)},
                    byte_pin=bytes.fromhex("3e67488b4504"))
                encdiff.classify(row)
                self.assertEqual(row.verdict, expect)
                if expect == "WRONG-BYTES":
                    self.assertIn("byte-exact pin violated", row.note)
                    self.assertIn("pinned 3e67488b4504", row.note)

    def test_pin_fires_before_any_canonisation_or_roundtrip(self):
        # The pin check precedes the dead-segment law and the round-trip
        # entirely: even an oracle that would verify ANYTHING cannot be
        # consulted for a pinned byte difference, and a row lccc refuses
        # to assemble fails its pin too (the pin's contract includes
        # assembling to exactly these bytes).
        row = encdiff.Row(
            "mov %ds:4(,%ebp,1), %rax", encoded(b"\x90"),
            {"gas": encoded(bytes.fromhex("67488b042d04000000"))},
            byte_pin=bytes.fromhex("3e67488b4504"))
        with mock.patch.object(encdiff, "decodes_same") as check:
            encdiff.classify(row)
        check.assert_not_called()
        self.assertEqual(row.verdict, "WRONG-BYTES")
        rejected = encdiff.Row(
            "mov %ds:4(,%ebp,1), %rax", encdiff.Encoding(False, None, "boom"),
            {"gas": encoded(bytes.fromhex("67488b042d04000000"))},
            byte_pin=bytes.fromhex("3e67488b4504"))
        encdiff.classify(rejected)
        self.assertEqual(rejected.verdict, "WRONG-BYTES")
        self.assertIn("did not assemble", rejected.note)

    def test_corpus_flip_annotations_match_the_encoder_pins(self):
        # The corpus annotations and the Rust unit pins are two
        # independent guards of one law; this test refuses to let them
        # drift apart. The complete flip set {rbp, ebp} x {ss, ds} must
        # be annotated in the corpus, and every corpus annotation whose
        # instruction the encoder also pins must carry the SAME bytes.
        mod = (ROOT / "src/backend/x86/assembler/encoder/mod.rs").read_text()
        start = mod.index("fn fold_decides_segment_elision_on_the_folded_view()")
        end = mod.find("\n    #[test]", start)
        body = mod[start:end]
        rust_pins = {
            insn: bytes.fromhex(hexs.replace(" ", ""))
            for insn, hexs in re.findall(
                r'assert_eq!\(hex\("([^"]+)"\), "((?:[0-9a-f]{2} ?)+)"\)',
                body)
        }
        corpus = (ROOT / "tests/encdiff-corpus/index-fold-64.insn").read_text()
        corpus_pins = {
            insn: bytes.fromhex(hexs)
            for insn, hexs in re.findall(
                r"^(.*?)\s*#\s*byte-exact\s+([0-9a-f]+)\s*$", corpus,
                re.M | re.IGNORECASE)
        }
        # The complete flip set is annotated (both halves, both segments).
        for flip in ("mov %ss:4(,%rbp,1), %rax", "mov %ds:4(,%rbp,1), %rax",
                     "mov %ss:4(,%ebp,1), %rax", "mov %ds:4(,%ebp,1), %rax"):
            self.assertIn(flip, corpus_pins,
                          "flip row lost its byte-exact annotation")
            self.assertEqual(corpus_pins[flip], rust_pins[flip])
        # And no corpus annotation anywhere contradicts an encoder pin.
        for insn, pin in corpus_pins.items():
            if insn in rust_pins:
                self.assertEqual(pin, rust_pins[insn],
                                 f"corpus pin drifts from the Rust pin for {insn!r}")

    def test_every_egpr_pin_is_corpus_recorded(self):
        # Historically only 7 of the 10 EGPR fold pins had corpus rows;
        # the (%r20)/(%r24) SIB-escape base twins and the positive
        # compressed-disp8 sign case did not. Now the coverage is a
        # CONTRACT: every pin in fold_moves_the_egpr_index_through_
        # avx512_evex must appear in the corpus with a byte-exact
        # annotation carrying the SAME bytes, and the count must be
        # exactly the pin count — a new pin without its corpus row (or a
        # deleted corpus row) fails here, not at the next audit.
        mod = (ROOT / "src/backend/x86/assembler/encoder/mod.rs").read_text()
        start = mod.index("fn fold_moves_the_egpr_index_through_avx512_evex()")
        end = mod.find("\n    #[test]", start)
        body = mod[start:end]
        rust_pins = {
            insn: bytes.fromhex(hexs.replace(" ", ""))
            for insn, hexs in re.findall(
                r'assert_eq!\(hex\("([^"]+)"\), "((?:[0-9a-f]{2} ?)+)"\)',
                body)
        }
        corpus = (ROOT / "tests/encdiff-corpus/index-fold-64.insn").read_text()
        corpus_pins = {
            insn: bytes.fromhex(hexs)
            for insn, hexs in re.findall(
                r"^(.*?)\s*#\s*byte-exact\s+([0-9a-f]+)\s*$", corpus,
                re.M | re.IGNORECASE)
        }
        self.assertEqual(len(rust_pins), 10,
                         "the EGPR pin count changed: update this contract "
                         "and the corpus together, in one commit")
        for insn, pin in rust_pins.items():
            with self.subTest(insn=insn):
                self.assertIn(insn, corpus_pins,
                              "EGPR pin has no corpus row with a byte-exact "
                              "annotation")
                self.assertEqual(corpus_pins[insn], pin)


class BytePinPolicyTests(unittest.TestCase):
    """harvest_byte_pins: a pin contract is unambiguous by construction."""

    def test_same_pin_repeated_is_idempotent(self) -> None:
        lines = ["mov %eax, %ebx # byte-exact 90",
                 "mov %eax, %ebx # byte-exact 90",
                 "mov %ecx, %edx"]
        stripped, pins = encdiff.harvest_byte_pins(lines)
        self.assertEqual(stripped, ["mov %eax, %ebx", "mov %eax, %ebx",
                                    "mov %ecx, %edx"])
        self.assertEqual(pins, {"mov %eax, %ebx": bytes.fromhex("90")})

    def test_conflicting_duplicate_pins_are_an_error(self) -> None:
        # Corpus ordering must not decide the contract: a dict's
        # last-write-wins would silently keep whichever pin came last.
        with self.assertRaisesRegex(ValueError, "conflicting bytes"):
            encdiff.harvest_byte_pins([
                "mov %eax, %ebx # byte-exact 90",
                "mov %eax, %ebx # byte-exact 91"])

    def test_pinned_and_unpinned_occurrences_are_an_error(self) -> None:
        # The same text appearing both pinned and unpinned would let the
        # deduplicated row inherit a pin by accident (or lose one).
        with self.assertRaisesRegex(ValueError, "both pinned and unpinned"):
            encdiff.harvest_byte_pins([
                "mov %eax, %ebx # byte-exact 90",
                "mov %eax, %ebx"])

    def test_case_insensitive_tag_and_hex(self) -> None:
        # `# BYTE-EXACT <hex>' and uppercase hex digits are the same pin
        # as their lowercase spelling — no near-miss spelling of the TAG
        # can fall through as prose.
        stripped, pins = encdiff.harvest_byte_pins(
            ["mov %eax, %ebx # BYTE-EXACT 3E67488B4504"])
        self.assertEqual(stripped, ["mov %eax, %ebx"])
        self.assertEqual(pins, {"mov %eax, %ebx":
                                bytes.fromhex("3e67488b4504")})
        with self.assertRaisesRegex(ValueError, "malformed byte-exact"):
            encdiff.split_byte_pin("mov %eax, %ebx # Byte_Exact 3e6")


class StrictBaselineParseTests(unittest.TestCase):
    """check_verdict_histogram: malformed baselines are errors.

    Duplicates, unknown or misspelled classes, negative counts and
    missing classes must never be silently overwritten or silently
    dropped data.
    """

    def _rows(self):
        rows = [row_as("a", b"L", gas=b"S"), row_as("b", b"L", gas=b"S")]
        for r in rows:
            r.verdict = "BEATS"
        return rows

    def _lines(self, **overrides: int) -> str:
        return "\n".join(
            f"{cls} {overrides.get(cls, 0)}"
            for cls in encdiff.SEVERITY) + "\n"

    def _check(self, text: str) -> bool:
        rows = self._rows()
        with tempfile.TemporaryDirectory() as td:
            base = Path(td) / "hist.txt"
            base.write_text(text)
            with redirect_stderr(StringIO()):
                return encdiff.check_verdict_histogram(rows, base)

    def test_well_formed_baseline_passes(self) -> None:
        good = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n"
                + self._lines(BEATS=2))
        self.assertTrue(self._check(good))

    def test_duplicate_digest_line(self) -> None:
        good = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n")
        self.assertFalse(self._check(good + "# rows-sha256: " + "0" * 64 + "\n"
                                     + self._lines(BEATS=2)))

    def test_duplicate_verdict_class(self) -> None:
        text = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n"
                + self._lines(BEATS=2) + "BEATS 2\n")
        self.assertFalse(self._check(text))

    def test_negative_count(self) -> None:
        text = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n"
                + self._lines(BEATS=2).replace("BEATS 2", "BEATS -3"))
        self.assertFalse(self._check(text))

    def test_unknown_class_nonzero(self) -> None:
        # A misspelled class with a nonzero count was already caught by
        # the count comparison; it is a parse error instead now.
        text = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n"
                + self._lines(BEATS=2).replace("BEATS 2", "BEATSS 5"))
        self.assertFalse(self._check(text))

    def test_misspelled_zero_class(self) -> None:
        # THE invisible case: a misspelled zero-count class was discarded
        # by the nonzero projection and could never fail.
        text = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n"
                + self._lines(BEATS=2).replace("ok 0", "okk 0"))
        self.assertFalse(self._check(text))

    def test_missing_class(self) -> None:
        # Every class is listed exactly once, zeros included — a class
        # absent from the baseline is a malformed baseline now (before,
        # absence compared equal through the nonzero projection).
        text = ("# rows-sha256: "
                + encdiff.rows_digest(self._rows()) + "\n"
                + self._lines(BEATS=2).replace("ok 0\n", ""))
        self.assertFalse(self._check(text))


class Data16DeclineVerificationTests(unittest.TestCase):
    """P1 regression: DECLINED-DATA16 is only claimable after the
    round-trip proves the relaxed oracle form is the same program."""

    def test_verified_decline(self):
        row = row_as("data16 je 1f", b"\x66\x0f\x84\x00\x00",
                     gas=b"\x66\x74\x00")
        with mock.patch.object(encdiff, "decodes_same", return_value=True):
            encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "DECLINED-DATA16")
        self.assertIn("round-trip verified", row.note)

    def test_unverified_equivalence_escalates_to_wrong_bytes(self):
        row = row_as("data16 je 1f", b"\x66\x0f\x84\x00\x00",
                     gas=b"\x66\x74\x00")
        with mock.patch.object(encdiff, "decodes_same", return_value=False):
            encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "WRONG-BYTES")
        self.assertIn("decline cannot be claimed", row.note)

    def test_undecodable_decline_is_unverified(self):
        row = row_as("data16 je 1f", b"\x66\x0f\x84\x00\x00",
                     gas=b"\x66\x74\x00")
        with mock.patch.object(encdiff, "decodes_same", return_value=None):
            encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "UNVERIFIED-BYTES")
        self.assertIn("fail-closed", row.note)

    def test_32bit_loop_policy_decline_is_verified_only(self):
        # `data16 loop` in 32-bit mode: LCCC keeps `66 e2` where GAS
        # drops the prefix with a warning. Both forms count ECX (the
        # counter width is address-size selected) and 66 only truncates
        # IP, so the round-trip proves them the same program -> the
        # decline is awarded WITH verification.
        row = row_as("data16 loop 1f", b"\x66\xe2\x00", gas=b"\xe2\x00")
        with mock.patch.object(encdiff, "decodes_same", return_value=True):
            encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "DECLINED-DATA16")
        self.assertIn("address-size", row.note)
        self.assertIn("same program", row.note)
        # The decline is NOT a rubber stamp: if the LCCC bytes are
        # anything other than the verified same-program form -- here
        # three NOPs -- the row is WRONG-BYTES, never DECLINED — the
        # fail-open hole this test pins shut.
        row = row_as("data16 loop 1f", b"\x90\x90\x90", gas=b"\xe2\x00")
        with mock.patch.object(encdiff, "decodes_same", return_value=False):
            encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "WRONG-BYTES")
        self.assertIn("decline cannot be claimed", row.note)


class SextImm32PartitionTests(unittest.TestCase):
    """The ICC `movq $u32-bit31, %reg` miscompile partition (64-bit mode).

    ICC's -O0 encoder emits the REX.W imm32 form (sign-extended) for
    requests whose value has bit31 set as an unsigned u32: the stored
    register value is V - 2**32, not V. objdump-verified. Those oracle
    forms are excluded from the comparison; lccc's no-W 32-bit write
    form (zero-fills upper) is correct and shortest-valid, and the row
    verifies as BEATS against the movabs oracles.
    """

    def test_form_detection(self):
        # `49 c7 c7 00 00 00 80`: REX.W C7 /7 -> stores 0xffffffff80000000.
        self.assertEqual(
            encdiff._sext_imm32_form(b"\x49\xc7\xc7\x00\x00\x00\x80"),
            0x80000000 - (1 << 32))
        # `48 c7 c0 00 00 00 80`: the rax twin.
        self.assertEqual(
            encdiff._sext_imm32_form(b"\x48\xc7\xc0\x00\x00\x00\x80"),
            0x80000000 - (1 << 32))
        # REX2 W `d5 18 c7 /0 imm32` (8 bytes): the APX sign-extending
        # form both encoders emit for plain values (mov $0x11223344,%r16
        # -> d5 18 c7 c0 ..); with bit31 set it stores V - 2**32.
        self.assertEqual(
            encdiff._sext_imm32_form(b"\xd5\x18\xc7\xc0\x00\x00\x00\x80"),
            0x80000000 - (1 << 32))
        # REX.W + B8 has NO imm32 form: movabs always consumes a full
        # imm64, so a 6-byte `48 b8 imm32` buffer is truncated and must
        # NOT be recognised as a sign-extending form.
        self.assertIsNone(
            encdiff._sext_imm32_form(b"\x48\xb8\x00\x00\x00\x80"))
        # REX2 without W (the r16d write `d5 10 b8 imm32`) zero-extends.
        self.assertIsNone(
            encdiff._sext_imm32_form(b"\xd5\x10\xb8\x00\x00\x00\x80"))
        # C7 with a non-zero /reg field is not MOV (C7 /1 encodes
        # nothing); it must not be treated as a sign-extending MOV.
        self.assertIsNone(
            encdiff._sext_imm32_form(b"\x48\xc7\xc8\x00\x00\x00\x80"))
        # Not the W form: lccc's 32-bit writes, movabs.
        self.assertIsNone(encdiff._sext_imm32_form(b"\x41\xbf\x00\x00\x00\x80"))
        self.assertIsNone(
            encdiff._sext_imm32_form(b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00"))
        # Small values return the stored value (17, 5); the request-range
        # guard lives in invalid_64_sext_imm32, not here.
        self.assertEqual(
            encdiff._sext_imm32_form(b"\x49\xc7\xc7\x11\x00\x00\x00"), 17)
        self.assertEqual(encdiff._sext_imm32_form(b"\x48\xc7\xc0\x05\x00\x00\x00"), 5)

    def test_partition_is_request_relative(self):
        insn = "movq $2147483648, %r15"
        self.assertTrue(encdiff.invalid_64_sext_imm32(
            insn, b"\x49\xc7\xc7\x00\x00\x00\x80"))
        # The same bytes are CORRECT for the sign-extended request
        # (0xffffffff80000000 explicitly): no exclusion then.
        self.assertFalse(encdiff.invalid_64_sext_imm32(
            "movq $0xffffffff80000000, %r15",
            b"\x49\xc7\xc7\x00\x00\x00\x80"))
        # Small values and non-movq rows never partition.
        self.assertFalse(encdiff.invalid_64_sext_imm32(
            "movq $5, %rax", b"\x48\xc7\xc0\x05\x00\x00\x00"))
        self.assertFalse(encdiff.invalid_64_sext_imm32(
            "movq $2147483648, %rax", b"\x41\xbf\x00\x00\x00\x80"))

    def test_icc_miscompile_row_becomes_beats(self):
        # The misc shape: lccc's 6B no-W form is shorter than the valid
        # 10B movabs oracles; ICC's 7B miscompile is excluded, and the
        # row verifies as BEATS with the exclusion noted.
        od = _leg_objdump()
        row = row_as("movq $2147483648, %r15", b"\x41\xbf\x00\x00\x00\x80",
                     gas=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00",
                     clang=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00",
                     gcc=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00",
                     icc=b"\x49\xc7\xc7\x00\x00\x00\x80",
                     icx=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00")
        with mock.patch.object(encdiff, "_OBJDUMP", od):
            encdiff.classify(row)
        self.assertEqual(row.verdict, "BEATS")
        self.assertIn("sign-extension miscompile (icc)", row.note)
        self.assertIn("round-trip verified", row.note)

    def test_lccc_sext_form_is_reported_not_laundered(self):
        # If LCCC itself emitted the miscompiling form, that is a bug:
        # reported immediately, never partitioned away.
        row = row_as("movq $2147483648, %r15", b"\x49\xc7\xc7\x00\x00\x00\x80",
                     gas=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00")
        encdiff.classify(row)
        self.assertEqual(row.verdict, "WRONG-BYTES")
        self.assertIn("sign-extends where the request asked", row.note)

if __name__ == "__main__":
    unittest.main()
