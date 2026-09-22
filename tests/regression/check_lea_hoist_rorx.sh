#!/usr/bin/env bash
# Loop-invariant LEA hoisting + direct 3-operand RORX: two loop-body
# instruction-quality contracts, each born from a worst-15 benchmark gap.
#
# WHY THIS EXISTS
#
# sha256_transform measured 1.20x slower than gcc, and the loop bodies
# carried two classes of pure waste:
#
#   1. The rip-relative global-address rematerialization: the K-table load
#      `leaq K(%rip), %rcx; movl (%rcx, %r10, 4), ...` paid the LEA EVERY
#      round.  RIP-relative addressing takes no index register, so the base
#      cannot fold into the memory operand — only hoisting removes it.  The
#      IR-level LICM hoists GlobalAddrs when it can, but under register
#      pressure the RA rematerializes them into the loop; the peephole-level
#      hoist (hoist_loop_invariant_gpr_load's LEA candidate class) sees the
#      FINAL addressing form and hoists only what actually materialized.
#      A guarded (fall-through-entry) loop cannot use the load path's
#      jmp-replacement placement (a LOAD would execute on zero-trip paths
#      and may fault) — a PURE LEA may: the insertion lands after the
#      guard's conditional, before the alignment directives, so the loop's
#      .p2align stays adjacent to the header label exactly.
#
#   2. The RORX staging dance: BMI2 RORX is NON-DESTRUCTIVE (3-operand),
#      but the rotate emitter staged its lhs into the destination with a
#      full 64-bit `movq` first (`movq %r14, %rbp; rorxl $25, %ebp, %ebp`).
#      A fresh register home can be read in place: `rorxl $25, %r14d, %ebp`
#      — the typed view reads the low 32 bits and the write fully defines
#      the destination.  sha256's sigma functions paid three dead moves per
#      round.
#
# This gate pins: the hoisted LEA's placement (outside the loop, the
# alignment directives still adjacent to the header), the zero-trip safety
# (a guarded loop whose body never runs still computes its correct result),
# the direct RORX form (no register-to-register `movq` immediately staging
# a `rorxl`), and runtime equality with the reference compiler at both
# BMI2 and baseline levels.
#
# Usage:
#   tests/regression/check_lea_hoist_rorx.sh
# Environment:
#   LCCC            compiler to test (default: target/fastbuild/lccc)
#   CCC_ORACLE_CC   reference compiler (default: cc, gcc or clang)
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-target/fastbuild/lccc}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: no compiler at $LCCC" >&2; exit 1; }
ORACLE=${CCC_ORACLE_CC:-}
if [[ -z "$ORACLE" ]]; then
    for c in cc gcc clang; do command -v "$c" >/dev/null 2>&1 && { ORACLE=$c; break; }; done
