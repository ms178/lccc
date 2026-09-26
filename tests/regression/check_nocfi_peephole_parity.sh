#!/usr/bin/env bash
# Without unwind tables (-fno-asynchronous-unwind-tables, the Linux kernel's
# default) the peephole must optimize exactly as with them. It delimits
# functions by .cfi_startproc/.cfi_endproc, so the codegen emits those as
# bare boundary markers and strips them after the peephole
# (CodegenState::fn_boundary_markers). Before, every liveness-driven fold
# was disabled in that mode: +19% instructions on SQLite's select.c.
#
# Checks, for x86-64 and i686:
#   1. the no-unwind assembly equals the default assembly with all CFI
#      directives removed (no lost optimization, no stray marker);
#   2. CFI written by the user in top-level asm survives the strip;
#   3. the no-unwind program runs correctly.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-nocfi-parity.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
#include <string.h>
struct node { struct node *next; long key; int n; char name[24]; };
__attribute__((noinline)) long walk(struct node *p, long lim)
{
    long s = 0;
    for (; p; p = p->next) {
        if (p->key > lim) s += p->key * 3 + p->n;
        else s ^= (long)strlen(p->name) << (p->n & 7);
    }
    return s;
}
__attribute__((noinline)) int pick(const int *a, int n, int t)
{
    int best = -1, bd = 1 << 30;
    for (int i = 0; i < n; i++) {
        int d = a[i] > t ? a[i] - t : t - a[i];
        if (d < bd) { bd = d; best = i; }
    }
    return best;
}
/* A hand-written function with its own CFI: must survive untouched. */
__asm__(".text\n.globl handcfi\n.type handcfi,@function\nhandcfi:\n"
        ".cfi_startproc\n    movl $7, %eax\n    ret\n.cfi_endproc\n"
        ".size handcfi, .-handcfi\n");
int handcfi(void);
int main(void)
{
    struct node c = {0, 9, 2, "gamma"}, b = {&c, 1, 3, "be"}, a = {&b, 5, 1, "a"};
    int v[6] = {4, 19, -3, 8, 11, 7};
    if (walk(&a, 4) != ((5 * 3 + 1) ^ (2L << 3)) + 9 * 3 + 2) return 1;
    if (pick(v, 6, 10) != 4) return 2;
    return handcfi() == 7 ? 0 : 3;
}
C
for m in "" -m32; do
    "$CCC" $m -O2 -S "$tmp/t.c" -o "$tmp/cfi.s"
    "$CCC" $m -O2 -fno-asynchronous-unwind-tables -S "$tmp/t.c" -o "$tmp/nocfi.s"
    if ! diff <(grep -v '\.cfi_' "$tmp/cfi.s") <(grep -v '\.cfi_' "$tmp/nocfi.s") >"$tmp/d"; then
        echo "FAIL${m:+ ($m)}: code differs without unwind tables:" >&2
        cat "$tmp/d" >&2
        exit 1
    fi
    n=$(grep -c '\.cfi_' "$tmp/nocfi.s" || true)
    if [[ "$n" != 2 ]]; then
        echo "FAIL${m:+ ($m)}: expected only the 2 user CFI directives, found $n" >&2
        grep -n '\.cfi_' "$tmp/nocfi.s" >&2 || true
        exit 1
    fi
    if [[ -z "$m" ]] || echo 'int main(void){return 0;}' | "$CCC" -m32 -x c - -o "$tmp/probe" 2>/dev/null; then
        "$CCC" $m -O2 -fno-asynchronous-unwind-tables "$tmp/t.c" -o "$tmp/t"
        "$tmp/t"
    fi
done
echo "PASS: no-unwind codegen parity"
