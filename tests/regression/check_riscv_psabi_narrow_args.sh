#!/usr/bin/env bash
# RISC-V psABI: integer arguments and return values narrower than XLEN.
#
# The psABI widens an integer scalar narrower than 64 bits by the signedness
# of its type to 32 bits and then SIGN-extends it to 64 bits -- for
# arguments in registers, arguments on the stack and return values alike.
# An `unsigned int` therefore travels sign-extended: 0x80000001u is
# 0xffffffff80000001 in a0 and in its 8-byte stack slot.
#
# LCCC keeps a U32 value zero-extended internally and passed it that way:
# a conforming callee that compares the incoming register with a W-instruction
# result as is -- GCC 16.2 and Clang 23.1.4 both compile
# `unsigned b = a + k; return a < b;` to `addw a1, a1, a0; sltu a0, a0, a1` --
# answered 1 for 0x80000001 + 0xffffffff (b = 0x80000000), a returned U32
# parameter (`return x;`) came back zero-extended to a conforming caller, and
# the ninth argument's stack slot held the zero extension.  Constants were
# affected too (`li a1, 2147483651` for 0x80000003u).
#
# The oracle is a psABI-conforming compiler for the other side of every call:
# riscv64-linux-gnu-gcc, or Clang with --target=riscv64-linux-gnu (override
# with RISCV_CC).  Each direction is executed under qemu-riscv64 at -O0, -O1,
# -O2 and -Os; the program is freestanding (raw `ecall`s), so no target libc is
# needed.  The oracle-only build must agree, which validates the test itself.
# Every other narrow type is pinned in both directions against oracle
# callees that are a bare `ret` (int/short/signed char sign-extend, unsigned
# short/char zero-extend -- what the psABI rule reduces to), from truncations
# whose sign bit is set; so are a conforming caller's sign-extended U32 in a
# register and on the stack, which an LCCC callee must widen to 0x80000001.
# Absent tools are reported and the leg is skipped.
set -euo pipefail

CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
RV=$bindir/lccc-riscv
tmp=${TMPDIR:-/tmp}/lccc-rv-psabi.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail=0
bad() {
    echo "FAIL: $*" >&2
    fail=1
}

if [ ! -x "$RV" ]; then
    echo "note: $RV is absent; RISC-V psABI leg skipped"
    exit 0
fi
qemu=${QEMU_RISCV64:-qemu-riscv64}
if ! command -v "$qemu" >/dev/null 2>&1; then
    echo "note: $qemu is absent; RISC-V psABI leg skipped"
    exit 0
fi
oracle=()
if [ -n "${RISCV_CC:-}" ]; then
    read -r -a oracle <<<"$RISCV_CC"
elif command -v riscv64-linux-gnu-gcc >/dev/null 2>&1; then
    oracle=(riscv64-linux-gnu-gcc)
else
    for c in clang-23 clang; do
        if command -v "$c" >/dev/null 2>&1; then
            oracle=("$c" --target=riscv64-linux-gnu -march=rv64gc -mabi=lp64d)
            break
        fi
    done
fi
if [ "${#oracle[@]}" -eq 0 ]; then
    echo "note: no psABI-conforming RISC-V compiler (riscv64-linux-gnu-gcc, clang); leg skipped"
    exit 0
fi
oflags=(-O2 -ffreestanding -fno-pic -fno-pie -fno-stack-protector -fno-builtin -c)

# The oracle side: relies on the psABI extension of every incoming argument,
# returns W-instruction results, and calls back into LCCC.
cat >"$tmp/oracle.c" <<'C'
int u32_add_lt(unsigned a, unsigned k) { unsigned b = a + k; return a < b; }
unsigned u32_add(unsigned a, unsigned k) { return a + k; }
int u16_eq(unsigned short a, unsigned short b) { return a == b; }
int u8_eq(unsigned char a, unsigned char b) { return a == b; }
int s16_lt(short a, short b) { return a < b; }
int s32_add_lt(int a, int k) { int b = (int)((unsigned)a + (unsigned)k); return a < b; }
/* Arguments nine and ten travel on the stack. */
int u32_stack(long r0, long r1, long r2, long r3, long r4, long r5, long r6, long r7,
              unsigned s0, unsigned s1) {
    return (int)(r0 + r1 + r2 + r3 + r4 + r5 + r6 + r7) + (s0 < s1 ? 1 : 0) * 1000;
}
/* The other direction: LCCC-compiled callees seen by a conforming caller. */
unsigned lccc_pass(unsigned x);
unsigned lccc_inc(unsigned x);
unsigned long lccc_widen(unsigned x);
int via_pass(unsigned v, unsigned w) { return lccc_pass(v) < w; }
int via_inc(unsigned v, unsigned w) { return lccc_inc(v) < w; }
unsigned long via_widen(unsigned v) { return lccc_widen(v + 0u); }
unsigned long lccc_stack_widen(long r0, long r1, long r2, long r3, long r4, long r5, long r6,
                               long r7, unsigned s0);
