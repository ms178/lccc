#!/usr/bin/env python3
"""F33 differential fuzz: lccc's _Decimal constant folding vs GCC.

Generates one C file of randomized decimal constants (literals shaped to
stress multi-step rounding, subnormal boundaries, exponent caps, large
coefficients with the exponent MSB set, cross-width conversions that
exercise the decoders, and integer/float sources), compiles it with both
gcc and lccc, and compares every payload. Any divergence fails loud.

Rounding-critical shapes (the bugs this guards):
  - dropped suffixes like 451 / 4999 / 5001 / 450: sequential
    digit-by-digit rounding with a sticky bit double-rounds these
    (1.000000451DF must keep 1000000e-6, not 1000001e-6);
  - subnormal-boundary literals (2.495e-101DF -> coef 2, not 3);
  - large coefficients with biased exp >= 128/512 (9000000e27DF):
    bit 28 (D32) / bit 60 (D64) is the exponent MSB, not steering;
  - exponents overflowing i64 (1e+-999...9): the sign must survive.

Comparison policy (mirrors check_decimal_const_agrees_with_gcc.sh):
  - D32/D64 literals, integer sources, widening conversions: BIT-EXACT.
  - D128 literals, narrowing conversions, float sources: VALUE-EXACT
    (written-quantum policy may differ; value must not).

Usage:
    fuzz_decimal_vs_gcc.py [--seed S] [--cases N] [--keep] [--lccc PATH]
Exit status is 0 iff every case agrees.
"""

import argparse
import random
import re
import subprocess
import sys
import tempfile
from decimal import Decimal
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
LCCC_DEFAULT = REPO_ROOT / "target" / "fastbuild" / "lccc"

# (width, suffix, prec, emin, cap, hard_inf): cap = Emax-(prec-1) is the
# last finite exponent for a full coefficient; hard_inf the first exponent
# that is always infinity for a 1-digit coefficient.
WIDTHS = {
    32: ("DF", 7, -101, 90, 97),
    64: ("DD", 16, -398, 369, 385),
    128: ("DL", 34, -6176, 6111, 6145),
}

# Dropped-suffix adversarial fragments (appended past the precision).
FRAGS = ["5", "45", "451", "450", "449", "4999", "5000", "5001", "5499",
         "5501", "9", "99", "9999999", "00001", "00005", "49999999",
         "50000001", "15", "1500001", "2500000", "75", "25"]


def rand_digits(rng, lo, hi):
    n = rng.randint(lo, hi)
    s = str(rng.randint(1, 9))
    for _ in range(n - 1):
        r = rng.random()
        if r < 0.12:
            s += rng.choice(FRAGS)
        elif r < 0.20:
            s += rng.choice("05")
        else:
            s += str(rng.randint(0, 9))
    return s


def rand_exp(rng, emin, cap, hard_inf):
    r = rng.random()
    if r < 0.30:  # boundary pileups
        base = rng.choice([emin, emin + 1, cap - 1, cap, hard_inf - 2,
                           hard_inf - 1, -2, -1, 0, 1, 2])
        return base + rng.randint(-2, 2)
    if r < 0.45:  # subnormal / underflow band
        return rng.randint(emin - 3, emin + 3)
    if r < 0.55:  # overflow band
        return rng.randint(cap - 2, hard_inf + 1)
    span = hard_inf + 1 - (emin - 3)
    return emin - 3 + rng.randint(0, span)


def lit_case(rng, width, i):
    suffix, prec, emin, cap, hard_inf = WIDTHS[width]
    # Most literals exceed the precision (rounding path); some are short
    # (exact path); a few are extreme-length (O(n) sanity).
    r = rng.random()
    if r < 0.70:
        digits = rand_digits(rng, prec + 1, prec + 25)
    elif r < 0.85:
        digits = rand_digits(rng, 1, prec)
    elif r < 0.95:
        digits = rand_digits(rng, prec + 26, prec + 60)
    else:
        digits = rng.choice(["0", "00", "0" * 40, "5", "50", "500"])
    e = rand_exp(rng, emin, cap, hard_inf)
    neg = rng.random() < 0.3
    # Spell some literals with a decimal point (fractional form) and some
    # with an exponent; both must fold identically.
    if rng.random() < 0.5 and len(digits) > 1:
        pt = rng.randint(1, len(digits) - 1)
        spell = f"{digits[:pt]}.{digits[pt:]}"
        if rng.random() < 0.5:
            spell += f"e{e}"
    else:
        spell = digits
        if e != 0 or rng.random() < 0.7:
            spell += f"e{e}"
        elif "." not in spell:
            # A DF/DD/DL suffix needs a floating constant: spell a bare
            # integer mantissa with a trailing point (`5DF` is rejected;
            # `5.DF` is the battery-proven form).
            spell += "."
    if neg:
        spell = "-" + spell
    ty = {32: "_Decimal32", 64: "_Decimal64", 128: "_Decimal128"}[width]
    return f"{ty} fz{width}_{i} = {spell}{suffix};"


