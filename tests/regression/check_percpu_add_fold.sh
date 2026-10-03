#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #4): per_cpu_ptr()-style Add fold.
#
# Shape: Cast(GlobalAddr(sym)) + register offset (an Add, not a GEP). The
# Cast result is in global_addr_map but NOT rematerializable, so the remat
# path doesn't fire and emit_binop stranded the dest without a register home
# (workqueue_prepare_cpu ICE: "value N has no register home"). The fix folds
# sym + reg into a SIB leaq, mirroring the upstream GEP fold (804ce8c).
#
# The test proves (a) the fold emits leaq sym(reg), and (b) the computed
# address is CORRECT (store through it lands at sym+off) — a fold that
# miscomputes the address would pass (a) and fail (b).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-percpu-add.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
#include <stdio.h>
char buf_base[64];

__attribute__((noinline)) int poke(unsigned long off)
{
    char *p = (char *)buf_base + off;
    *p = 42;
    return (unsigned char)buf_base[off];
}

int main(void)
{
    if (poke(10) != 42) return 1;
    if (buf_base[10] != 42) return 2;
    if (poke(63) != 42) return 3;
    printf("OK\n");
    return 0;
}
C
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"
"$CCC" -O2 "$tmp/t.c" -o "$tmp/t"
"$tmp/t" | grep -q OK
# The poke body computes buf_base + %reg via a SIB leaq (or an equivalent
# symbol-indexed addressing form) rather than materialising the symbol in a
# register first and adding.
sed -n '/^poke:/,/\.size poke/p' "$tmp/t.s" | grep -q 'buf_base(%'