unsigned long via_stack_widen(unsigned v) { return lccc_stack_widen(1, 2, 3, 4, 5, 6, 7, 8, v); }
/* Every narrow type, both directions: these callees are a bare `ret` (GCC
 * 16.2 and Clang 23.1.4 rely on the extension the psABI guarantees). */
long w_i32(int x) { return x; }
long w_i16(short x) { return x; }
long w_i8(signed char x) { return x; }
unsigned long w_u16(unsigned short x) { return x; }
unsigned long w_u8(unsigned char x) { return x; }
int lccc_t32(long x);
short lccc_t16(long x);
signed char lccc_t8(long x);
unsigned short lccc_tu16(long x);
unsigned char lccc_tu8(long x);
long v_t32(long x) { return lccc_t32(x); }
long v_t16(long x) { return lccc_t16(x); }
long v_t8(long x) { return lccc_t8(x); }
unsigned long v_tu16(long x) { return lccc_tu16(x); }
unsigned long v_tu8(long x) { return lccc_tu8(x); }
C

cat >"$tmp/lccc.c" <<'C'
typedef unsigned long u64;
int u32_add_lt(unsigned a, unsigned k);
unsigned u32_add(unsigned a, unsigned k);
int u16_eq(unsigned short a, unsigned short b);
int u8_eq(unsigned char a, unsigned char b);
int s16_lt(short a, short b);
int s32_add_lt(int a, int k);
int u32_stack(long r0, long r1, long r2, long r3, long r4, long r5, long r6, long r7,
              unsigned s0, unsigned s1);
int via_pass(unsigned v, unsigned w);
int via_inc(unsigned v, unsigned w);
u64 via_widen(unsigned v);
u64 via_stack_widen(unsigned v);
long w_i32(int x);
long w_i16(short x);
long w_i8(signed char x);
u64 w_u16(unsigned short x);
u64 w_u8(unsigned char x);
long v_t32(long x);
long v_t16(long x);
long v_t8(long x);
u64 v_tu16(long x);
u64 v_tu8(long x);

unsigned lccc_pass(unsigned x) { return x; }
unsigned lccc_inc(unsigned x) { return x + 1u; }
u64 lccc_widen(unsigned x) { return x; }
u64 lccc_stack_widen(long r0, long r1, long r2, long r3, long r4, long r5, long r6, long r7,
                     unsigned s0) {
    return (u64)s0 + (u64)(r0 + r1 + r2 + r3 + r4 + r5 + r6 + r7 - 36);
}
int lccc_t32(long x) { return (int)x; }
short lccc_t16(long x) { return (short)x; }
signed char lccc_t8(long x) { return (signed char)x; }
unsigned short lccc_tu16(long x) { return (unsigned short)x; }
unsigned char lccc_tu8(long x) { return (unsigned char)x; }

#define SYS_WRITE 64
#define SYS_EXIT 93
static long sys3(long n, long a, long b, long c) {
    register long a0 __asm__("a0") = a;
    register long a1 __asm__("a1") = b;
    register long a2 __asm__("a2") = c;
    register long a7 __asm__("a7") = n;
    __asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a2), "r"(a7) : "memory");
    return a0;
}

