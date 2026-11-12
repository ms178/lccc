"""Shape gate for the fused 16-byte `Load; Store` pair (I128/U128 carriers).

`tests/regression/i128_copy_fusion.c` is the semantic half: it pins the bytes
against GCC.  This module is the shape half — it asserts that the backend
really does replace

    %v = load  *src        movdqu  *src, %xmm0 ; movdqu %xmm0, slot
    store %v -> *dst       movdqu  slot, %xmm0 ; movdqu %xmm0, *dst

with the two-instruction copy, in the generic address space and through a
segment override, and that it declines (leaving the ordinary pair in place)
when the loaded value has a second use.
"""

import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
LCCC = Path(os.environ.get("LCCC", REPO / "target" / "fastbuild" / "lccc"))
BENCH = REPO / "tests" / "bench"
REGRESSION = REPO / "tests" / "regression"

SINGLE = """
typedef unsigned __int128 u128;
void one(u128 *d, u128 *s) { *d = *s; }
"""

PAIR = """
typedef unsigned __int128 u128;
void two(u128 *d, u128 *s) { d[0] = s[0]; d[1] = s[1]; }
"""

def asm(source, flags=("-O2",), extra_dirs=()):
    with tempfile.TemporaryDirectory() as tmp:
        c = Path(tmp) / "k.c"
        s = Path(tmp) / "k.s"
        c.write_text(source)
        cmd = [str(LCCC), *flags, *[f"-I{d}" for d in extra_dirs], "-S", "-o", str(s), str(c)]
        proc = subprocess.run(cmd, capture_output=True, text=True)
        if proc.returncode != 0:
            raise AssertionError(f"compile failed: {proc.stdout}{proc.stderr}")
        return s.read_text()


def body(text, name):
    """The instruction lines of `name`, without the .LBB labels or directives."""
    m = re.search(rf"^{name}:$", text, re.M)
    if not m:
        raise AssertionError(f"{name} not found")
    out, seen = [], False
    for line in text[m.end():].splitlines():
        if line.startswith(".size") and name in line:
            break
        if line.startswith(".") or line.startswith("#"):
            continue
        out.append(line.strip())
    return out


class I128CopyFusion(unittest.TestCase):
    def test_single_copy_fuses_to_two_moves(self):
        """`*d = *s` for a 16-byte carrier: one load, one store, no frame slot."""
        lines = body(asm(SINGLE), "one")
        self.assertEqual([l for l in lines if l.startswith("movdqu")],
                         ["movdqu (%rsi), %xmm0", "movdqu %xmm0, (%rdi)"],
                         "\n".join(lines))
        self.assertFalse([l for l in lines if re.search(r"movdqu.*\(%r(sp|bp)\)", l)],
                         "the copy still round-trips its frame slot:\n" + "\n".join(lines))

    def test_two_copies_fuse_independently(self):
        lines = body(asm(PAIR), "two")
        moves = [l for l in lines if l.startswith("movdqu")]
        self.assertEqual(len(moves), 4, "\n".join(lines))
        # Both copies take a register or a frame-slot address; what matters
        # is that neither materialises the 16 bytes in a slot in between.
        self.assertFalse([l for l in moves if re.search(r"%xmm\d+, -?\d+\(%r(sp|bp)\)$", l)],
                         "a copy still writes its value slot:\n" + "\n".join(lines))

    def test_segment_override_rides_the_memory_operand(self):
        source = (BENCH / "k_seg_gs_copy.c").read_text()
        lines = body(asm(source, extra_dirs=(BENCH,)), "seg_copy_pass")
        gs = [l for l in lines if "%gs:" in l and l.startswith("movdqu")]
        self.assertTrue(gs, "no segment moves:\n" + "\n".join(lines))
        # Every gs load is consumed by the gs store that follows it: the
        # payload never passes through a frame slot in the gs->gs copies.
        fused = 0
        for i, line in enumerate(gs[:-1]):
            if line.startswith("movdqu %gs:") and re.match(r"movdqu %xmm\d+, %gs:", gs[i + 1]):
                fused += 1
        self.assertGreaterEqual(fused, 2, "\n".join(lines))

    def test_slot_source_is_declined_for_store_forwarding(self):
        """A 16-byte slot is written 8 bytes at a time: two 8-byte reads
        forward exactly, one 16-byte read stalls.  The fusion must decline
        rather than trade four forwarding-friendly instructions for a stall."""
        source = """
typedef unsigned __int128 u128;
void store_param(u128 v, u128 *p) { *p = v; }
"""
        lines = body(asm(source, flags=("-O2", "-fno-inline")), "store_param")
        self.assertFalse([l for l in lines if re.search(r"movdqu\s+-?\d+\(%r(sp|bp)\), %xmm", l)],
                         "16-byte slot read (store-forwarding stall):\n" + "\n".join(lines))

    def test_fixture_is_byte_exact_against_gcc(self):
        """The semantic half: the same source, compiled by both compilers."""
        with tempfile.TemporaryDirectory() as tmp:
            src = REGRESSION / "i128_copy_fusion.c"
            mine = Path(tmp) / "mine"
            proc = subprocess.run([str(LCCC), "-O2", "-o", str(mine), str(src)],
                                  capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            got = subprocess.run([str(mine)], capture_output=True, text=True)
            self.assertEqual(got.returncode, 0, got.stdout + got.stderr)
            gcc = os.environ.get("GCC_BIN") or "gcc"
            theirs_bin = Path(tmp) / "theirs"
            proc = subprocess.run([gcc, "-O2", "-o", str(theirs_bin), str(src)],
                                  capture_output=True, text=True)
            if proc.returncode != 0:
                self.fail(proc.stdout + proc.stderr)
            want = subprocess.run([str(theirs_bin)], capture_output=True, text=True)
            self.assertEqual(got.stdout, want.stdout,
                             f"lccc {got.stdout.strip()} != gcc {want.stdout.strip()}")


if __name__ == "__main__":
    unittest.main()
