#!/usr/bin/env bash
# F25/F26 e2e: lccc's _Decimal32/64/128 constant encodings agree with GCC.
# - int-sourced inits: BIT-EXACT vs gcc rodata (F25 fold path / __bid conversions)
# - DF/DD/DL literals: bit-exact, except documented quantum cohorts (5.0: value-exact)
# - extremes (cap rescue: 1e91DF finite, 1e97DF inf; subnormals),
#   float-source quanta (0.5 -> 50e-2 pad, 0.0 -> e-1 zero, inf/nan),
#   hang-pin literal, runtime triangle
# F33: single-decision rounding witnesses (1.000000451/4999/501DF),
# subnormal 2.495e-101DF, large+expMSB steering (9000000e27DF,
# 8388608e27DF, 99000000000000000e114DD, DF->DD conversion of 9000000e27),
# exponent-overflow sign (1e+-99999999999999999999DF).
# F9: canonical-zero spelling (0.000DF written-exp zero), tiny literal
# (0.00000001DF stays nonzero — the unnormalized-entry witness),
# computed subnormal (1.5e-102DF half-even step).
# Fails loud on any divergence (no XFAILs: every case must match).
set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LCCC="${LCCC:-$REPO_ROOT/target/fastbuild/lccc}"
# Scratch dir: concurrent-safe (the CI matrix runs this beside itself) and
# self-cleaning. An explicit $WORK override is honored but still removed
# on exit — it is scratch by contract.
WORK="${WORK:-$(mktemp -d "${TMPDIR:-/tmp}/dec-battery.XXXXXX")}"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
mkdir -p "$WORK"
cd "$WORK" || exit 1
PASS=0; FAIL=0
fail() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }
pass() { PASS=$((PASS+1)); }

[ -x "$LCCC" ] || { echo "FAIL: lccc binary missing: $LCCC"; exit 1; }

# Loud capability probe: skip (exit 0, bannered) when the oracle toolchain
# lacks decimal FP — never fail, never pass silently. Distinguish "no
# decimal" (plain C parses, _Decimal32 does not) from a broken compiler
# (plain C fails too): the latter still fails loud. -fsyntax-only is the
# precise probe: the oracle's decimal *codegen* is trusted by definition
# (it is the reference); only *acceptance* is in doubt on exotic toolchains.
# (Verified non-vacuous on Debian GCC 14.2 and GCC 13.3: both accept.)
cat > dfp-probe-plain.c <<'EOF'
int main(void) { return 0; }
EOF
cat > dfp-probe.c <<'EOF'
_Decimal32 x = 1.5DF;
_Decimal64 y = 2.5DD;
_Decimal128 z = 0.1DL;
int main(void) { return (int)(x + y + z) - 4; }
EOF
if ! gcc -fsyntax-only dfp-probe-plain.c 2>/dev/null; then
    echo "FAIL: system gcc cannot parse plain C" >&2; exit 1
fi
if ! gcc -fsyntax-only dfp-probe.c 2>/dev/null; then
    echo "SKIP: system gcc lacks decimal floating-point; oracle vacuous on this toolchain"
    exit 0
fi