void _start(void) {
    /* Locals, not globals: the linker relaxes global accesses to `gp`, and
     * nothing in a freestanding _start points `gp` at __global_pointer$. */
    volatile unsigned vu = 0x80000001u;
    volatile u64 vl = 0x80000001ul;
    volatile unsigned short vh = 0xfffeu;
    volatile unsigned char vb = 0xfeu;
    volatile short vs = -2;
    volatile int vi = 0x7fffffff;
    /* Every truncation below has its sign bit set: low 32 0x9abcdef0,
     * low 16 0xdef0, low 8 0xf0. */
    volatile long vw = 0x123456789abcdef0l;
    unsigned bad = 0;
    /* 0x80000001 + 0xffffffff wraps to 0x80000000: a < b is false. */
    if (u32_add_lt(vu, 0xffffffffu) != 0) bad |= 1u << 0;
    if (u32_add_lt((unsigned)vl, 0xffffffffu) != 0) bad |= 1u << 1;
    /* A constant argument with bit 31 set. */
    if (u32_add_lt(0x80000001u, vu + 0x7ffffffeu) != 0) bad |= 1u << 2;
    /* A returned U32 widened and compared by LCCC. */
    unsigned r = u32_add(vu, 0x7fffffffu); /* 0x00000000 */
    unsigned r2 = u32_add(1u, vu - 2u);    /* 0x80000000 */
    if (r != 0u || (u64)r2 != 0x80000000ul) bad |= 1u << 3;
    if (!(r2 > 0x7fffffffu)) bad |= 1u << 4;
    /* Narrower types. */
    if (u16_eq(vh, 0xfffeu) != 1 || u8_eq(vb, 0xfeu) != 1) bad |= 1u << 5;
    if (s16_lt(vs, 1) != 1 || s32_add_lt(vi, 1) != 0) bad |= 1u << 6;
    /* Stack-passed U32 arguments: 0x80000001 < 0x80000000 is false. */
    if (u32_stack(1, 2, 3, 4, 5, 6, 7, 8, vu, vu - 1u) != 36) bad |= 1u << 7;
    /* LCCC callees returning U32 to a conforming caller. */
    if (via_pass(vu, 0x80000000u) != 0) bad |= 1u << 8;
    if (via_inc(vu, 0x80000003u) != 1) bad |= 1u << 9;
    if (via_inc(0xffffffffu, 1u) != 1) bad |= 1u << 10;
    if (via_widen(vu) != 0x80000001ul) bad |= 1u << 11;
    /* A conforming caller's stack slot holds the sign extension. */
    if (via_stack_widen(vu) != 0x80000001ul) bad |= 1u << 12;
    /* Narrow arguments (LCCC caller) and results (LCCC callee). */
    long x = vw;
    if (w_i32((int)x) != (long)(int)0x9abcdef0u || w_i16((short)x) != (long)(short)0xdef0 ||
        w_i8((signed char)x) != (long)(signed char)0xf0)
        bad |= 1u << 13;
    if (w_u16((unsigned short)x) != 0xdef0ul || w_u8((unsigned char)x) != 0xf0ul) bad |= 1u << 14;
    if (v_t32(x) != (long)(int)0x9abcdef0u || v_t16(x) != (long)(short)0xdef0 ||
        v_t8(x) != (long)(signed char)0xf0)
        bad |= 1u << 15;
    if (v_tu16(x) != 0xdef0ul || v_tu8(x) != 0xf0ul) bad |= 1u << 16;
    char msg[8];
    int n = 0;
    do {
        msg[n++] = (char)('0' + bad % 10u);
        bad /= 10u;
    } while (bad && n < 7);
    for (int i = 0; i < n / 2; i++) {
        char t = msg[i];
        msg[i] = msg[n - 1 - i];
        msg[n - 1 - i] = t;
    }
    msg[n] = '\n';
    sys3(SYS_WRITE, 1, (long)msg, n + 1);
    sys3(SYS_EXIT, 0, 0, 0);
    for (;;) {
    }
}
C

"${oracle[@]}" "${oflags[@]}" "$tmp/oracle.c" -o "$tmp/oracle.o"

# The oracle on both sides validates the program itself.
if "${oracle[@]}" "${oflags[@]}" "$tmp/lccc.c" -o "$tmp/self.o" 2>"$tmp/self.err"; then
    "$RV" -nostdlib -static "$tmp/self.o" "$tmp/oracle.o" -o "$tmp/self"
    got=$("$qemu" "$tmp/self")
    [ "$got" = 0 ] || bad "oracle-only build printed '$got' (the test is wrong)"
else
    bad "the oracle rejected the LCCC side: $(cat "$tmp/self.err")"
fi

for opt in -O0 -O1 -O2 -Os; do
    "$RV" "$opt" -ffreestanding -nostdlib -static -c "$tmp/lccc.c" -o "$tmp/l$opt.o"
    "$RV" -nostdlib -static "$tmp/l$opt.o" "$tmp/oracle.o" -o "$tmp/mixed$opt"
    got=$("$qemu" "$tmp/mixed$opt")
    [ "$got" = 0 ] || bad "lccc-riscv $opt: failure mask $got (bit 0-2: U32 arguments, 3-4: U32 returns, 5-6: narrow types, 7: stack, 8-12: LCCC callees, 13-16: every narrow type both ways)"
done

# The shape that carries the fix: the U32 argument register is written by a
# sign extension, and `return x;` of a U32 sign-extends into a0.
"$RV" -O2 -S "$tmp/lccc.c" -o "$tmp/l.s"
body() {
    sed -n "/^$2:/,/^\\.size $2,/p" "$1" | sed -e 's/^[[:space:]]*//' | grep -v -E '^(\.|#|$)' || true
}
text=$(body "$tmp/l.s" lccc_pass)
grep -q -E '^sext\.w a0, ' <<<"$text" || bad "lccc_pass does not sign-extend its U32 result into a0: $(paste -sd ';' - <<<"$text")"
text=$(body "$tmp/l.s" _start)
grep -q -E '^sext\.w (a[0-7]|t[3-5]), ' <<<"$text" || bad "_start stages no U32 argument through sext.w: $(paste -sd ';' - <<<"$text")"

[ "$fail" -eq 0 ] || exit 1
echo "check_riscv_psabi_narrow_args: PASS (U32 sign-extended in argument and return registers; narrow, stack and callee directions; -O0/-O1/-O2/-Os under qemu-riscv64 against ${oracle[0]})"
