#!/usr/bin/env bash
# check_redundant_flags_compare.sh — one comparison, one set of flags.
#
# THE DEFECT
# ----------
# x86's CMOVcc reads EFLAGS but does not write it. The backend emits one
# `cmp` per `cmov`, so when if-conversion turns a chain of branches that share
# a condition into a chain of `cmov`s, every `cmov` after the first is handed a
# fresh recomputation of a comparison the flags already hold. SQLite's varint
# decoder is the canonical shape -- the two tail arms test the same byte:
#
#     cmpb $-128, %r10b
#     movl %r11d, %edx
#     cmovbl %r12d, %edx
#     cmpb $-128, %r10b        <-- recomputes flags nobody changed
#     movl $3, %ecx
#     movl $4, %ebx
#     cmovbl %ecx, %ebx
#
# The second `cmp` is dead: `cmovbl` left the flags exactly as the first `cmp`
# set them. It is also the long pole -- it re-derives a dependency on %r10b's
# comparison result that the first `cmp` had already settled, so it costs both
# an instruction and latency in the arm the kernel spends most of its time in.
#
# MEASURED
# --------
# Interleaved A/B, min of 41 alternating samples, CPU-pinned:
#     min    -4.32 %      median  -2.66 %      paired win rate  25/41 (61 %)
# The noise floor, calibrated on a kernel this pass does NOT touch
# (strcmp_signed), is a 52 % paired win rate -- so 61 % is a real effect and a
# 52 % reading is not. Corpus-wide the pass fires on 28 files and shrinks every
# one of them (76 instructions total); no file grows.
#
# THE CONTROL IS THE POINT
# ------------------------
# An instruction-count assertion passes for the wrong reason just as easily as
# any other: a peephole that deleted the *first* cmp, or deleted both, would
# also lower the count while breaking the program. So this gate pins three
# things together:
#
#   1. RUNTIME. The decoded values and lengths must be bit-identical to GCC's
#      for the same input. A dropped cmp changes which arm is taken and shows
#      up here as a wrong checksum long before anyone reads the assembly.
#   2. SHAPE. The hot block must contain exactly ONE `cmpb $-128, %r10b` and
#      still contain TWO `cmovb` -- one comparison feeding two conditional
#      moves is the property; the count alone would not distinguish it from
#      "the fold deleted a cmov too".
#   3. NEGATIVE CONTROL. `CCC_PEEPHOLE_SKIP=flags_compare` must bring the
#      duplicate BACK. Without this, a build where the pass silently stopped
#      running -- renamed, re-ordered out of the pass list, skipped by a
#      default -- would sail through 1 and 2 and this gate would be
#      decoration rather than evidence.
#
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

src=$here/../oracle/varint_decode.c
# Fail CLOSED on a missing prerequisite. These used to `exit 0`, which meant a
# gate whose entire subject -- the fixture, or the compiler it exists to
# inspect -- could silently vanish and the suite would still print PASS. A
# gate that cannot run is not a gate that passed.
[ -f "$src" ] || { echo "FAIL: fixture missing: $src" >&2; exit 1; }
"$CCC" --version >/dev/null 2>&1 || { echo "FAIL: no lccc at $CCC" >&2; exit 1; }

fail=0
note() { printf '      %s\n' "$*"; }

# ── 1. runtime differential against GCC ─────────────────────────────────────
cat > "$work/main.c" <<'EOF'
#include <stdint.h>
#include <stdio.h>
extern unsigned char v[2048];
extern unsigned long long bench_run(void);
int main(void) {
    for (int i = 0; i < 2048; i++) v[i] = (unsigned char)((i * 37) | ((i & 3) ? 0x80 : 0));
    printf("%llu\n", bench_run());
    return 0;
}
EOF
"$GCC" -O2 -march=x86-64-v3 -w "$src" "$work/main.c" -o "$work/ref" 2>/dev/null
"$CCC" -O3 -march=x86-64-v3 -w "$src" "$work/main.c" -o "$work/got" 2>/dev/null
ref=$("$work/ref"); got=$("$work/got")
if [ "$ref" = "$got" ]; then
    note "runtime: matches gcc ($got)"
else
    echo "FAIL: runtime result differs from gcc: ref=$ref got=$got" >&2
    fail=1
fi

# ── 2 & 3. assembly shape, with the pass disabled as the control ───────────
"$CCC" -O3 -march=x86-64-v3 -S "$src" -o "$work/on.s" 2>/dev/null
CCC_PEEPHOLE_SKIP=flags_compare \
    "$CCC" -O3 -march=x86-64-v3 -S "$src" -o "$work/off.s" 2>/dev/null

# The hot block is the one holding the tail-arm cmovs.
#
# Two robustness notes, both learned from this gate lying before:
#   * `grep -c` exits 1 when it counts ZERO. Under `set -euo pipefail` the
#     `on=$(hot ...)` assignment therefore aborted the whole script BEFORE the
#     diagnostic below could print, so the regression that matters most --
#     the compare disappearing entirely -- presented as a bare non-zero exit.
#     `awk` does the counting instead, so the count is always a real integer.
#   * The register was hardcoded to `%r10b`. That couples the gate to an
#     incidental register-allocation choice: any unrelated change to the
#     allocator would break the gate for a reason that has nothing to do with
#     what it tests. Match any byte register instead.
hot() { awk '/^\.LBB[0-9]+:/{blk=$0; buf=""} {buf=buf"\n"$0}
        /cmovb/{seen[blk]=buf} END{for(b in seen) print seen[b]}' "$1" \
        | awk '/^[[:space:]]+cmpb \$-128, %[a-z0-9]+b$/ {n++} END{print n+0}'; }

on=$(hot "$work/on.s"); off=$(hot "$work/off.s")

if [ "$on" -eq 1 ]; then
    note "shape: one cmpb feeding the tail-arm cmovs (was $off)"
else
    echo "FAIL: expected exactly 1 'cmpb \$-128, %<byte-reg>' in the hot block, found $on" >&2
    fail=1
fi

ncmov=$(grep -cE '^[[:space:]]+cmovb' "$work/on.s")
if [ "$ncmov" -ge 2 ]; then
    note "shape: $ncmov cmovb present, so the fold removed a cmp and not a cmov"
else
    echo "FAIL: only $ncmov cmovb in the function; the fold may have eaten a move" >&2
    fail=1
fi

if [ "$off" -gt "$on" ]; then
    note "control: disabling the pass restores the duplicate ($off -> $on with it on)"
else
    echo "FAIL: negative control did not restore the duplicate (off=$off, on=$on);" >&2
    echo "      the pass is not the thing removing it, so this gate proves nothing" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: redundant-flags-compare contract" >&2
    exit 1
fi
echo "PASS: one comparison feeds the cmov chain; runtime matches gcc; control restores the duplicate"