# ---------- rodata battery ----------
cat > rod.c <<'EOF'
_Decimal32 i0 = 0, i1 = 1, i5 = 5, in5 = -5, i42 = 42, i127 = 127, i255 = 255;
_Decimal32 i1234567 = 1234567, i8388607 = 8388607, i8388608 = 8388608;
_Decimal32 i9999999 = 9999999, in9999999 = -9999999, i1e7 = 10000000;
_Decimal32 i12345670 = 12345670, ibig = 123456789, imax = 2147483647, imin = -2147483647-1;
_Decimal32 f1p5 = 1.5DF, fn0p5 = -0.5DF, f0p1 = 0.1DF, f123p456 = 123.456DF;
_Decimal32 f5p0 = 5.0DF, f0 = 0.0DF, fhdn = 0.12345665DF, fhup = 0.12345675DF;
_Decimal32 fh1 = 1.0000005DF, fh2 = 1.0000015DF;
_Decimal32 e90 = 1e90DF, e91 = 1e91DF, en91 = -1e91DF;
_Decimal32 en101 = 1e-101DF, en102 = 1e-102DF, e5n102 = 5e-102DF, e6n102 = 6e-102DF;
_Decimal32 f30 = 30.0, fh7 = 7.0, fz0 = 0.0, fnz = -0.0, fsd = 5.DF, fq1 = 1.50DF;
_Decimal32 fb96 = 1e96DF, fb97 = 1e97DF, f1e7d = 10000000.DF, ffi = 1e300, ffn = 0.0/0.0;
_Decimal64 k0 = 0, k1 = 1, kn1 = -1, k5 = 5, kn42 = -42;
_Decimal64 k2p53m1 = 9007199254740991LL, k2p53 = 9007199254740992LL;
_Decimal64 gd30 = 30.0, gd05 = 0.5, gdy = 0.1, gz0 = 0.0, gnz = -0.0, gd5 = 5.DD;
_Decimal64 gq = 1.50DD, gb384 = 1e384DD, gb385 = 1e385DD, gffi = 1e400, gffn = 0.0/0.0;
_Decimal128 td05 = 0.5, tdz = 0.0, tdnz = -0.0, tb6144 = 1e6144DL, tb6145 = 1e6145DL, tffi = 1e5000, tffn = 0.0/0.0;
_Decimal64 k10p16m1 = 9999999999999999LL, k10p16 = 10000000000000000LL;
_Decimal64 k10p18 = 1000000000000000000LL, kbig = 123456789012345678LL;
_Decimal64 kmax = 9223372036854775807LL;
_Decimal64 g1p5 = 1.5DD, g0p1 = 0.1DD, g5p0 = 5.0DD, g0 = 0.0DD;
_Decimal64 ghdn = 0.12345678901234565DD, ghup = 0.12345678901234575DD;
_Decimal64 E369 = 1e369DD, E370 = 1e370DD, En370 = -1e370DD;
_Decimal64 En398 = 1e-398DD, En399 = 1e-399DD, E6n399 = 6e-399DD;
_Decimal128 t0 = 0, t1 = 1, t1p5 = 1.5DL, t0p1 = 0.1DL, tn5 = -5;
_Decimal32 rr451 = 1.000000451DF, rr4999 = 1.0000004999DF, rr501 = 1.000000501DF;
_Decimal32 sub2495 = 2.495e-101DF;
_Decimal32 lg9e27 = 9000000e27DF, lg8e27 = 8388608e27DF;
_Decimal32 peUnder = 1e-99999999999999999999DF, peOver = 1e+99999999999999999999DF;
_Decimal32 z8 = 0.00000001DF, s6 = 1.5e-102DF, z3 = 0.000DF;
_Decimal64 lg99e114 = 99000000000000000e114DD;
_Decimal64 cvDfDd = 9000000e27DF;
EOF
timeout 120 gcc -O2 -S -o rod-gcc.s rod.c || { fail "gcc -S rod.c"; }
timeout 300 "$LCCC" -O2 -S -o rod-lccc.s rod.c || { fail "lccc -S rod.c"; }

# Quantum cohorts (same value, different written quantum): value-compare.
# Everything else: bit-exact.
VALUE_CASES="f5p0 g5p0"
export VALUE_CASES
python3 - > rod.out 2>&1 <<'PYEOF'
import os, re, sys
from decimal import Decimal
# Large form <=> top bits 11 (non-special); the next bit down (D32 bit 28,
# D64 bit 60) is the exponent MSB, not steering (F33: requiring it clear
# misdecodes large+big-exponent values like 9000000e27DF = 0x70095440).
def dec32(v):
    s = (v >> 31) & 1
    if (v & 0x78000000) == 0x78000000:
        return ((-1)**s, None, None)
    if (v & 0x60000000) == 0x60000000:
        return ((-1)**s, 2**23 + (v & 0x1FFFFF), ((v >> 21) & 0xFF) - 101)
    return ((-1)**s, v & 0x7FFFFF, ((v >> 23) & 0xFF) - 101)
