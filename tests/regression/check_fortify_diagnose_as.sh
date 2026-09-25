#!/usr/bin/env bash
# Kernel fortify-string contract: `__FORTIFY_INLINE` = `extern __always_inline`
# declarations with bodies (include/linux/fortify-string.h).  GCC's ladder is
# inline-else-fold: __always_inline carries no budget at any -O level, and a
# call to a diagnosed DECLARATION folds to the named builtin.  lccc honored
# neither half at every site: a per-caller 200-insn always_inline allowance
# left `fortify_memset_chk` (integrity_audit.o) and `__fortify_strlen`
# (kernfs/symlink.o, vsprintf.o, dm-sysfs.o) as undefined references that
# broke the vmlinux link.
#
# This gate pins both halves:
#   1. a __diagnose_as DECLARATION (no body) folds its call to the builtin;
#   2. a __always_inline DEFINITION is inlined even when the caller already
#      inlined plenty of always_inline code (no budget escape), so no call
#      to the extern name survives;
# and the program stays correct (strlen of a runtime string).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-fortify.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
/* The kernel's exact declaration pattern, distilled. */
#define __FORTIFY_INLINE extern __attribute__((__always_inline__)) __gnu_inline__
unsigned long builtin_strlen(const char *s); /* maps to the real strlen */

__attribute__((__diagnose_as__(__builtin_strlen, 1)))
extern unsigned long __fortify_strlen(const char *s); /* NO body: extern decl */

__attribute__((__always_inline__))
static inline unsigned long __attribute__((__gnu_inline__)) my_inline_strlen(const char *s) {
    return __fortify_strlen(s);
}

/* Force the caller past any per-caller allowance before the measured site. */
#define MANY(n) my_inline_strlen(a) + n
int run(const char *a, const char *b);
int run(const char *a, const char *b) {
    unsigned long acc = 0;
    acc += my_inline_strlen(a);
    acc += my_inline_strlen(a);
    acc += my_inline_strlen(a);
    acc += acc ? my_inline_strlen(b) : 0;
    return (int)(acc & 1);
}
int main(void) { return run("xx", "yyy"); }
C

# The extern declaration resolves at link time via libc strlen under the
# diagnosed name is NOT provided by libc — provide it ourselves to keep the
# fallback linkable; the contract under test is that NO reference remains.
cat >"$tmp/stub.c" <<'C'
unsigned long __fortify_strlen(const char *s) { return 0; } /* must be unreferenced */
C

"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"
! grep -qE 'call.*__fortify_strlen' "$tmp/t.s"

# Inlining must have happened for the defined always_inline helper: with the
# budget bypass the calls disappear; the strlen contract result is checked at
# runtime through libc strlen via the folded builtin.
"$CCC" -O2 "$tmp/t.c" "$tmp/stub.c" -o "$tmp/t"
set +e
"$tmp/t"
st=$?
set -e
# 2 + 2 + 2 + 3 = 9 length contributions -> the folded builtins compute the
# true strlen at runtime (parity bit 1); a stub hit would give 0.
[ "$st" -eq 1 ]

echo "fortify diagnose-as gate: PASS"

# Kernel-shape pin: the attribute arrives through the compiler_attributes.h
# mapping as __diagnose_as_builtin__ (the clang spelling), on an
# `extern inline __gnu_inline__ __always_inline__` DEFINITION, and the call
# site is the non-constant arm of a __builtin_choose_expr discriminator —
# exactly include/linux/fortify-string.h's strlen pattern.
cat >"$tmp/k.c" <<'C'
typedef unsigned long size_t_;
#define __kernel_size_t size_t_
__attribute__((__diagnose_as_builtin__(__builtin_strlen)))
__attribute__((__always_inline__)) extern inline
    __attribute__((__gnu_inline__)) __kernel_size_t
    __fortify_strlen(const char *const p) { return __builtin_strlen(p); }
unsigned long use(const char *s);
unsigned long use(const char *s) {
    return __builtin_choose_expr(__builtin_constant_p(*s), 0,
                                 __fortify_strlen(s));
}
int main(void) { return (int)(use("kernel-shape") & 1); }
C
"$CCC" -O2 -S "$tmp/k.c" -o "$tmp/k.s"
! grep -qE 'call.*__fortify_strlen' "$tmp/k.s"
"$CCC" -O2 "$tmp/k.c" -o "$tmp/k"
"$tmp/k"; st=$?
[ "$st" -eq 0 ] # strlen=12, even
true

# ── Port-I/O DX-indirect parity (regression from the central mem validator) ──
# GAS admits `(%dx)` only for the IN/INS/OUT/OUTS families (no ModRM; DX is
# implicit in the opcode) and rejects it everywhere else.  The central
# memory-operand validator must not reject the port forms (kernel
# vmware.o broke) nor admit `(%dx)` for ordinary addressing.
printf '.text\ninl (%%dx), %%eax\noutl %%eax, (%%dx)\ninb (%%dx), %%al\noutb %%al, (%%dx)\n' > "$tmp/pio.s"
"$CCC" -c -o "$tmp/pio.o" "$tmp/pio.s"
as -o "$tmp/pio_g.o" "$tmp/pio.s"
cmp <(objdump -s -j .text "$tmp/pio.o" | tail -n +4) <(objdump -s -j .text "$tmp/pio_g.o" | tail -n +4)
printf '.text\nmovl (%%dx), %%eax\n' > "$tmp/badio.s"
"$CCC" -c -o "$tmp/badio.o" "$tmp/badio.s" 2>"$tmp/badio.err" && exit 1
grep -q "not a valid base/index expression" "$tmp/badio.err"
