#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #5): MIN_JUMP_TABLE_CASES must match GCC's
# effective x86 threshold. Measured with GCC 14.2 -O2 on side-effecting
# dense switches: 4 cases -> compare chain (no table); 5 cases -> jump table.
# Keeping lccc at 4 emitted jump tables GCC would not, diverging from the
# reference compiler on the switches that dominate kernel dispatch.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-switch.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

gen() {  # gen N > body with N side-effecting cases
    local n=$1 i
    echo "extern void sink(void);"
    echo "__attribute__((noinline)) void sw(int n) {"
    echo "  switch (n) {"
    for ((i = 1; i <= n; i++)); do echo "  case $i: sink(); break;"
    done
    echo "  }"
    echo "}"
}

gen 4 > "$tmp/sw4.c"
gen 5 > "$tmp/sw5.c"
gen 8 > "$tmp/sw8.c"
for f in sw4 sw5 sw8; do
    "$CCC" -O2 -S "$tmp/$f.c" -o "$tmp/$f.s"
    "$CCC" -O2 -c "$tmp/$f.c" -o "$tmp/$f.o"   # must assemble+link-clean
done

# 4 cases: NO indirect jump.
! grep -q 'jmp.*\*\|jmpq.*\*' "$tmp/sw4.s"
# 5 and 8 cases: jump table present.
grep -q 'jmp.*\*\|jmpq.*\*' "$tmp/sw5.s"
grep -q 'jmp.*\*\|jmpq.*\*' "$tmp/sw8.s"