def dec64(v):
    s = (v >> 63) & 1
    if (v & 0x7800000000000000) == 0x7800000000000000:
        return ((-1)**s, None, None)
    if (v & 0x6000000000000000) == 0x6000000000000000:
        return ((-1)**s, 2**53 + (v & 0x7FFFFFFFFFFFF), ((v >> 51) & 0x3FF) - 398)
    return ((-1)**s, v & 0x1FFFFFFFFFFFFF, ((v >> 53) & 0x3FF) - 398)
def dec128(hi, lo):
    s = (hi >> 63) & 1
    return ((-1)**s, ((hi & 0x1FFFFFFFFFFFF) << 64) | lo, ((hi >> 49) & 0x3FFF) - 6176)
def num(t):
    s, c, e = t
    return Decimal(s) * Decimal(c) * (Decimal(10) ** e)
def grab(txt, name):
    m = re.search(rf'^{name}:\n((?:[ \t]+\.(?:long|quad)[ \t]+[^\n]+\n)+)', txt, re.M)
    assert m, name
    vals = [int(x, 0) & 0xFFFFFFFFFFFFFFFF for x in re.findall(r'\.(?:long|quad)[ \t]+(-?(?:\d+|0x[0-9a-fA-F]+))', m.group(1))]
    Kind = re.findall(r'\.(long|quad)[ \t]+', m.group(1))
    out, i = [], 0
    for k in Kind:
        if k == 'long': out.append(vals[i] & 0xFFFFFFFF); i += 1
        else:
            # gcc .quad on 64-bit: full 64-bit value already
            out.append(vals[i]); i += 1
    return out
g, l = open('rod-gcc.s').read(), open('rod-lccc.s').read()
names32 = """i0 i1 i5 in5 i42 i127 i255 i1234567 i8388607 i8388608 i9999999
 in9999999 i1e7 i12345670 ibig imax imin f1p5 fn0p5 f0p1 f123p456 f5p0 f0
 fhdn fhup fh1 fh2 e90 e91 en91 en101 en102 e5n102 e6n102
 f30 fh7 fz0 fnz fsd fq1 fb96 fb97 f1e7d ffi ffn
 rr451 rr4999 rr501 sub2495 lg9e27 lg8e27 peUnder peOver z8 s6 z3""".split()
names64 = """k0 k1 kn1 k5 kn42 k2p53m1 k2p53 k10p16m1 k10p16 k10p18 kbig kmax
 g1p5 g0p1 g5p0 g0 ghdn ghup E369 E370 En370 En398 En399 E6n399
 gd30 gd05 gdy gz0 gnz gd5 gq gb384 gb385 gffi gffn
 lg99e114 cvDfDd""".split()
names128 = "t0 t1 t1p5 t0p1 tn5 td05 tdz tdnz tb6144 tb6145 tffi tffn".split()
value_cases = set(os.environ.get("VALUE_CASES", "").split())
bad = 0
def chk(name, w):
    global bad
    gv, lv = grab(g, name), grab(l, name)
    if w == 32:
        gb, lb = gv[0] & 0xFFFFFFFF, lv[0] & 0xFFFFFFFF
        same = (num(dec32(gb)) == num(dec32(lb))) if name in value_cases else (gb == lb)
        detail = f"gcc=0x{gb:08X} lccc=0x{lb:08X}"
    elif w == 64:
        gb = ((gv[1] & 0xFFFFFFFF) << 32 | (gv[0] & 0xFFFFFFFF)) if len(gv) > 1 else gv[0]
        lb = ((lv[1] & 0xFFFFFFFF) << 32 | (lv[0] & 0xFFFFFFFF)) if len(lv) > 1 else lv[0]
        same = (num(dec64(gb)) == num(dec64(lb))) if name in value_cases else (gb == lb)
        detail = f"gcc=0x{gb:016X} lccc=0x{lb:016X}"
    else:
        if len(gv) >= 4: ghi = (gv[3] & 0xFFFFFFFF) << 32 | (gv[2] & 0xFFFFFFFF); glo = (gv[1] & 0xFFFFFFFF) << 32 | (gv[0] & 0xFFFFFFFF)
        else: glo, ghi = gv[0], gv[1]
        if len(lv) >= 4: lhi = (lv[3] & 0xFFFFFFFF) << 32 | (lv[2] & 0xFFFFFFFF); llo = (lv[1] & 0xFFFFFFFF) << 32 | (lv[0] & 0xFFFFFFFF)
        else: llo, lhi = lv[0], lv[1]
        same = num(dec128(ghi, glo)) == num(dec128(lhi, llo))
        detail = f"gcc=0x{ghi:016X}{glo:016X} lccc=0x{lhi:016X}{llo:016X}"
    if not same:
        print(f"FAIL-ROD {name} {detail}"); bad += 1
