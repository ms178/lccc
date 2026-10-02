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
        import shutil
        if shutil.which("objdump") is None:  # pragma: no cover
            self.skipTest("objdump unavailable")
        row = row_as("testb %bpl, %al", b"\x40\x84\xe8",
                     gas=b"\x40\x84\xe8", clang=b"\x40\x84\xe8",
                     gcc=b"\x40\x84\xe8", icc=b"\x40\x84\xc5",
                     icx=b"\x40\x84\xe8")
        encdiff.classify(row)
        self.assertEqual(row.verdict, "ok-best")
        self.assertIn("round-trip verified", row.note)

    def test_clang_w_row_selector_move_verifies_ok_best(self):
        # The apx shape: same length (4B), one REX2 bit apart; with the
        # selector-view canonicalisation the round-trip must verify.
        import shutil
        if shutil.which("objdump") is None:  # pragma: no cover
            self.skipTest("objdump unavailable")
        row = row_as("movq %fs, %r16", b"\xd5\x10\x8c\xe0",
                     gas=b"\xd5\x10\x8c\xe0", clang=b"\xd5\x18\x8c\xe0",
                     gcc=b"\xd5\x10\x8c\xe0", icc=b"\xd5\x10\x8c\xe0",
                     icx=b"\xd5\x18\x8c\xe0")
        encdiff.classify(row)
        self.assertEqual(row.verdict, "ok-best")
        self.assertIn("round-trip verified", row.note)

    def test_genuinely_different_selector_move_stays_wrong(self):
        # A control: different target register is a real difference — the
        # rule must not launder it.
        import shutil
        if shutil.which("objdump") is None:  # pragma: no cover
            self.skipTest("objdump unavailable")
        row = row_as("movq %fs, %r16", b"\xd5\x10\x8c\xe0",
                     gas=b"\xd5\x10\x8c\xe0", clang=b"\xd5\x10\x8c\xe1")
        encdiff.classify(row)
        self.assertEqual(row.verdict, "WRONG-BYTES")


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
        # three NOPs -- the row is WRONG-BYTES, never DECLINED (the
        # PR #711 audit's fail-open finding).
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
        import shutil
        if shutil.which("objdump") is None:  # pragma: no cover
            self.skipTest("objdump unavailable")
        row = row_as("movq $2147483648, %r15", b"\x41\xbf\x00\x00\x00\x80",
                     gas=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00",
                     clang=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00",
                     gcc=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00",
                     icc=b"\x49\xc7\xc7\x00\x00\x00\x80",
                     icx=b"\x49\xbf\x00\x00\x00\x80\x00\x00\x00\x00")
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
