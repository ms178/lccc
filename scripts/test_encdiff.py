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
    return encdiff.Row(
        "probe %eax,%xmm1,%xmm2",
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
        check.assert_called_once_with(encdiff._OBJDUMP, b"L", b"GAS")

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


if __name__ == "__main__":
    unittest.main()