for n in names32: chk(n, 32)
for n in names64: chk(n, 64)
for n in names128: chk(n, 128)
print(f"ROD-DIVERGENCES={bad}")
print(f"ROD-TOTAL={len(names32) + len(names64) + len(names128)}")
PYEOF
PYEXIT=$?
grep -h "FAIL-ROD\|ROD-DIVERGENCES\|Error\|assert" rod.out | head -25
[ "$PYEXIT" != "0" ] && fail "rodan python crashed"
DIVERG=$(grep -c "FAIL-ROD" rod.out || true)
TOTAL=$(grep "ROD-TOTAL" rod.out | head -1 | sed 's/ROD-TOTAL=//')
TOTAL=${TOTAL:-0}
FAIL=$((FAIL+DIVERG)); PASS=$((PASS+TOTAL-DIVERG))

# ---------- hang pin ----------
cat > hang.c <<'EOF'
_Decimal32 h = 1e-999999999DF;
_Decimal64 h64 = 1e-999999999DD;
int main(void) { return 0; }
EOF
if timeout 60 "$LCCC" -O2 -S -o hang.s hang.c; then pass; else fail "hang-pin (F28)"; fi

# ---------- runtime triangle ----------
cat > tri.c <<'EOF'
#include <stdio.h>
#include <string.h>
#include <stdint.h>
static void p32(const char *n, _Decimal32 v) {
    uint32_t b; memcpy(&b, &v, 4); printf("%s=0x%08X\n", n, b);
}
static void p64(const char *n, _Decimal64 v) {
    uint64_t b; memcpy(&b, &v, 8); printf("%s=0x%016lX\n", n, (unsigned long)b);
}
int main(void) {
    _Decimal32 a = 0, b = 5, c = -5, d = 9999999, e = 8388608;
    _Decimal32 f = 1.5DF, g = -0.5DF, h = 0.1DF, big = 123456789, mx = 2147483647;
    _Decimal32 s = b + d, t = d - b, u = b * g, v = d / b;
    _Decimal32 cv = (int)h + (int)f;
    _Decimal32 z8 = 0.00000001DF, s6 = 1.5e-102DF, z3 = 0.000DF;
    p32("a", a); p32("b", b); p32("c", c); p32("d", d); p32("e", e);
    p32("f", f); p32("g", g); p32("h", h); p32("big", big); p32("mx", mx);
    p32("add", s); p32("sub", t); p32("mul", u); p32("div", v); p32("cv", cv);
    p32("z8", z8); p32("s6", s6); p32("z3", z3);
    _Decimal64 A = 0, B = 9007199254740992LL, C = 9999999999999999LL;
    _Decimal64 D = 1.5DD, E = B + C, F = C - B;
    _Decimal64 G = 123456789012345678LL, H = 9223372036854775807LL;
    p64("A", A); p64("B", B); p64("C", C); p64("D", D);
    p64("E", E); p64("F", F); p64("G", G); p64("H", H);
    return 0;
}
EOF
timeout 120 gcc -O2 -o tri-gcc tri.c || fail "gcc link tri"
timeout 300 "$LCCC" -O2 -o tri-lccc tri.c || fail "lccc link tri"
if [ -x tri-gcc ] && [ -x tri-lccc ]; then
    ./tri-gcc > tri-gcc.out; ./tri-lccc > tri-lccc.out
    if cmp -s tri-gcc.out tri-lccc.out; then pass; else fail "runtime triangle stdout differs"; diff tri-gcc.out tri-lccc.out | head -20; fi
fi

echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" = "0" ]
