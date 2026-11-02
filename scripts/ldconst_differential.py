#!/usr/bin/env python3
"""Differential oracle: long-double (`x87` / `long double`) constant emission.

WHY THIS EXISTS
---------------
The emitted bytes of a `long double` constant are the product of a long chain
that never sees a runtime test:

    decimal / hex literal
      -> constant parser
      -> IrConst payload (binary128 on the IR side)
      -> widening  f64 -> binary128
      -> narrowing binary128 -> x87 80-bit
      -> data emission

Every link is pure compile-time arithmetic, so a bug there is *silent*: the
program links, runs, and prints a value that is off by 1 ULP or by a factor of
2^52. The unit tests only cover the hand-picked values someone thought of.

This script replaces "someone thought of it" with a differential experiment:
emit N constants with LCCC and with a reference compiler (GCC by default,
Clang/ICX opt-in), then compare the emitted object bytes element by element.

It found three real defects in one session:

  1. `f64_to_f128_bytes_lossless` re-biased the exponent as
     `biased_exp as u128 - 1023`, which overflows for every `|v| < 1.0`
     (ICE with overflow checks on) and mis-encodes every *subnormal* f64 as if
     it had an implicit integer bit -- off by up to 4.5e14.
  2. `f64_to_x87_bytes_simple` had the same subnormal defect.
  3. `f128_bytes_to_x87_bytes` truncated the 113-bit significand to 64 bits
     (`mantissa >> 49`) instead of rounding, making every constant up to 1 ULP
     low relative to GCC/Clang.

USAGE
-----
    ./scripts/ldconst_differential.py                 # default sweep
    ./scripts/ldconst_differential.py --n 2000        # bigger sweep
    ./scripts/ldconst_differential.py --ref clang     # compare against clang
    ./scripts/ldconst_differential.py --keep          # keep artifacts

Exit status is 0 when every constant agrees, 1 otherwise.

Reference compilers are located on PATH; a missing one is reported as SKIP,
not as a failure, so the script stays useful on minimal research VMs.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

# --------------------------------------------------------------------------
# Deterministic corpus construction
# --------------------------------------------------------------------------


def splitmix64(state: int) -> int:
    """Deterministic PRNG; identical output on every run and every machine."""
    state = (state + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    z = state
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return (z ^ (z >> 31)) & 0xFFFFFFFFFFFFFFFF


def hex_float_literal(mantissa64: int, exp15: int) -> str:
    """Exact C hex-float literal for the x87 value `mantissa64 * 2^(exp15-16383-63)`.

    A hex literal is *exact*: it names the value, so both compilers must round
    it identically. That isolates the encoding/rounding chain from decimal
    string parsing, which is a separate concern (and is covered below).
    """
    unbiased = exp15 - 16383 - 63
    return f"0x{mantissa64:x}p{unbiased}L"


def build_corpus(n: int) -> list[str]:
    """Return a list of C literal strings for `long double` initialisers."""
    lits: list[str] = []

    # --- Structural boundaries: the cases a random sweep walks past ---------
    lits += [
        "0.0L",
        "-0.0L",
        "1.0L",
        "-1.0L",
        "0.5L",                # |v| < 1.0: the u128 underflow class
        "-0.5L",
        "1e-10L",
        "0.1L",
        "3.14159265358979323846L",
        "2.2250738585072014e-308L",   # DBL_MIN
        "5.45247436838069e-309L",     # subnormal as a double (issue #114)
        "1e-320L",                    # 1-ULP rounding witness
        "2.5e-323L",
        "1e300L",
        "1e-300L",
        "1.18973149535723176502e+4932L",   # LDBL_MAX
        "3.36210314311209350626e-4932L",   # LDBL_MIN
    ]

    # --- Exact hex floats: every structural x87 exponent class -------------
    # Smallest/largest normal exponents, the subnormal boundary, and the
    # binary128 subnormal boundary -- then a deterministic random sweep.
    boundary_exps = [1, 2, 3, 0x3FF, 0x3C00, 0x3C01, 0x4000, 0x7FFE, 0x7FFF - 1]
    for e in boundary_exps:
        lits.append(hex_float_literal(0x8000000000000000, e))
        lits.append(hex_float_literal(0xFFFFFFFFFFFFFFFF, e))
        lits.append(hex_float_literal(0x8000000000000001, e))

    state = 0x1234567890ABCDEF
    for _ in range(n):
        state = splitmix64(state)
        exp15 = 1 + (state % 0x7FFE)
        state = splitmix64(state)
        mantissa64 = state | (1 << 63)
        lits.append(hex_float_literal(mantissa64, exp15))

    # --- Decimal literals that need full long-double precision -------------
    # These exercise the decimal parser as well as the encoder, so a mismatch
    # here may be a rounding-mode difference rather than an encoding bug; the
    # report says which group failed.
    state = 0xFEDCBA0987654321
    for _ in range(min(n, 400)):
        state = splitmix64(state)
        exp = (state % 90) - 45
        state = splitmix64(state)
        digits = 1 + (state % 20)
        state = splitmix64(state)
        mant = state % (10**digits)
        lits.append(f"{mant}.{state % 10**7}e{exp}L")

    return lits


# --------------------------------------------------------------------------
# Emission + parsing
# --------------------------------------------------------------------------


def emit(cc: str, src: str, workdir: str, tag: str, extra: list[str]) -> str | None:
    """Compile `src` to assembly; return the assembly text or None on failure."""
    asm = os.path.join(workdir, f"{tag}.s")
    cmd = [cc, "-O2", "-S", "-o", asm, src] + extra
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    except FileNotFoundError:
        return None
    except subprocess.TimeoutExpired:
        return None
    if r.returncode != 0:
        print(f"  [{tag}] {cc} failed: {r.stderr.strip().splitlines()[:3]}")
        return None
    with open(asm) as fh:
        return fh.read()


# Data directives, with their natural widths. GCC emits `.long` for x87 long
# doubles while LCCC emits `.quad`, so a word-oriented comparison would report
# a spurious mismatch on every constant; both are reconstructed into a flat
# little-endian byte stream instead and compared per 16-byte element.
DATA_DIRECTIVES = {
    ".quad": 8,
    ".8byte": 8,
    ".long": 4,
    ".int": 4,
    ".word": 2,
    ".short": 2,
    ".byte": 1,
}


def parse_table(asm: str, symbol: str) -> bytes:
    """Reconstruct the object-representation bytes emitted for `symbol`.

    Handles every data directive width, comma-separated value lists, and
    `.zero N`. Stops at the next label or section directive.
    """
    out = bytearray()
    in_sym = False
    for raw in asm.splitlines():
        line = raw.strip()
        if line.startswith(symbol + ":"):
            in_sym = True
            continue
        if not in_sym:
            continue
        if not line:
            continue
        # A new label or a section/text directive ends this object.
        if re.match(r"^[A-Za-z_.$][A-Za-z0-9_.$]*:", line) or line.startswith(
            (".section", ".text", ".data", ".bss", ".rodata")
        ):
            break
        if line.startswith(".size") or line.startswith(".type") or line.startswith(".ident"):
            continue

        m = re.match(r"^\.zero\s+(\d+)(?:\s*,\s*(\d+))?\s*$", line)
        if m:
            fill = int(m.group(2)) if m.group(2) is not None else 0
            out.extend(bytes([fill]) * int(m.group(1)))
            continue

        m = re.match(r"^(\.[A-Za-z0-9]+)\s+(.*)$", line)
        if not m:
            # Anything unrecognised (an instruction) ends the data object.
            if out:
                break
            continue
        directive, rest = m.group(1), m.group(2)

        if directive not in DATA_DIRECTIVES:
            continue

        width = DATA_DIRECTIVES[directive]
        for tok in rest.split(","):
            tok = tok.strip()
            if not tok:
                continue
            try:
                val = int(tok, 0)
            except ValueError:
                # Symbolic/expression initialiser: cannot compare numerically.
                return b""
            try:
                out.extend(val.to_bytes(width, "little", signed=True))
            except OverflowError:
                out.extend((val & ((1 << (width * 8)) - 1)).to_bytes(width, "little"))
    return bytes(out)


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--n", type=int, default=600, help="random constants per group")
    ap.add_argument("--lccc", default=None, help="path to the lccc binary")
    ap.add_argument("--refs", default="gcc,clang", help="comma-separated reference compilers")
    ap.add_argument("--keep", action="store_true", help="keep the generated sources")
    ap.add_argument("--chunk", type=int, default=250, help="constants per translation unit")
    args = ap.parse_args()

    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    lccc = args.lccc
    if lccc is None:
        for cand in (
            os.path.join(repo, "target", "fastbuild", "lccc"),
            os.path.join(repo, "target", "release", "lccc"),
        ):
            if os.path.exists(cand):
                lccc = cand
                break
    if lccc is None or not os.path.exists(lccc):
        print("error: no lccc binary found (build it or pass --lccc)", file=sys.stderr)
        return 2

    lits = build_corpus(args.n)
    print(f"lccc        : {lccc}")
    print(f"constants   : {len(lits)}")

    workdir = tempfile.mkdtemp(prefix="ldconst-diff-")
    if args.keep:
        print(f"artifacts   : {workdir}")

    failures = 0
    total = 0
    skipped: list[str] = []

    # Chunk so a single bad constant cannot blow up compile time and so a
    # failure localises to a small file.
    for ci in range(0, len(lits), args.chunk):
        chunk = lits[ci : ci + args.chunk]
        src = os.path.join(workdir, f"t{ci}.c")
        with open(src, "w") as fh:
            fh.write("long double tab[] = {\n")
            for lit in chunk:
                fh.write(f"    {lit},\n")
            fh.write("};\n")

        ours = emit(lccc, src, workdir, f"t{ci}.lccc", [])
        if ours is None:
            print(f"  chunk {ci}: lccc failed to compile -- aborting")
            return 1
        ours_bytes = parse_table(ours, "tab")
        if not ours_bytes:
            print(f"  chunk {ci}: could not parse lccc's emission for `tab`")
            failures += len(chunk)
            continue

        for ref in [r.strip() for r in args.refs.split(",") if r.strip()]:
            if shutil.which(ref) is None:
                if ref not in skipped:
                    skipped.append(ref)
                continue
            theirs = emit(ref, src, workdir, f"t{ci}.{ref}", [])
            if theirs is None:
                print(f"  chunk {ci}: {ref} failed to compile")
                failures += 1
                continue
            theirs_bytes = parse_table(theirs, "tab")

            if len(ours_bytes) != len(theirs_bytes):
                print(
                    f"  chunk {ci}: {ref}: byte-count mismatch "
                    f"(lccc={len(ours_bytes)} {ref}={len(theirs_bytes)})"
                )
                failures += len(chunk)
                continue

            # x86-64 `long double` occupies 16 bytes for both toolchains.
            step = len(theirs_bytes) // max(1, len(chunk))
            for i, lit in enumerate(chunk):
                total += 1
                a = ours_bytes[i * step : (i + 1) * step]
                b = theirs_bytes[i * step : (i + 1) * step]
                if a != b:
                    failures += 1
                    if failures <= 25:
                        print(f"  MISMATCH {ref} {lit}")
                        print(f"      lccc: {a.hex()}")
                        print(f"      {ref}: {b.hex()}")

    print()
    print(f"compared    : {total} constants")
    print(f"mismatches  : {failures}")
    if skipped:
        print(f"skipped     : {', '.join(skipped)} (not on PATH)")
    if not args.keep:
        shutil.rmtree(workdir, ignore_errors=True)

    print("VERDICT     : " + ("MATCH" if failures == 0 else "MISMATCH"))
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
