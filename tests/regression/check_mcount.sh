#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #3): GCC-contract mcount instrumentation
# (-pg / -mfentry / -mrecord-mcount / -mnop-mcount).
#
# Reference shapes measured with GCC 14.2:
#   -pg -mfentry                        -> `call __fentry__` FIRST instruction
#   -pg -mfentry -mrecord-mcount        -> + __mcount_loc,"a",@progbits entry
#   -pg -mfentry -mrecord-mcount -mnop  -> 5-byte NOP (0f 1f 44 00 00) site
#   -pg (classic)                       -> frame first, THEN `call mcount`
#   no_instrument_function              -> no site at all
# Sub-mode flags without -pg are inert (kernel CFLAGS_REMOVE contract).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-mcount.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
__attribute__((noinline)) int add(int a, int b) { return a + b; }
__attribute__((noinline, no_instrument_function)) int noinst(int a) { return a + 1; }
int main(void) { return add(1, 2) + noinst(0) != 4; }
C
body() { sed -n "/^$1:/,/\.size $1/p" "$tmp/$2"; }

# 1. -pg -mfentry: call __fentry__ before any prologue save.
"$CCC" -O2 -pg -mfentry -S "$tmp/t.c" -o "$tmp/fentry.s"
b=$(body add fentry.s)
grep -q 'call __fentry__' <<<"$b"
first_insn=$(grep -m1 -E '^[[:space:]]*(call|pushq?|nop|\.byte)' <<<"$b")
grep -q 'call __fentry__' <<<"$first_insn"
! grep -q 'call __fentry__\|call mcount' <<<"$(body noinst fentry.s)"

# 2. + -mrecord-mcount: __mcount_loc entry for every instrumented function
#    (add + main; noinst is excluded).
"$CCC" -O2 -pg -mfentry -mrecord-mcount -S "$tmp/t.c" -o "$tmp/rec.s"
grep -q '\.section __mcount_loc,"a",@progbits' "$tmp/rec.s"
n=$(grep -c '\.section __mcount_loc' "$tmp/rec.s")
[ "$n" -eq 2 ]

# 3. + -mnop-mcount: the kernel's canonical 5-byte NOP, not `call`.
"$CCC" -O2 -pg -mfentry -mrecord-mcount -mnop-mcount -S "$tmp/t.c" -o "$tmp/nop.s"
grep -q '\.byte 0x0f, 0x1f, 0x44, 0x00, 0x00' "$tmp/nop.s"
! grep -q 'call __fentry__' "$tmp/nop.s"
grep -q '\.section __mcount_loc' "$tmp/nop.s"

# 4. Classic -pg: frame is established BEFORE `call mcount`.
"$CCC" -O2 -pg -S "$tmp/t.c" -o "$tmp/classic.s"
b=$(body add classic.s)
grep -q 'call mcount' <<<"$b"
awk '/pushq %rbp/{push=1} /call mcount/{exit push ? 0 : 1}' <<<"$b"
# Full pipeline smoke: glibc provides mcount; the binary must run.
"$CCC" -O2 -pg "$tmp/t.c" -o "$tmp/classic"
"$tmp/classic"

# 5. Sub-modes without -pg stay inert (VDSO CFLAGS_REMOVE contract).
"$CCC" -O2 -mfentry -mrecord-mcount -mnop-mcount -S "$tmp/t.c" -o "$tmp/inert.s"
! grep -q '__fentry__\|__mcount_loc\|call mcount' "$tmp/inert.s"