def conv_case(rng, i):
    # Random cross-width conversion, spelled via a temp of the source
    # width (exercises decode of the source + encode of the target,
    # including large coefficients with the exponent MSB set).
    src, dst = rng.choice([(32, 64), (32, 128), (64, 32), (64, 128),
                           (128, 32), (128, 64)])
    suffix, prec, emin, cap, hard_inf = WIDTHS[src]
    digits = rand_digits(rng, 1, prec + 10)
    e = rand_exp(rng, emin, cap, hard_inf)
    tsrc = {32: "_Decimal32", 64: "_Decimal64", 128: "_Decimal128"}[src]
    tdst = {32: "_Decimal32", 64: "_Decimal64", 128: "_Decimal128"}[dst]
    widening = dst > src
    return (f"{tdst} cv{i} = ({tdst})(({tsrc})({digits}e{e}{suffix}));",
            widening)


def int_case(rng, i):
    width = rng.choice([32, 64, 128])
    ty = {32: "_Decimal32", 64: "_Decimal64", 128: "_Decimal128"}[width]
    r = rng.random()
    if r < 0.4:
        v = rng.randint(0, 2**64 - 1)
        spell = f"{v}ULL"
    elif r < 0.6:
        v = rng.randint(-2**63, 2**63 - 1)
        spell = f"{v}LL"
    elif r < 0.8:
        # __int128-range integer, spelled as a shift-or expression:
        # unsuffixed constants above 2^64-1 are diagnosed (lccc errors,
        # gcc warns), so build the value from ULL parts instead -- both
        # compilers const-fold this exactly and silently.
        v = rng.randint(10**18, 2**128 - 1)
        spell = (f"((unsigned __int128){v >> 64}ULL << 64 | "
                 f"{v & (2**64 - 1)}ULL)")
    else:
        v = rng.choice([2**53 - 1, 2**53, 2**53 + 1, 2**64 - 1, 2**64,
                        10**16 - 1, 10**16, 10**34 - 1, 0, 5, 2**23 - 1,
                        2**23, 2**23 + 1, 9999999, 10**7])
        if v > 2**64 - 1:
            spell = (f"((unsigned __int128){v >> 64}ULL << 64 | "
                     f"{v & (2**64 - 1)}ULL)")
        elif v > 2**63 - 1:
            spell = f"{v}ULL"
        else:
            spell = str(v)
    return f"{ty} iz{i} = {spell};"


def float_case(rng, i):
    width = rng.choice([32, 64])
    ty = {32: "_Decimal32", 64: "_Decimal64"}[width]
    r = rng.random()
    if r < 0.5:
        v = rng.choice(["0.5", "0.1", "1.5", "30.0", "0.0", "-0.0",
                        "123.456", "1e300", "5e-324", "2.5e-7"])
    else:
        v = repr(rng.uniform(-1e6, 1e6))
    return f"{ty} fl{i} = {v};"


def dec32(v):
    s = (v >> 31) & 1
    if (v & 0x78000000) == 0x78000000:
        return (s, None, None)
    if (v & 0x60000000) == 0x60000000:
        return ((-1) ** s, 2**23 + (v & 0x1FFFFF), ((v >> 21) & 0xFF) - 101)
    return ((-1) ** s, v & 0x7FFFFF, ((v >> 23) & 0xFF) - 101)


def dec64(v):
    s = (v >> 63) & 1
    if (v & 0x7800000000000000) == 0x7800000000000000:
        return (s, None, None)
    if (v & 0x6000000000000000) == 0x6000000000000000:
        return ((-1) ** s, 2**53 + (v & 0x7FFFFFFFFFFFF),
                ((v >> 51) & 0x3FF) - 398)
    return ((-1) ** s, v & 0x1FFFFFFFFFFFFF, ((v >> 53) & 0x3FF) - 398)


def dec128(hi, lo):
    s = (hi >> 63) & 1
    if (hi & 0x7800000000000000) == 0x7800000000000000:
        return (s, None, None)
    return ((-1) ** s, ((hi & 0x1FFFFFFFFFFFF) << 64) | lo,
            ((hi >> 49) & 0x3FFF) - 6176)


def num(t):
    s, c, e = t
    if c is None:  # special: compare the (sign, kind) pair instead
        return (s, e)
    return Decimal(s) * Decimal(c) * (Decimal(10) ** e)


