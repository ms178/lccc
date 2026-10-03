#!/usr/bin/env python3
"""Unit tests for asmdiff's semantic-equality oracle.

`allow_better` groups accept a smaller LCCC encoding only when the pinned
disassembler proves both objects decode to the same instruction sequence,
so `semantically_equal` is verdict machinery: every way it can be fed
non-evidence — a failed disassembler, an empty listing, an undecodable
instruction, a `...` gap of undecoded bytes — must land on False, never
on a vacuous equality. These tests pin that contract directly, with
mocked `subprocess.run` for the failure modes that are hard to produce
on demand, and a real-toolchain leg that anchors the parser to genuine
objdump output.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts import asmdiff  # noqa: E402


# ─── canned `objdump -d' listings, in objdump's exact column format ────────
# (verified against binutils 2.44 and 2.47: "<sp><addr>:\t<bytes><pad>\t<insn>")

def _listing(lines: str) -> str:
    return ("a.o:     file format elf64-x86-64\n"
            "Disassembly of section .text:\n"
            "\n"
            "0000000000000000 <.text>:\n") + lines


# Two different encodings of the same program: the second mov uses a
# redundant SIB scale-1 index, the first does not (same effective address).
EQUAL_A = _listing(
    "   0:\t31 c0                \txor    %eax,%eax\n"
    "   2:\t48 8b 45 04           \tmov    -0x1(,%rdi,1),%rax\n")
EQUAL_B = _listing(
    "   0:\t31 c0                \txor    %eax,%eax\n"
    "   2:\t48 8b 47 ff           \tmov    -0x1(%rdi),%rax\n")

# Genuinely different programs (operand differs).
DIFFERENT_B = _listing(
    "   0:\t31 c0                \txor    %eax,%eax\n"
    "   2:\t48 8b 47 ff           \tmov    -0x1(%rsi),%rax\n")

# Undecodable renderings: a ONE-byte `(bad)' and a TWO-byte `(bad)'.
# objdump prints the bytes, but the comparison drops the byte column, so
# under the pre-audit oracle these two different garbage streams compared
# EQUAL ("(bad)" == "(bad)") — the exact false-accept this suite pins shut.
BAD_1BYTE = _listing(
    "   0:\t06                   \t(bad)\n"
    "   1:\t31 c0                \txor    %eax,%eax\n")
BAD_2BYTE = _listing(
    "   0:\t0f 04                \t(bad)\n"
    "   1:\t31 c0                \txor    %eax,%eax\n")

# A `.byte' fallback rendering (defensive: x86 `-d' prints `(bad)', but
# the refusal must not depend on which of the two spellings appears).
BYTE_FALLBACK = _listing(
    "   0:\t90                   \t.byte   0x90\n"
    "   1:\t31 c0                \txor    %eax,%eax\n")

# Identical decoded prefixes, but a `...' gap of bytes the decoder
# declined to decode at the end: the undecoded tails could differ.
GAP_TAIL = _listing(
    "   0:\t31 c0                \txor    %eax,%eax\n"
    "\t...\n")

# A branch-target comment and a {vex} annotation are not semantics.
ANNOTATED = _listing(
    "   0:\t74 02                \tje     4 <.text+0x4>\n"
    "   2:\t62 f1 7d 08 70 c1     \t{vex} vpshufd $0x1,%xmm1,%xmm0\n")


def _proc(stdout: str = "", returncode: int = 0) -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess(args=[], returncode=returncode,
                                       stdout=stdout, stderr="")


def _patch_run(table: dict[tuple[str, str], object]):
    """Patch asmdiff's subprocess.run with a lookup on (tool, object).

    Values are stdout strings (exit 0), CompletedProcess objects (exact
    exit status), or exceptions (raised verbatim). The patch records
    every invocation for non-vacuity assertions.
    """
    calls: list[list[str]] = []

    def fake_run(argv, **_kwargs):
        calls.append(list(argv))
        value = table[(argv[0], argv[2])]
        if isinstance(value, Exception):
            raise value
        if isinstance(value, subprocess.CompletedProcess):
            return value
        return _proc(stdout=value)

    return mock.patch.object(asmdiff.subprocess, "run", side_effect=fake_run), calls


OBJDUMP = "/pinned/gas-2.47-x86_64-linux-gnu/bin/objdump"
A, B = Path("/fake/a.o"), Path("/fake/b.o")


# ── The real-toolchain leg's tool resolution ───────────────────────────────
# The leg exists to anchor the parser to the listing grammar the PINNED
# oracle prints — a distro objdump is a different grammar authority, which
# is exactly the unpinned-oracle class the differential gates refuse. The
# tools therefore come from EXPLICIT test channels (ASMDIFF_TEST_AS /
# ASMDIFF_TEST_OBJDUMP, set by both CI mirrors to the provisioned 2.47
# pair), never from ambient PATH, unless a human runs the suite bare.
#
# LCCC_REQUIRE_PINNED_ORACLE=1 (set by both CI mirrors) turns a missing
# pin into a FAILURE: in CI a silently-skipped leg is a silently-untested
# parser — the same defect as an unwired suite, one level down. Bare local
# runs keep the old skip so the suite stays runnable on any box.
def _resolve_leg_tools() -> tuple[str | None, str | None, str]:
    """(as, objdump, problem) for the real-toolchain leg."""
    require = os.environ.get("LCCC_REQUIRE_PINNED_ORACLE") == "1"
    as_path = os.environ.get("ASMDIFF_TEST_AS") or ""
    od_path = os.environ.get("ASMDIFF_TEST_OBJDUMP") or ""
    if require:
        for role, path in (("as", as_path), ("objdump", od_path)):
            if not path:
                return None, None, (
                    f"LCCC_REQUIRE_PINNED_ORACLE=1 but ASMDIFF_TEST_{role.upper()} "
                    "is unset — the leg must run against the pinned 2.47 pair, "
                    "not skip")
            if not (Path(path).is_file() and os.access(path, os.X_OK)):
                return None, None, (
                    f"LCCC_REQUIRE_PINNED_ORACLE=1 but {path!r} is not an "
                    "executable file — provision the pinned pair first "
                    "(bash scripts/ensure_gas_247.sh x86_64-linux-gnu)")
        return as_path, od_path, ""
    # Bare human run: explicit channels still win, PATH is the fallback.
    if as_path and od_path:
        return as_path, od_path, ""
    found_as = shutil.which("as")
    found_od = shutil.which("objdump")
    if found_as and found_od:
        return found_as, found_od, ""
    return None, None, "needs GNU as and objdump on PATH (or ASMDIFF_TEST_AS/" \
        "ASMDIFF_TEST_OBJDUMP; CI sets LCCC_REQUIRE_PINNED_ORACLE=1 to refuse the skip)"


class ParseDisasmTests(unittest.TestCase):
    """The listing parser: admitted lines, refused lines, no silent skips."""

    def test_headers_and_labels_are_skipped(self) -> None:
        # Headers, blanks and symbol labels produce no instructions and no
        # parse failures; a listing that contains ONLY them has an empty
        # instruction stream, which is inadmissible (None) — so the skip
        # behaviour is pinned through a listing that also has one real
        # instruction.
        text = ("a.o:     file format elf64-x86-64\n"
                "\n"
                "Disassembly of section .text:\n"
                "\n"
                "0000000000000000 <.text>:\n"
                "00000000 <.text>:\n"
                "   0:\t31 c0                \txor    %eax,%eax\n")
        self.assertEqual(asmdiff._parse_disasm(text), ["xor %eax,%eax"])
        self.assertIsNone(asmdiff._parse_disasm(
            "a.o:     file format elf64-x86-64\n"
            "Disassembly of section .text:\n"
            "0000000000000000 <.text>:\n"))

    def test_empty_instruction_stream_is_not_evidence(self) -> None:
        self.assertIsNone(asmdiff._parse_disasm(""))
        self.assertIsNone(asmdiff._parse_disasm("a.o: file format elf64-x86-64\n"))

    def test_undecodable_renderings_refuse_the_whole_stream(self) -> None:
        for lines in ("   0:\t06                   \t(bad)\n",
                      "   0:\t0f 04                \t(bad)\n",
                      "   0:\t90                   \t.byte   0x90\n",
                      "   0:\t90                   \t(bad) [xmm]\n"):
            with self.subTest(lines=lines):
                self.assertIsNone(asmdiff._parse_disasm(_listing(lines)))

    def test_gap_marker_refuses_the_stream(self) -> None:
        self.assertIsNone(asmdiff._parse_disasm(GAP_TAIL))
        # ...with leading whitespace, as a bare tab-ellipsis line can also
        # appear mid-listing between symbol regions:
        self.assertIsNone(asmdiff._parse_disasm(_listing("  \t...\n")))

    def test_unknown_line_shape_is_a_parse_failure(self) -> None:
        for text in ("garbage\n",
                     "   0: 31 c0   xor %eax,%eax\n",     # spaces, no tabs
                     "   0:\tzz                    \txor %eax,%eax\n",  # not hex
                     "   0:\t3                     \txor %eax,%eax\n",  # 1-nibble byte
                     "   0:\t31 0xc0               \txor %eax,%eax\n",  # 0x-prefixed byte
                     "   0:\t                     \txor %eax,%eax\n"):    # empty bytes
            with self.subTest(text=text):
                self.assertIsNone(asmdiff._parse_disasm(_listing(text)))
        # A missing tab between the byte column and the instruction:
        self.assertIsNone(asmdiff._parse_disasm(_listing(
            "   0:\t31 c0                    xor %eax,%eax\n")))

    def test_byte_continuation_fragments_are_skipped(self) -> None:
        # objdump splits a byte column wider than one line across
        # continuation lines that carry an address and bytes but no
        # instruction column (real form, binutils 2.44/2.47 — the mov
        # below is `48 8b 04 3d ff ff ff ff', shown as 7 bytes + a 1-byte
        # fragment). The fragment must be skipped, not mistaken for a
        # parse failure, and never for a second instruction.
        self.assertEqual(
            asmdiff._parse_disasm(_listing(
                "   0:\t48 8b 04 3d ff ff ff \tmov    -0x1(,%rdi,1),%rax\n"
                "   7:\tff \n")),
            ["mov -0x1(%rdi),%rax"])
        # But a bytes-only line whose tokens are NOT bytes stays a parse
        # failure (objdump never emits prose there):
        self.assertIsNone(asmdiff._parse_disasm(_listing(
            "   7:\tff zz\n")))

    def test_normalisation_chain(self) -> None:
        # Branch-target comments, {vex}/{evex} annotations, scale-1
        # indexes and explicit zero displacements are encoding spelling,
        # not semantics; each is unified before comparison.
        self.assertEqual(
            asmdiff._parse_disasm(ANNOTATED),
            ["je 4 <.text+0x4>", "vpshufd $0x1,%xmm1,%xmm0"])
        zero_disp = _listing("   0:\t48 8b 45 00           \tmov    0x0(%rax),%rbx\n")
        self.assertEqual(asmdiff._parse_disasm(zero_disp), ["mov (%rax),%rbx"])


class SemanticallyEqualTests(unittest.TestCase):
    """The eight contract cases, through the public oracle entry point."""

    def _equal(self, a_val, b_val) -> bool:
        table = {(OBJDUMP, str(A)): a_val, (OBJDUMP, str(B)): b_val}
        patch, calls = _patch_run(table)
        with patch:
            return asmdiff.semantically_equal(A, B, OBJDUMP), calls

    def test_valid_equal(self) -> None:
        result, calls = self._equal(EQUAL_A, EQUAL_B)
        self.assertTrue(result)
        # Non-vacuity: both objects were disassembled with the PINNED
        # tool, in the -d form, exactly once each.
        self.assertEqual(calls, [[OBJDUMP, "-d", str(A)], [OBJDUMP, "-d", str(B)]])

    def test_valid_different(self) -> None:
        result, _ = self._equal(EQUAL_A, DIFFERENT_B)
        self.assertFalse(result)

    def test_one_byte_vs_two_byte_bad(self) -> None:
        # THE regression pin: different garbage must not compare equal.
        # The pre-audit oracle returned True here — both listings
        # normalised to the same "(bad)" text after the byte column was
        # dropped — laundering a size-only BETTER on undecodable evidence.
        result, _ = self._equal(BAD_1BYTE, BAD_2BYTE)
        self.assertFalse(result)

    def test_byte_fallback(self) -> None:
        result, _ = self._equal(EQUAL_A, BYTE_FALLBACK)
        self.assertFalse(result)

    def test_one_nonzero_exit(self) -> None:
        result, _ = self._equal(EQUAL_A, _proc(returncode=1))
        self.assertFalse(result)

    def test_two_nonzero_exits(self) -> None:
        # THE fail-open pin: two failed disassemblers printed nothing,
        # and the pre-audit oracle compared the two empty strings and
        # returned True. Failure is never equality.
        result, _ = self._equal(_proc(returncode=1), _proc(returncode=3))
        self.assertFalse(result)

    def test_empty_output(self) -> None:
        # Exit 0 with nothing on stdout (an unreadable object, say):
        # no instructions is no evidence.
        result, _ = self._equal(_proc(""), _proc(""))
        self.assertFalse(result)

    def test_timeout_and_oserror(self) -> None:
        for exc in (subprocess.TimeoutExpired(cmd=[OBJDUMP], timeout=120),
                    FileNotFoundError(OBJDUMP)):
            with self.subTest(exc=type(exc).__name__):
                table = {(OBJDUMP, str(A)): EQUAL_A, (OBJDUMP, str(B)): exc}
                patch, _ = _patch_run(table)
                with patch:
                    self.assertFalse(asmdiff.semantically_equal(A, B, OBJDUMP))
                # The exception on the FIRST object refuses too:
                table = {(OBJDUMP, str(A)): exc, (OBJDUMP, str(B)): EQUAL_B}
                patch, _ = _patch_run(table)
                with patch:
                    self.assertFalse(asmdiff.semantically_equal(A, B, OBJDUMP))

    def test_gap_of_undecoded_bytes(self) -> None:
        # Identical decoded prefixes, undecoded tails behind a `...':
        # the tails could differ, so this is not equality either.
        result, _ = self._equal(GAP_TAIL, GAP_TAIL)
        self.assertFalse(result)


class RealToolchainTests(unittest.TestCase):
    """Anchor the parser to a genuine objdump: real bytes, real listings.

    The tools are resolved once per class: the explicit ASMDIFF_TEST_*
    channels (both CI mirrors pass the provisioned 2.47 pair through them),
    PATH only as a bare-run fallback. Under LCCC_REQUIRE_PINNED_ORACLE=1
    a missing or non-executable pin is an ERROR, not a skip: CI must never
    report this leg green because it quietly ran nothing.
    """

    @classmethod
    def setUpClass(cls) -> None:
        # `as` is a Python keyword, so the assembler lands on cls.as_tool.
        cls.as_tool, cls.objdump, problem = _resolve_leg_tools()
        if problem:
            if os.environ.get("LCCC_REQUIRE_PINNED_ORACLE") == "1":
                raise AssertionError(problem)
            raise unittest.SkipTest(problem)

    def setUp(self) -> None:
        # One directory per test: the assembled objects must outlive the
        # helper call that creates them.
        self._td = tempfile.TemporaryDirectory(prefix="asmdiff-test-")
        self.addCleanup(self._td.cleanup)
        self._n = 0

    def _obj(self, asm: str) -> Path:
        self._n += 1
        src = Path(self._td.name) / f"p{self._n}.s"
        out = Path(self._td.name) / f"p{self._n}.o"
        src.write_text(".text\n" + asm + "\n")
        subprocess.run([self.as_tool, "-o", str(out), str(src)], check=True)
        return out

    def test_scale1_sib_fold_is_equality(self) -> None:
        a = self._obj("mov -0x1(,%rdi,1), %rax")
        b = self._obj("mov -0x1(%rdi), %rax")
        self.assertTrue(asmdiff.semantically_equal(a, b, self.objdump))

    def test_one_byte_vs_two_byte_bad_real(self) -> None:
        a = self._obj(".byte 0x06\nxor %eax, %eax")
        b = self._obj(".byte 0x0f, 0x04\nxor %eax, %eax")
        self.assertFalse(asmdiff.semantically_equal(a, b, self.objdump))

    def test_undecodable_prefix_refuses_real(self) -> None:
        a = self._obj(".byte 0x0f, 0x04\nxor %eax, %eax")
        b = self._obj("xor %eax, %eax")
        self.assertFalse(asmdiff.semantically_equal(a, b, self.objdump))


if __name__ == "__main__":
    unittest.main()