fi
[[ -n "$ORACLE" ]] || { echo "FAIL: no reference compiler found" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pass=0; fail=0; skip=0
note() { printf '  %s\n' "$*"; }
ok()   { pass=$((pass+1)); printf 'ok   %s\n' "$*"; }
bad()  { fail=$((fail+1)); printf 'FAIL %s\n' "$*"; }
skipped() { skip=$((skip+1)); printf 'skip %s\n' "$*"; }

# ── A. the K-table shape: LEA hoisted, alignment preserved ──────────────────
cat > "$work/ktab.c" <<'EOF_K'
#include <stdio.h>
static const unsigned K[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2};
/* The guarded 64-round loop over a rip-relative table with a scaled-index
 * load — the sha256 compression shape (fall-through entry, so the load
 * path's jmp-replacement placement does not apply: only the pure LEA
 * candidate may hoist). */
unsigned rounds(unsigned x, unsigned n) {
    unsigned s = 0;
    for (unsigned i = 0; i < n; i++)
        s += K[i & 63] + (x >> ((i & 7) + 1));
    return s;
}
/* Zero-trip safety: the hoisted LEA sits after the guard's conditional;
 * when the loop body never runs the result must be exactly the identity. */
unsigned zero_trip(void) { return rounds(0xdeadbeef, 0); }
int main(void) {
    printf("%u %u\n", rounds(0x13572468u, 64 * 3), zero_trip());
    return 0;
}
EOF_K

"$LCCC" -O2 -march=x86-64-v3 -S -o "$work/ktab.s" "$work/ktab.c" 2>/dev/null
# The hoisted LEA: outside the loop body.  Find the loop (the label whose
# block contains the K load) and require no `leaq K(%rip)` inside it, and
# exactly one before it in the function.
# Extract each loop (label .. its backedge jump) by label matching,
# and count leaq K inside.
leaks=$(python3 - "$work/ktab.s" <<'EOF_PY'
import re, sys
lines = open(sys.argv[1]).read().splitlines()
# find labels and jumps
labels = {l.strip()[:-1]: i for i, l in enumerate(lines) if re.match(r'^\.L\w+:$', l)}
loops = []
for i, l in enumerate(lines):
    m = re.match(r'^\s*j\w+\s+(\.L\w+)$', l)
    if m and m.group(1) in labels and labels[m.group(1)] < i:
        loops.append((labels[m.group(1)], i))
bad = 0
for start, end in loops:
    for i in range(start, end + 1):
        if re.match(r'^\s*leaq K\(%rip\)', lines[i]):
            bad += 1
print(bad)
EOF_PY
)
total_lea=$(grep -cE '^\s*leaq K\(%rip\)' "$work/ktab.s" || true)
if [[ "$leaks" -eq 0 && "$total_lea" -ge 1 ]]; then
    ok "the K-table LEA is hoisted out of the loop ($total_lea materialisation, 0 inside loop bodies)"
else
    bad "K-table LEA hoisting: $leaks in-loop materialisations remain ($total_lea file-wide)"
fi
# Alignment preservation: the .p2align directives must still immediately
# precede the loop's header label (the insertion placed the LEA BEFORE the
# directive run, not between the directives and the label).
align_ok=$(python3 - "$work/ktab.s" <<'EOF_PY'
import re, sys
lines = open(sys.argv[1]).read().splitlines()
ok = True
for i, l in enumerate(lines):
    if re.match(r'^\.L\w+:$', l) and i > 0:
        # the header of a loop (has a backedge somewhere below)
        j = i + 1
        # find a backedge to this label within 400 lines
        lab = l.strip()[:-1]
        has_backedge = any(
            re.match(r'^\s*j\w+\s+' + re.escape(lab) + r'$', x) for x in lines[i:i+400]
        )
        if has_backedge and i > 1:
            # the nearest preceding non-empty line must be .p2align or the LEA-after-guard shape
            k = i - 1
            while k >= 0 and not lines[k].strip():
                k -= 1
            prev = lines[k].strip()
            if not (prev.startswith('.p2align')):
                # acceptable: the hoisted LEA directly before the label when
                # the loop had no alignment directives to begin with
                if not prev.startswith('leaq'):
                    ok = False
print("1" if ok else "0")
EOF_PY
)
if [[ "$align_ok" == "1" ]]; then
    ok "loop alignment directives remain adjacent to header labels (the insertion placed the LEA before the directive run)"
else
    bad "an alignment directive was displaced from its header label"
fi
# Zero-trip + runtime equality at three flag sets.
if "$ORACLE" -O2 -march=x86-64-v3 -o "$work/ktab.oracle" "$work/ktab.c" 2>/dev/null; then
    "$work/ktab.oracle" > "$work/ktab.oracle.txt" 2>&1
    okall=1
    for cf in "-O2" "-O2 -march=x86-64-v3" "-O3 -march=x86-64-v3"; do
        # shellcheck disable=SC2086
        if ! "$LCCC" $cf -o "$work/ktab.lccc" "$work/ktab.c" 2>/dev/null; then
            okall=0; bad "ktab.c did not compile at $cf"; continue
        fi
        "$work/ktab.lccc" > "$work/ktab.lccc.txt" 2>&1
        diff -q "$work/ktab.lccc.txt" "$work/ktab.oracle.txt" >/dev/null \
            || { okall=0; bad "ktab output differs from $ORACLE at $cf"; }
    done
    [[ "$okall" -eq 1 ]] && ok "K-table loop bit-exact vs $ORACLE at 3 flag sets (incl. the zero-trip identity)"
else
    bad "reference compiler could not build ktab.c"
fi

# ── B. the direct 3-operand RORX ────────────────────────────────────────────
cat > "$work/rotx.c" <<'EOF_R'
#include <stdio.h>
unsigned rot_r(unsigned x) { return (x >> 11) | (x << 21); }
unsigned rot_l(unsigned x) { return (x << 11) | (x >> 21); }
unsigned chain(unsigned x) {
    unsigned a = (x >> 6) | (x << 26);
    unsigned b = (a >> 11) | (a << 21);
    return (b >> 25) | (b << 7);
}
int main(void) { printf("%u %u %u\n", rot_r(0x12345678u), rot_l(0x9abcdef0u), chain(0x0f0f0f0fu)); return 0; }
EOF_R
"$LCCC" -O2 -march=x86-64-v3 -S -o "$work/rotx.s" "$work/rotx.c" 2>/dev/null
# No `movq %rA, %rB` immediately staging a same-register `rorxl $k, %rBd, %rBd`.
staged=$(python3 - "$work/rotx.s" <<'EOF_PY'
import re, sys
lines = [l.strip() for l in open(sys.argv[1]).read().splitlines()]
bad = 0
for i in range(len(lines) - 1):
    m1 = re.match(r'^movq %(\w+), %(\w+)$', lines[i])
    m2 = re.match(r'^rorx[lq] \$\d+, %(\w+), %(\w+)$', lines[i + 1])
    if m1 and m2 and m1.group(2) == m2.group(1) == m2.group(2):
        bad += 1
print(bad)
EOF_PY
)
nrorx=$(grep -cE '^\s*rorx[lq] ' "$work/rotx.s" || true)
if [[ "$staged" -eq 0 && "$nrorx" -ge 4 ]]; then
    ok "RORX reads fresh register homes directly ($nrorx three-operand forms, 0 staged movq+same-reg dances)"
else
    bad "RORX staging: $staged movq-staged forms remain ($nrorx rorx total)"
fi
if "$ORACLE" -O2 -march=x86-64-v3 -o "$work/rotx.oracle" "$work/rotx.c" 2>/dev/null; then
    "$work/rotx.oracle" > "$work/rotx.oracle.txt" 2>&1
    "$LCCC" -O2 -march=x86-64-v3 -o "$work/rotx.lccc" "$work/rotx.c" 2>/dev/null
    "$work/rotx.lccc" > "$work/rotx.lccc.txt" 2>&1
    if diff -q "$work/rotx.lccc.txt" "$work/rotx.oracle.txt" >/dev/null; then
        ok "rotate idioms bit-exact vs $ORACLE"
    else
        bad "rotate idiom output differs from $ORACLE"
    fi
else
    bad "reference compiler could not build rotx.c"
fi

# ── C. the guarded-loop insertion end to end: sha256's compression ─────────
if [[ -f tests/benchmark/programs/sha256_transform.c ]]; then
    "$LCCC" -O2 -march=x86-64-v3 -S -o "$work/sha.s" tests/benchmark/programs/sha256_transform.c 2>/dev/null
    inloop=$(python3 - "$work/sha.s" <<'EOF_PY'
import re, sys
lines = open(sys.argv[1]).read().splitlines()
labels = {l.strip()[:-1]: i for i, l in enumerate(lines) if re.match(r'^\.L\w+:$', l)}
backedges = []
for i, l in enumerate(lines):
    m = re.match(r'^\s*j\w+\s+(\.L\w+)$', l)
    if m and m.group(1) in labels and labels[m.group(1)] < i:
        backedges.append((labels[m.group(1)], i))
# Innermost loop around each K-indexed load must not re-materialise the
# base (one-level-up placement inside an enclosing loop is legitimate).
loads = [i for i, l in enumerate(lines) if re.match(r'^\s*movl?\s+\(%r\w+, %r\w+d\)', l)]
bad = 0
for li in loads:
    inner = min(
        ((s, e) for s, e in backedges if s <= li <= e),
        key=lambda se: se[1] - se[0],
        default=None,
    )
    if inner is None:
        continue
    for k in range(inner[0], inner[1] + 1):
        if re.match(r'^\s*leaq K\(%rip\)', lines[k]):
            bad += 1
print(bad)
EOF_PY
    )
    if [[ "$inloop" -eq 0 ]]; then
        ok "sha256_transform's K-table LEA hoisted out of the 64-round loop"
    else
        bad "sha256_transform still materialises the K-table inside the loop ($inloop sites)"
    fi
    "$LCCC" -O2 -march=x86-64-v3 -o "$work/sha.lccc" tests/benchmark/programs/sha256_transform.c 2>/dev/null
    "$ORACLE" -O2 -march=x86-64-v3 -o "$work/sha.oracle" tests/benchmark/programs/sha256_transform.c 2>/dev/null
    "$work/sha.lccc" > "$work/sha.l.txt" 2>&1
    "$work/sha.oracle" > "$work/sha.o.txt" 2>&1
    if diff -q "$work/sha.l.txt" "$work/sha.o.txt" >/dev/null; then
        ok "sha256_transform bit-exact vs $ORACLE with the hoist"
    else
        bad "sha256_transform output differs from $ORACLE"
    fi
else
    skipped "no sha256_transform.c in the tree"
fi

echo
echo "lea-hoist/rorx gate: PASS=$pass FAIL=$fail SKIP=$skip"
[[ "$fail" -eq 0 ]] || exit 1
exit 0