def grab(txt, name):
    m = re.search(rf"^{name}:\n((?:[ \t]+\.(?:long|quad)[ \t]+[^\n]+\n)+)",
                  txt, re.M)
    assert m, name
    kinds = re.findall(r"\.(long|quad)[ \t]+", m.group(1))
    vals = [int(x, 0) & 0xFFFFFFFFFFFFFFFF for x in
            re.findall(r"\.(?:long|quad)[ \t]+(-?(?:\d+|0x[0-9a-fA-F]+))",
                       m.group(1))]
    return kinds, vals


def payload(kinds, vals, width):
    if width == 32:
        return vals[0] & 0xFFFFFFFF
    if width == 64:
        if len(vals) > 1:
            return ((vals[1] & 0xFFFFFFFF) << 32) | (vals[0] & 0xFFFFFFFF)
        return vals[0]
    if len(vals) >= 4:  # gcc spells _Decimal128 as 4 .longs
        hi = ((vals[3] & 0xFFFFFFFF) << 32) | (vals[2] & 0xFFFFFFFF)
        lo = ((vals[1] & 0xFFFFFFFF) << 32) | (vals[0] & 0xFFFFFFFF)
        return (hi, lo)
    return (vals[1], vals[0])


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--cases", type=int, default=300,
                    help="literal cases per width (default 300)")
    ap.add_argument("--lccc", default=str(LCCC_DEFAULT))
    ap.add_argument("--keep", action="store_true",
                    help="keep the generated .c file in $PWD")
    args = ap.parse_args()
    rng = random.Random(args.seed)

    lines, plan = [], []  # plan: (name, width, bit_exact)
    n = args.cases
    for width in (32, 64, 128):
        count = n if width != 128 else max(20, n // 5)
        for i in range(count):
            stmt = lit_case(rng, width, i)
            name = re.search(r"(\w+) =", stmt).group(1)
            lines.append(stmt)
            plan.append((name, width, width != 128))
    for i in range(max(40, n // 2)):
        stmt, widening = conv_case(rng, i)
        name = re.search(r"(\w+) =", stmt).group(1)
        lines.append(stmt)
        width = 32 if stmt.startswith("_Decimal32") else (
            64 if stmt.startswith("_Decimal64") else 128)
        plan.append((name, width, widening and width != 128))
    for i in range(max(20, n // 5)):
        stmt = int_case(rng, i)
        name = re.search(r"(\w+) =", stmt).group(1)
        lines.append(stmt)
        width = 32 if stmt.startswith("_Decimal32") else (
            64 if stmt.startswith("_Decimal64") else 128)
        plan.append((name, width, width != 128))
    for i in range(max(20, n // 5)):
        stmt = float_case(rng, i)
        name = re.search(r"(\w+) =", stmt).group(1)
        lines.append(stmt)
        width = 32 if stmt.startswith("_Decimal32") else 64
        plan.append((name, width, False))  # quantum policy may differ

    with tempfile.TemporaryDirectory(prefix="decfuzz") as td:
        src = Path(td) / "fuzz.c"
        src.write_text("\n".join(lines) + "\n")
        if args.keep:
            Path(f"decfuzz-seed{args.seed}.c").write_text(src.read_text())
        gcc_s = Path(td) / "gcc.s"
        lccc_s = Path(td) / "lccc.s"
        r = subprocess.run(["gcc", "-O2", "-S", "-o", str(gcc_s), str(src)],
                           capture_output=True, text=True, timeout=300)
        if r.returncode != 0:
            print("gcc failed:\n" + r.stderr[-3000:])
            return 2
        r = subprocess.run([args.lccc, "-O2", "-S", "-o", str(lccc_s),
                            str(src)], capture_output=True, text=True,
                           timeout=900)
        if r.returncode != 0:
            print("lccc failed:\n" + r.stderr[-3000:])
            return 2
        g, l = gcc_s.read_text(), lccc_s.read_text()

    bad, shown = 0, 0
    for name, width, bit_exact in plan:
        gk, gv = grab(g, name)
        lk, lv = grab(l, name)
        gp, lp = payload(gk, gv, width), payload(lk, lv, width)
        if bit_exact:
            same = gp == lp
        elif width == 128:
            same = num(dec128(*gp)) == num(dec128(*lp))
        elif width == 64:
            same = num(dec64(gp)) == num(dec64(lp))
        else:
            same = num(dec32(gp)) == num(dec32(lp))
        if not same:
            bad += 1
            if shown < 20:
                shown += 1
                print(f"DIVERGE {name} (w{width}): gcc={gp} lccc={lp}")
    total = len(plan)
    print(f"seed={args.seed} cases={total} divergences={bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
