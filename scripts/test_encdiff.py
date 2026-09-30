#!/usr/bin/env python3
"""Unit tests for encoding-diff semantics and casefile input handling."""
from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts import encdiff, insndiff  # noqa: E402


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
    line = f"   0:\t{byte_column:<20}\t{instruction}\n"
    return subprocess.CompletedProcess(
        args=["objdump"], returncode=returncode, stdout=line, stderr="")


class EncDiffSemanticTests(unittest.TestCase):
    def test_agreed_shorter_candidate_is_a_beat_only_after_roundtrip(self):
        candidate = row_with(b"L", gas=b"GAS", clang=b"GAS")
        with mock.patch.object(encdiff, "decodes_same", return_value=True) as check:
            encdiff.classify(candidate)
        self.assertEqual(candidate.verdict, "BEATS")
        check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS", bits32=False)

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

    def test_data16_loop_counter_width_is_mode_sensitive(self):
        # In 64-bit mode the 66 is dead on E0-E3 (hardware-verified: `66
        # e2` still decrements the full RCX), so `data16 loop` is the
        # plain short row and unifies. In 32-bit mode the prefix is ALIVE
        # (LOOPW counts CX) and the spellings must stay distinct.
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("66 e2 01", "data16 loop 0x4"),
                objdump_result("e2 01", "loop 0x3")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x66\xe2\x01", b"\xe2\x01",
                bits32=False), True)
        with mock.patch.object(encdiff.subprocess, "run", side_effect=[
                objdump_result("66 e2 01", "data16 loop 0x4"),
                objdump_result("e2 01", "loop 0x3")]):
            self.assertIs(encdiff.decodes_same(
                "objdump", b"\x66\xe2\x01", b"\xe2\x01",
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

    def test_32bit_loop_counter_policy_is_a_decline_not_a_bug(self):
        # `data16 loop` in 32-bit mode: LCCC keeps `66 e2` = LOOPW (CX
        # counter) where GAS drops the prefix with a warning (LOOP/ECX) —
        # different instructions, deliberate policy, and the round-trip
        # correctly refuses to call them equivalent.
        row = row_as("data16 loop 1f", b"\x66\xe2\x00", gas=b"\xe2\x00")
        with mock.patch.object(encdiff, "decodes_same", return_value=False):
            encdiff.classify(row, bits32=True)
        self.assertEqual(row.verdict, "DECLINED-DATA16")
        self.assertIn("LOOPW", row.note)


if __name__ == "__main__":
    unittest.main()
