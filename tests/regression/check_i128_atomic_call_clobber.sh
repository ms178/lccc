#!/usr/bin/env bash
# A value that lives across a 16-byte atomic must survive it.
#
# On x86-64 a 16-byte atomic load, store or RMW is not an instruction: it
# lowers to a __atomic_*_16 libatomic CALL. A call destroys every caller-saved
# register, so liveness must record the atomic as a call point. It did not --
# instruction_is_call_point covered i128 division and the F128 helpers, which
# are calls for the same reason, but had no arm for atomics. With the call
# missing from call_points, collect_call_spanning_owners found nothing spanning
# it, and every caller-saved register wave was then free to home a value that
# lives across the atomic in a register the call clobbers.
#
# gaddrcse is what supplies such a value: at -O2 it merges the two
# materialisations of one global's address into a single live range that
# reaches past the atomic. The 16-byte load on the far side was emitted as
#
#     call __atomic_store_16@PLT
#     movdqu (%r11), %xmm0        <-- nothing in the function ever writes %r11
#
# so the load dereferenced whatever the PLT stub happened to leave behind.
# -O0/-O1 were correct, which is why the -O1 runtime half of
# check_atomic16_runtime.sh never saw it. See
# docs/FOLLOWUP-2026-10-21-i128-atomic-o2.md for the investigation.
#
# Two independent halves, because either alone can pass on a broken compiler:
#
#   1. STRUCTURAL (no oracle needed, so it always runs): in the -O2 assembly,
#      no memory operand may be based on a register the function never writes.
#      This catches the defect class directly, including on a run where the
#      stale register happens to hold a plausible value.
#
#   2. BEHAVIOURAL (GCC 16 differential): the same program must produce the
#      same value as GCC 16.2 at every optimisation level and ISA config.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc=${GCC16:-gcc-16}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
fail() { echo "FAIL: $*" >&2; exit 1; }

# The store/load pair is the whole trigger: one 16-byte atomic store, then a
# 16-byte atomic load of the same object, with the address materialised twice
# so that gaddrcse has something to merge across the call.
cat >"$tmp/repro.c" <<'EOF'
typedef unsigned __int128 u128;
static _Alignas(16) _Atomic u128 v;
unsigned long long h;

int main(void) {
    u128 k = ((u128)0x0123456789ABCDEFULL << 64) | 0xFEDCBA9876543210ULL;
    __atomic_store_n(&v, k + 1, __ATOMIC_RELAXED);
    /* The do/while(0) block is load-bearing: it is what makes the frontend
       emit the second address materialisation as a separate value, which is
       the value gaddrcse then merges across the atomic. Flattened, the
       miscompile does not reproduce. */
    do {
        u128 t_ = (v);
        h = h * 1000003ULL + (unsigned long long)t_ + (unsigned long long)(t_ >> 64);
    } while (0);
    return (int)h;
}
EOF

# ------------------------------------------------------------- structural ---
# Every register used as a memory-operand base in main must also occur outside
# a parenthesised address somewhere in main -- i.e. the function must actually
# write it. %rsp/%rbp are exempt: the ABI and the prologue establish them.
check_bases() { # <asm file> <label>
    python3 - "$1" "$2" <<'PY'
import re, sys

path, label = sys.argv[1], sys.argv[2]
text = open(path).read()
if "\nmain:" not in text:
    sys.exit(f"FAIL: {label}: no main in the assembly")
body = text.split("\nmain:", 1)[1]
for stop in ("\n.size", "\n.LFE", "\nmain:"):
    body = body.split(stop, 1)[0]

bases = set(re.findall(r"\(%(r[a-z0-9]+)", body))
# RIP-relative operands (`v(%rip)`) are not register bases at all.
bases -= {"rip", "eip"}
# An occurrence not preceded by "(" is a real operand, not an address base.
written = {m.group(1) for m in re.finditer(r"(?<!\()%(r[a-z0-9]+)", body)}
written |= {"rsp", "rbp"}

dangling = sorted(bases - written)
if dangling:
    offenders = [ln.strip() for ln in body.splitlines()
                 if any(f"(%{r}" in ln for r in dangling)]
    sys.exit("FAIL: {}: main dereferences register(s) it never writes: {}\n  {}"
             .format(label, ", ".join("%" + r for r in dangling),
                    "\n  ".join(offenders)))
PY
}

for isa in "" "-mcx16"; do
    for o in -O1 -O2 -O3; do
        lbl="${isa:-default} $o"
        $ccc $o $isa -S -o "$tmp/s.s" "$tmp/repro.c" \
            || fail "lccc $lbl could not compile the reproducer"
        check_bases "$tmp/s.s" "$lbl"
    done
done
echo "ok: no memory operand is based on a register main never writes"

# ----------------------------------------------------------- behavioural ---
if ! command -v "$gcc" >/dev/null 2>&1; then
    echo "SKIP: $gcc not available; structural half only" >&2
    exit 0
fi

mismatch=0
for isa in "" "-mcx16" "-march=x86-64-v3" "-march=x86-64-v2" "-mno-cx16"; do
    for o in -O0 -O1 -O2 -O3; do
        lbl="${isa:-default} $o"
        $gcc $o $isa "$tmp/repro.c" -latomic -o "$tmp/o_gcc" 2>/dev/null \
            || { echo "SKIP: $gcc cannot build $lbl" >&2; exit 0; }
        $ccc $o $isa "$tmp/repro.c" -latomic -o "$tmp/o_lccc" \
            || fail "lccc $lbl build"
        set +e
        "$tmp/o_gcc";  want=$?
        "$tmp/o_lccc"; got=$?
        set -e
        if [ "$want" != "$got" ]; then
            echo "FAIL: $lbl value differs from $gcc: gcc=$want lccc=$got" >&2
            mismatch=1
        fi
    done
done
[ "$mismatch" = 0 ] || exit 1
echo "ok: matches $gcc at -O0..-O3 across five ISA configs"
