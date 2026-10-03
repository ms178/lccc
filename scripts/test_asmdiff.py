#!/usr/bin/env python3
"""Unit tests for asmdiff's semantic-equality oracle.

`allow_better` groups accept a smaller LCCC encoding only when the pinned
disassembler proves both objects decode to the same instruction sequence,
so `semantically_equal` is verdict machinery: every way it can be fed
non-evidence — a failed disassembler, an empty listing, an undecodable
instruction, a `...` gap of undecoded bytes, an orphan byte fragment —
must land on False, never on a vacuous equality. These tests pin that
contract directly, with mocked `subprocess.run` for the failure modes
that are hard to produce on demand, and a real-toolchain leg that anchors
the parser to genuine objdump output.
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
# Verified against the pinned binutils 2.47 (and 2.44) on x86 in BOTH
# modes: the byte column is a fixed 21-character field — three characters
# per byte ("XX "), space-padded when an instruction is shorter, exactly
# full (7 bytes) when the instruction wraps — and continuation lines carry
# the RUNNING address (previous address + previous bytes) and no
# instruction column. The helpers below build exactly that shape;
# hand-typed padding can drift from the real column width, which the
# stateful parser rejects by design.

def _col(byts: str) -> str:
    """One padded 21-character anchor byte column."""
    return (" ".join(byts.split()) + " ").ljust(21)


def _a(addr: int, byts: str, insn: str) -> str:
    """One anchor line: address, padded byte column, instruction."""
    return f"{addr:>4x}:\t{_col(byts)}\t{insn}\n"


def _c(addr: int, byts: str) -> str:
    """One continuation line: running address, unpadded bytes, no column."""
    return f"{addr:>4x}:\t{' '.join(byts.split())} \n"


def _listing(lines: str) -> str:
    return ("a.o:     file format elf64-x86-64\n"
            "Disassembly of section .text:\n"
            "\n"
            "0000000000000000 <.text>:\n") + lines


# Two different encodings of the same program: the second mov uses a
# redundant SIB scale-1 index, the first does not (same effective address).
EQUAL_A = _listing(
    _a(0, "31 c0", "xor    %eax,%eax") +
    _a(2, "48 8b 45 04", "mov    -0x1(,%rdi,1),%rax"))
EQUAL_B = _listing(
    _a(0, "31 c0", "xor    %eax,%eax") +
    _a(2, "48 8b 47 ff", "mov    -0x1(%rdi),%rax"))

# Genuinely different programs (operand differs).
DIFFERENT_B = _listing(
    _a(0, "31 c0", "xor    %eax,%eax") +
    _a(2, "48 8b 47 ff", "mov    -0x1(%rsi),%rax"))

# Undecodable renderings: a ONE-byte `(bad)' and a TWO-byte `(bad)'.
# objdump prints the bytes, but the comparison drops the byte column, so
# under the pre-audit oracle these two different garbage streams compared
# EQUAL ("(bad)" == "(bad)") — the exact false-accept this suite pins shut.
BAD_1BYTE = _listing(
    _a(0, "06", "(bad)") +
    _a(1, "31 c0", "xor    %eax,%eax"))
BAD_2BYTE = _listing(
    _a(0, "0f 04", "(bad)") +
    _a(1, "31 c0", "xor    %eax,%eax"))

# A `.byte' fallback rendering (defensive: x86 `-d' prints `(bad)', but
# the refusal must not depend on which of the two spellings appears).
BYTE_FALLBACK = _listing(
    _a(0, "90", ".byte   0x90") +
    _a(1, "31 c0", "xor    %eax,%eax"))

# Identical decoded prefixes, but a `...' gap of bytes the decoder
# declined to decode at the end: the undecoded tails could differ.
GAP_TAIL = _listing(
    _a(0, "31 c0", "xor    %eax,%eax") +
    "\t...\n")

# A branch-target comment and a {vex} annotation are not semantics.
ANNOTATED = _listing(
    _a(0, "74 02", "je     4 <.text+0x4>") +
    _a(2, "62 f1 7d 08 70 c1", "{vex} vpshufd $0x1,%xmm1,%xmm0"))

# A real 2.47 wrap, byte-for-byte as the pinned objdump prints it
# (`addq $imm32, disp32(%rbx,%rcx,8)' = 12 bytes: 7 + 5).
WRAP_7_5 = _listing(
    _a(0, "48 81 84 cb 88 77 66",
       "addq   $0x11223344,0x55667788(%rbx,%rcx,8)") +
    _c(7, "55 44 33 22 11") +
    _a(12, "62 f1 fd 48 6f 05 78",
       "vmovdqa64 0x12345678(%rip),%zmm0") +
    _c(19, "56 34 12") +
    _a(22, "90", "nop"))

# A triple wrap: a full 15-byte instruction (7 + 7 + 1).
WRAP_7_7_1 = _listing(
    _a(0, "62 f1 fd 48 6f 05 22",
       "vmovdqa64 0x11223344(%rip),%zmm0") +
    _c(7, "33 44 55 66 77 88 99") +
    _c(14, "aa") +
    _a(15, "90", "nop"))

# THE audit repro pair: identical decoded text, differing only in an
# ORPHAN byte-only line after a one-byte nop whose byte column was NOT
# full — objdump wraps only on overflow, so these are malformed streams.
ORPHAN_A = _listing(_a(0, "90", "nop") + _c(1, "ff"))
ORPHAN_B = _listing(_a(0, "90", "nop") + _c(1, "ee"))


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




# ── The real-toolchain leg's tool resolution ────────────────────────────────────────────────────────
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
#
# EXPLICIT channels are ALL-OR-NOTHING: a partially-set or invalid pair is
# a wiring error, and completing it silently from PATH would paper over
# exactly the misconfiguration the channels exist to make visible (a
# pinned `as' arbitrating against an ambient distro objdump — a mixed
# oracle pair is what the whole pin regime refuses). Explicit-but-broken
# therefore NEVER falls back; it reports the broken channel and the leg
# skips (bare) or errors (require mode).
def _resolve_leg_tools() -> tuple[str | None, str | None, str]:
    """(as, objdump, problem) for the real-toolchain leg."""
    require = os.environ.get("LCCC_REQUIRE_PINNED_ORACLE") == "1"
    as_path = os.environ.get("ASMDIFF_TEST_AS") or ""
    od_path = os.environ.get("ASMDIFF_TEST_OBJDUMP") or ""
    if as_path or od_path:
        # Explicit configuration: every channel must be present AND valid,
        # whatever the mode — the counterpart is never taken from PATH.
        for role, path in (("as", as_path), ("objdump", od_path)):
            if not path:
                # This channel is the missing one; its counterpart is set
                # (this branch is only reachable when at least one is).
                set_one = "ASMDIFF_TEST_AS" if role == "objdump" \
                    else "ASMDIFF_TEST_OBJDUMP"
                return None, None, (
                    f"{set_one} is set but ASMDIFF_TEST_{role.upper()} is "
                    "not — explicit test channels are all-or-nothing; set "
                    "both (the provisioned 2.47 pair) or neither (PATH "
                    "fallback). Refusing to mix a pinned tool with an "
                    "ambient one.")
            if not (Path(path).is_file() and os.access(path, os.X_OK)):
                return None, None, (
                    f"ASMDIFF_TEST_{role.upper()}={path!r} is not an "
                    "executable file — an explicit channel is never "
                    "completed from PATH; fix the path (or unset the "
                    "channels for a bare PATH run).")
        return as_path, od_path, ""
    if require:
        return None, None, (
            "LCCC_REQUIRE_PINNED_ORACLE=1 but ASMDIFF_TEST_AS and "
            "ASMDIFF_TEST_OBJDUMP are both unset — the leg must run "
            "against the pinned 2.47 pair, not skip")
    # Bare human run, no explicit channels: PATH is the fallback.
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
                + _a(0, "31 c0", "xor    %eax,%eax"))
        self.assertEqual(asmdiff._parse_disasm(text), ["xor %eax,%eax"])
        self.assertIsNone(asmdiff._parse_disasm(
            "a.o:     file format elf64-x86-64\n"
            "Disassembly of section .text:\n"
            "0000000000000000 <.text>:\n"))

    def test_empty_instruction_stream_is_not_evidence(self) -> None:
        self.assertIsNone(asmdiff._parse_disasm(""))
        self.assertIsNone(asmdiff._parse_disasm("a.o: file format elf64-x86-64\n"))

    def test_undecodable_renderings_refuse_the_whole_stream(self) -> None:
        for lines in (_a(0, "06", "(bad)"),
                      _a(0, "0f 04", "(bad)"),
                      _a(0, "90", ".byte   0x90"),
                      _a(0, "90", "(bad) [xmm]")):
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

    def test_valid_continuations_follow_the_real_objdump_grammar(self) -> None:
        # A byte column wider than one line wraps onto continuation lines
        # that carry the RUNNING address and no instruction column (real
        # binutils 2.44/2.47 form). Every wrap below is admissible: the
        # anchor filled the column, each fragment sits at the running
        # address, and the accumulated instruction stays within the
        # 15-byte architectural maximum. The fragment produces no record
        # of its own — the anchor line owns the instruction.
        self.assertEqual(
            asmdiff._parse_disasm(_listing(
                _a(0, "48 8b 04 3d ff ff ff",
                   "mov    -0x1(,%rdi,1),%rax") + _c(7, "ff"))),
            ["mov -0x1(%rdi),%rax"])
        self.assertEqual(
            asmdiff._parse_disasm(WRAP_7_5),
            ["addq $0x11223344,0x55667788(%rbx,%rcx,8)",
             "vmovdqa64 0x12345678(%rip),%zmm0",
             "nop"])
        self.assertEqual(
            asmdiff._parse_disasm(WRAP_7_7_1),
            ["vmovdqa64 0x11223344(%rip),%zmm0", "nop"])

    def test_orphan_byte_fragments_refuse_the_stream(self) -> None:
        # THE audit regression: two listings that differ only in an orphan
        # byte-only line must BOTH be inadmissible — the pre-stateful
        # parser skipped such lines silently, so both parsed to ['nop']
        # and a size-only BETTER could stand on streams never proven
        # identical. objdump wraps ONLY when the anchor's byte column
        # overflowed, so a fragment after a one-byte nop is malformed.
        self.assertIsNone(asmdiff._parse_disasm(ORPHAN_A))
        self.assertIsNone(asmdiff._parse_disasm(ORPHAN_B))
        for text in (
            # orphan at stream start (no anchor to continue):
            _c(0, "ff") + _a(1, "90", "nop"),
            # orphan right after a symbol label (chain was reset):
            _a(0, "90", "nop") + "0000000000000008 <next>:\n" + _c(8, "ff"),
            # orphan at a NON-running address (chain break):
            _a(0, "48 8b 04 3d ff ff ff", "mov    -0x1(,%rdi,1),%rax")
            + _c(9, "ff"),
            # a continuation that itself did not fill the column may not
            # be continued again (objdump wraps only on overflow):
            _a(0, "48 8b 04 3d ff ff ff", "mov    -0x1(,%rdi,1),%rax")
            + _c(7, "ff") + _c(8, "ff"),
            # accumulated instruction exceeds the 15-byte maximum:
            _a(0, "62 f1 fd 48 6f 05 22",
               "vmovdqa64 0x11223344(%rip),%zmm0")
            + _c(7, "33 44 55 66 77 88 99") + _c(14, "aa bb cc") + _c(17, "dd"),
            # prose in a byte-only line is not bytes (objdump never
            # emits prose there) — and never a silent skip:
            _a(0, "48 8b 04 3d ff ff ff", "mov    -0x1(,%rdi,1),%rax")
            + _c(7, "ff zz"),
            # an address-chain break between two anchors in one block:
            _a(0, "31 c0", "xor    %eax,%eax") + _a(3, "90", "nop"),
            # an anchor byte column inconsistent with the listing's width:
            _a(0, "31 c0", "xor    %eax,%eax")
            + "   2:\t48 8b 47 ff             \tmov    -0x1(%rdi),%rax\n",
        ):
            with self.subTest(text=text[:40]):
                self.assertIsNone(asmdiff._parse_disasm(_listing(text)))

    def test_normalisation_chain(self) -> None:
        # Branch-target comments, {vex}/{evex} annotations, scale-1
        # indexes and explicit zero displacements are encoding spelling,
        # not semantics; each is unified before comparison.
        self.assertEqual(
            asmdiff._parse_disasm(ANNOTATED),
            ["je 4 <.text+0x4>", "vpshufd $0x1,%xmm1,%xmm0"])
        zero_disp = _listing(_a(0, "48 8b 45 00", "mov    0x0(%rax),%rbx"))
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

    def test_orphan_fragments_never_compare_equal(self) -> None:
        # THE audit false-pass through the public oracle entry point:
        # both listings carry an orphan byte-only line, the streams differ
        # in exactly that byte, and equality must be refused — the old
        # parser silently skipped orphans and compared them equal.
        result, _ = self._equal(ORPHAN_A, ORPHAN_B)
        self.assertFalse(result)

    def test_real_wraps_compare_equal_to_their_unwrapped_form(self) -> None:
        # Non-vacuity of the continuation grammar: a genuinely wrapped
        # instruction listing still parses and compares equal to itself
        # through the public entry point (the state machine admits every
        # real 2.47 wrap shape — see the fixtures above).
        result, _ = self._equal(WRAP_7_5, WRAP_7_5)
        self.assertTrue(result)


class ResolverTests(unittest.TestCase):
    """The explicit test channels are all-or-nothing and never fall back.

    A partially-set or invalid pair is a wiring error; the pre-hardening
    resolver silently completed it from ambient PATH, which is exactly the
    mixed-oracle-pair defect the channels exist to prevent (a pinned `as'
    arbitrating against a distro objdump). Every matrix cell is pinned,
    including the DIAGNOSTIC POLARITY of the partial-config message (an
    inverted "X is set but Y is not" is worse than none: it sends the
    reader to fix the wrong variable).
    """

    # Two real executables INDEPENDENT of the ambient channels: capturing
    # os.environ here would poison the "valid pair" cells the moment a
    # human runs the suite with a broken ASMDIFF_TEST_* exported. The
    # resolver checks existence+executability, not identity, and the
    # Python interpreter is an executable every run of this suite has.
    GOOD_AS = sys.executable
    GOOD_OD = sys.executable

    def _resolve(self, **env):
        base = {k: v for k, v in os.environ.items()
                if k not in ("ASMDIFF_TEST_AS", "ASMDIFF_TEST_OBJDUMP",
                             "LCCC_REQUIRE_PINNED_ORACLE")}
        base.update(env)
        with mock.patch.dict(os.environ, base, clear=True):
            return _resolve_leg_tools()

    def test_partial_config_never_falls_back_to_path(self) -> None:
        # Only AS set: the problem names the MISSING channel (OD), and no
        # tool is resolved — PATH never completes an explicit pair.
        a, o, problem = self._resolve(ASMDIFF_TEST_AS=self.GOOD_AS)
        self.assertIsNone(a)
        self.assertIsNone(o)
        self.assertIn("ASMDIFF_TEST_AS is set but ASMDIFF_TEST_OBJDUMP is not", problem)
        # Only OD set: polarity flips with the situation.
        a, o, problem = self._resolve(ASMDIFF_TEST_OBJDUMP=self.GOOD_OD)
        self.assertIsNone(a)
        self.assertIsNone(o)
        self.assertIn("ASMDIFF_TEST_OBJDUMP is set but ASMDIFF_TEST_AS is not", problem)

    def test_invalid_explicit_channel_never_falls_back(self) -> None:
        for broken_as, broken_od in (("/nonexistent/as", self.GOOD_OD),
                                     (self.GOOD_AS, "/nonexistent/od")):
            with self.subTest(as_path=broken_as, od_path=broken_od):
                a, o, problem = self._resolve(
                    ASMDIFF_TEST_AS=broken_as, ASMDIFF_TEST_OBJDUMP=broken_od)
                self.assertIsNone(a)
                self.assertIsNone(o)
                self.assertIn("is not an executable file", problem)

    def test_valid_pair_resolves_in_both_modes(self) -> None:
        for extra in ({}, {"LCCC_REQUIRE_PINNED_ORACLE": "1"}):
            with self.subTest(mode=extra or "bare"):
                a, o, problem = self._resolve(
                    ASMDIFF_TEST_AS=self.GOOD_AS,
                    ASMDIFF_TEST_OBJDUMP=self.GOOD_OD, **extra)
                self.assertEqual((a, o, problem),
                                 (self.GOOD_AS, self.GOOD_OD, ""))

    def test_require_mode_without_channels_is_an_error(self) -> None:
        a, o, problem = self._resolve(LCCC_REQUIRE_PINNED_ORACLE="1")
        self.assertIsNone(a)
        self.assertIsNone(o)
        self.assertIn("LCCC_REQUIRE_PINNED_ORACLE=1", problem)
        self.assertIn("both unset", problem)

    def test_bare_mode_without_channels_uses_path(self) -> None:
        # No channels, no require flag: the suite must stay runnable on
        # any box — either the PATH pair or the documented skip message.
        a, o, problem = self._resolve()
        if a is None:
            self.assertIn("PATH", problem)
        else:
            self.assertEqual(problem, "")


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

    def test_long_instruction_wrap_is_evidence_real(self) -> None:
        # A real >7-byte instruction wraps onto continuation lines in the
        # genuine objdump listing; the stateful grammar must admit it as
        # semantic evidence (and equal to itself) — never reject the real
        # format the pinned oracle actually prints.
        a = self._obj("addq $0x11223344, 0x55667788(%rbx,%rcx,8)\n"
                      "vmovdqa64 0x12345678(%rip), %zmm0")
        self.assertIsNotNone(asmdiff._disasm_stream(a, self.objdump))
        self.assertTrue(asmdiff.semantically_equal(a, a, self.objdump))


if __name__ == "__main__":
    unittest.main()
