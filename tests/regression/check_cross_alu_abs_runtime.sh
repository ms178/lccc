#!/usr/bin/env bash
# Constant-operand ALU forms and sign-magnitude selects, executed on all four
# targets from one freestanding source, plus the shapes that make them fast.
#
#   * RISC-V selects the I-type form for `x op K` whenever the constant fits
#     (`addi[w]`, `andi`, `ori`, `xori`, `slli[w]`/`srli[w]`/`srai[w]`, and
#     `x - K` as `addi[w] -K`): one instruction where the backend used to
#     stage K through two `li`s and a copy into an R-type form (`x >> 3` was
#     `li t0, 3; li t2, 3; sra t0, t1, t2`, now `srai t0, a0, 3` as in GCC
#     16.2 and Clang 23.1.4).  The RV64 peephole's sext32-producer table
#     learned the `w` immediates and stopped treating a short `srai` as one:
#     `(int)(x >> 3)` keeps its `sext.w`.
#   * `x < 0 ? -x : x` (and `abs`, `labs`, `llabs`, and the zero-inclusive
#     spellings `x <= 0 ? -x : x`, `x > 0 ? x : -x`) is the sign-magnitude
#     identity on every target -- the IR pass used to see the select only on
#     x86-64 -O2/-O3, where the vectorizer's if-conversion formed it early;
#     a select formed by cfg_simplify at the end of an iteration now wakes
#     the select-shape passes in the next one -- and x86-64 then turns the
#     identity into GCC's and Clang's `mov; neg; cmovs` (`abs_cmov`).
#   * AArch64 leaves every function holding inline assembly as emitted: its
#     peephole deleted the `movz x8, #64` and `movz x2, #17` feeding an
#     `svc 0` because no later line named x8/x2 before the next system call
#     overwrote them (`write` ran as `semget`).  The program below makes
#     exactly those two calls.
#
# Execution is the oracle: every build prints one FNV-1a digest over ~60
# functions, the I-type boundaries (-2048, 2047, 2048, -2049), shift counts
# at and past the width, truncations of 64-bit shifts, and the magnitudes,
# on edge values and 40,000 LCG draws.  GCC 16.2 at -O2 defines the digest;
# x86-64 (-O0/-O1/-O2/-Os), i686 (-O0/-O2/-Os), RISC-V and AArch64 (-O0/-O1/
# -O2/-Os, under qemu-user) must all print it.  Cross legs whose emulator or
# backend is absent are reported and skipped.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
GCC=${GCC:-gcc-16}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-cross-alu-abs.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
want=432d112575bf3b5f
fail=0
note() { printf '%s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

cat > "$tmp/t.c" <<'C'
/* Freestanding: every target runs it bare (qemu-user for the cross ones), so
   no libc is needed to compare the answers. */
typedef unsigned long long u64;
typedef long long i64;
typedef unsigned int u32;
typedef int i32;

#if defined(__x86_64__)
static long sys3(long n, long a, long b, long c) {
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return r;
}
#define SYS_WRITE 1
#define SYS_EXIT 60
__asm__(".globl _start\n_start:\n    xorl %ebp, %ebp\n    andq $-16, %rsp\n    call entry\n    hlt\n");
#elif defined(__i386__)
static long sys3(long n, long a, long b, long c) {
    long r;
    __asm__ volatile("int $0x80" : "=a"(r) : "a"(n), "b"(a), "c"(b), "d"(c) : "memory");
    return r;
}
#define SYS_WRITE 4
#define SYS_EXIT 1
__asm__(".globl _start\n_start:\n    xorl %ebp, %ebp\n    andl $-16, %esp\n    call entry\n    hlt\n");
#elif defined(__aarch64__)
static long sys3(long n, long a, long b, long c) {
    register long x8 __asm__("x8") = n;
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    __asm__ volatile("svc 0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}
#define SYS_WRITE 64
#define SYS_EXIT 93
__asm__(".globl _start\n_start:\n    mov x29, #0\n    bl entry\n1:\n    b 1b\n");
#elif defined(__riscv)
static long sys3(long n, long a, long b, long c) {
    register long a7 __asm__("a7") = n;
    register long a0 __asm__("a0") = a;
    register long a1 __asm__("a1") = b;
    register long a2 __asm__("a2") = c;
    __asm__ volatile("ecall" : "+r"(a0) : "r"(a7), "r"(a1), "r"(a2) : "memory");
    return a0;
}
#define SYS_WRITE 64
#define SYS_EXIT 93
__asm__(".globl _start\n_start:\n    li fp, 0\n    call entry\n1:\n    j 1b\n");
#else
#error unsupported target
#endif

#define NI __attribute__((noinline))

/* Constant right operands at the I-type boundaries (-2048, 2047), just past
   them (2048, -2049), and the shift counts at and past the width. */
NI i32 add1(i32 x) { return (i32)((u32)x + 1u); }
NI i32 add_max(i32 x) { return (i32)((u32)x + 2047u); }
NI i32 add_min(i32 x) { return (i32)((u32)x - 2048u); }
NI i32 add_past(i32 x) { return (i32)((u32)x + 2048u); }
NI i32 sub5(i32 x) { return (i32)((u32)x - 5u); }
NI i32 sub_m2048(i32 x) { return (i32)((u32)x - (u32)-2048); }
NI i64 add64(i64 x) { return (i64)((u64)x + 100u); }
NI i64 sub64(i64 x) { return (i64)((u64)x - 2048u); }
NI i64 sub64_past(i64 x) { return (i64)((u64)x - 2049u); }
NI u32 uadd(u32 x) { return x + 4000000000u; }
NI u32 usub(u32 x) { return x - 7u; }
NI i32 and255(i32 x) { return x & 255; }
NI i32 and_neg(i32 x) { return x & -16; }
NI u32 uand_neg(u32 x) { return x & 0xfffffff0u; }
NI u64 uand64_neg(u64 x) { return x & ~15ull; }
NI i64 or16(i64 x) { return x | 16; }
NI u32 uor_neg(u32 x) { return x | 0xfffff800u; }
NI i32 xor_m1(i32 x) { return x ^ -1; }
NI u32 uxor(u32 x) { return x ^ 0x7ffu; }
NI i64 xor64(i64 x) { return x ^ -2048; }
NI i32 shl3(i32 x) { return (i32)((u32)x << 3); }
NI i32 shl31(i32 x) { return (i32)((u32)x << 31); }
NI i64 shl40(i64 x) { return (i64)((u64)x << 40); }
NI i32 sra3(i32 x) { return x >> 3; }
NI i32 sra31(i32 x) { return x >> 31; }
NI i64 sra3l(i64 x) { return x >> 3; }
NI i64 sra63(i64 x) { return x >> 63; }
NI u32 srl3(u32 x) { return x >> 3; }
NI u32 srl31(u32 x) { return x >> 31; }
NI u64 srl3l(u64 x) { return x >> 3; }
NI u64 srl63(u64 x) { return x >> 63; }
/* Truncation after a 64-bit arithmetic shift: the `sext.w` is real code. */
NI i32 trunc_sra(i64 x) { return (i32)(x >> 3); }
NI i32 trunc_sra31(i64 x) { return (i32)(x >> 31); }
NI i32 trunc_sra32(i64 x) { return (i32)(x >> 32); }
NI i32 trunc_srl(u64 x) { return (i32)(x >> 5); }
NI unsigned char ub(unsigned char x) { return (unsigned char)(x + 200); }
NI signed char sb(signed char x) { return (signed char)(x ^ 0x55); }
NI unsigned short uh(unsigned short x) { return (unsigned short)(x << 9); }
NI short sh(short x) { return (short)(x >> 2); }
/* Signed-typed constant operations on inputs that cannot overflow. */
NI i32 sadd(i32 x) { return x + 2047; }
NI i32 ssub(i32 x) { return x - 2048; }
NI i64 sadd64(i64 x) { return x + -2048; }
/* Magnitudes, in every spelling the sign-magnitude pass accepts. */
NI i32 abs_lt(i32 x) { return x < 0 ? -x : x; }
NI i32 abs_le(i32 x) { return x <= 0 ? -x : x; }
NI i32 abs_gt(i32 x) { return x > 0 ? x : -x; }
NI i32 abs_ge(i32 x) { return x >= 0 ? x : -x; }
NI i64 abs64(i64 x) { return x < 0 ? -x : x; }
NI i32 abs_plus(i32 x) { return (x < 0 ? -x : x) + x; }
NI i32 absdiff(i32 a, i32 b) { return a > b ? a - b : b - a; }
/* One step away: not a magnitude. */
NI i32 not_abs(i32 x) { return x < 1 ? x : -x; }
NI i32 neg_abs(i32 x) { return x < 0 ? x : -x; }

static u64 h = 1469598103934665603ull;
static void mix(u64 v) {
    for (int i = 0; i < 8; i++) {
        h ^= (v >> (8 * i)) & 0xffu;
        h *= 1099511628211ull;
    }
}

static volatile u64 seed_box = 0x9e3779b97f4a7c15ull;

static void run(u64 v) {
    i32 x = (i32)v;
    i64 y = (i64)v;
    mix((u64)add1(x)); mix((u64)add_max(x)); mix((u64)add_min(x)); mix((u64)add_past(x));
    mix((u64)sub5(x)); mix((u64)sub_m2048(x)); mix((u64)add64(y)); mix((u64)sub64(y));
    mix((u64)sub64_past(y)); mix(uadd((u32)v)); mix(usub((u32)v)); mix((u64)and255(x));
    mix((u64)and_neg(x)); mix(uand_neg((u32)v)); mix(uand64_neg(v)); mix((u64)or16(y));
    mix(uor_neg((u32)v)); mix((u64)xor_m1(x)); mix(uxor((u32)v)); mix((u64)xor64(y));
    mix((u64)shl3(x)); mix((u64)shl31(x)); mix((u64)shl40(y)); mix((u64)sra3(x));
    mix((u64)sra31(x)); mix((u64)sra3l(y)); mix((u64)sra63(y)); mix(srl3((u32)v));
    mix(srl31((u32)v)); mix(srl3l(v)); mix(srl63(v)); mix((u64)trunc_sra(y));
    mix((u64)trunc_sra31(y)); mix((u64)trunc_sra32(y)); mix((u64)trunc_srl(v));
    mix(ub((unsigned char)v)); mix((u64)sb((signed char)v)); mix(uh((unsigned short)v));
    mix((u64)sh((short)v));
    mix((u64)sadd(x / 2)); mix((u64)ssub(x / 2)); mix((u64)sadd64(y / 2));
    /* No INT_MIN for the magnitudes: -INT_MIN is undefined in C. */
    if (x != (i32)0x80000000) {
        mix((u64)abs_lt(x)); mix((u64)abs_le(x)); mix((u64)abs_gt(x)); mix((u64)abs_ge(x));
        mix((u64)abs_plus(x / 2)); mix((u64)not_abs(x)); mix((u64)neg_abs(x));
    }
    if (y != (i64)0x8000000000000000ull)
        mix((u64)abs64(y));
    mix((u64)absdiff(x / 4, (i32)(v >> 40) / 4));
}

static void put_hex(u64 v) {
    char buf[17];
    for (int i = 15; i >= 0; i--) {
        buf[i] = "0123456789abcdef"[v & 15];
        v >>= 4;
    }
    buf[16] = '\n';
    sys3(SYS_WRITE, 1, (long)buf, 17);
}

void entry(void) {
    static const u64 edges[] = {
        0, 1, 2, 3, 7, 8, 31, 32, 33, 63, 64, 255, 256, 2047, 2048, 2049,
        0x7fffffffull, 0x80000000ull, 0x80000001ull, 0xffffffffull, 0x100000000ull,
        0x7fffffffffffffffull, 0x8000000000000000ull, 0xffffffffffffffffull,
        0xfffffffffffff800ull, 0xfffffffffffff7ffull, 0x123456789abcdef0ull,
        0xfedcba9876543210ull, 0x0000001000000000ull, 0x00000007fffffff8ull,
    };
    for (unsigned i = 0; i < sizeof edges / sizeof edges[0]; i++) {
        run(edges[i]);
        run(-edges[i]);
        run(edges[i] - 1);
    }
    u64 s = seed_box;
    for (int i = 0; i < 20000; i++) {
        s = s * 6364136223846793005ull + 1442695040888963407ull;
        run(s);
        run(s >> (i & 63));
    }
    put_hex(h);
    sys3(SYS_EXIT, 0, 0, 0);
    for (;;) {
    }
}
C

# ── 1. The digest, from the oracle ─────────────────────────────────────────
if command -v "$GCC" >/dev/null 2>&1; then
    "$GCC" -O2 -ffreestanding -nostdlib -static -fno-pie -no-pie "$tmp/t.c" -o "$tmp/gcc"
    got=$("$tmp/gcc")
    [ "$got" = "$want" ] || bad "GCC digest $got, pinned $want (the source changed?)"
else
    note "note: no $GCC; the pinned digest $want stands in for it"
fi

run_leg() { # $1 = binary, $2 = runner ("" = native), $3 = label, $4.. = options
    local bin=$1 runner=$2 label=$3 out
    shift 3
    for opt in "$@"; do
        "$bin" "$opt" -ffreestanding -nostdlib -static "$tmp/t.c" -o "$tmp/$label$opt"
        out=$(${runner:+$runner} "$tmp/$label$opt" || true)
        [ "$out" = "$want" ] || bad "$label $opt printed '${out}', want $want"
    done
}

# ── 2. Execution on every target ───────────────────────────────────────────
run_leg "$CCC" "" x86-64 -O0 -O1 -O2 -Os
CCC_PEEPHOLE_SKIP=abs_cmov run_leg "$CCC" "" x86-64-noabscmov -O2
CCC_DISABLE_PASSES=signmag run_leg "$CCC" "" x86-64-nosignmag -O2
if [ -x "$bindir/lccc-i686" ]; then
    run_leg "$bindir/lccc-i686" "" i686 -O0 -O2 -Os
else
    note "note: no lccc-i686; i686 leg skipped"
fi
for t in riscv:riscv64 arm:aarch64; do
    be=${t%%:*} q=qemu-${t##*:}
    if [ -x "$bindir/lccc-$be" ] && command -v "$q" >/dev/null 2>&1; then
        run_leg "$bindir/lccc-$be" "$q" "$be" -O0 -O1 -O2 -Os
    else
        note "note: no lccc-$be or $q; $be execution skipped"
    fi
done

# ── 3. The shapes ──────────────────────────────────────────────────────────
body() { # $1 = asm file, $2 = function: the instructions, one per line
    sed -n "/^$2:/,/^[[:space:]]*ret\$/p" "$1" |
        grep -E '^[[:space:]]+[a-z]' | grep -v -E '^[[:space:]]+\.' | sed 's/^[[:space:]]*//'
}
expect_body() { # $1 = asm, $2 = function, $3 = exact body (lines joined by ';')
    local got
    got=$(body "$1" "$2" | paste -sd ';' -)
    [ "$got" = "$3" ] || bad "$2 in $(basename "$1"): '$got', want '$3'"
}
# The body is captured first: `grep -q` on a pipe would stop reading early and
# SIGPIPE the producer under pipefail.
has() {
    local text
    text=$(body "$1" "$2")
    grep -q -E "$3" <<<"$text" || bad "$2 in $(basename "$1") lacks /$3/: $(paste -sd ';' - <<<"$text")"
}
lacks() {
    local text
    text=$(body "$1" "$2")
    if grep -q -E "$3" <<<"$text"; then
        bad "$2 in $(basename "$1") has /$3/: $(paste -sd ';' - <<<"$text")"
    fi
}

# x86-64: the oracles' three instructions, at -O2 and at -Os (where the
# vectorizer never formed the select before), straight into the return
# register; the kill switches bring the identity / the select back.
for opt in -O2 -Os; do
    "$CCC" "$opt" -S "$tmp/t.c" -o "$tmp/x$opt.s"
    for f in abs_lt abs_le abs_gt abs_ge; do
        expect_body "$tmp/x$opt.s" "$f" 'movl %edi, %eax;negl %eax;cmovs %edi, %eax;ret'
    done
    expect_body "$tmp/x$opt.s" abs64 'movq %rdi, %rax;negq %rax;cmovs %rdi, %rax;ret'
    expect_body "$tmp/x$opt.s" abs_plus \
        'movl %edi, %eax;negl %eax;cmovs %edi, %eax;addl %edi, %eax;ret'
    # One step away from a magnitude: no identity, no neg/cmovs pair.
    lacks "$tmp/x$opt.s" not_abs 'sarl'
    lacks "$tmp/x$opt.s" neg_abs 'sarl'
done
CCC_PEEPHOLE_SKIP=abs_cmov "$CCC" -O2 -S "$tmp/t.c" -o "$tmp/x-noabs.s"
has "$tmp/x-noabs.s" abs_lt 'sarl \$31'
lacks "$tmp/x-noabs.s" abs_lt 'cmovs'
CCC_DISABLE_PASSES=signmag "$CCC" -O2 -S "$tmp/t.c" -o "$tmp/x-nosm.s"
lacks "$tmp/x-nosm.s" abs_lt 'sarl'

if [ -x "$bindir/lccc-riscv" ]; then
    rv="$tmp/rv.s"
    "$bindir/lccc-riscv" -O2 -S "$tmp/t.c" -o "$rv"
    has "$rv" sra3l '^srai [a-z0-9]+, [a-z0-9]+, 3$'
    has "$rv" sra31 '^sraiw '
    has "$rv" srl3l '^srli [a-z0-9]+, [a-z0-9]+, 3$'
    has "$rv" shl40 '^slli [a-z0-9]+, [a-z0-9]+, 40$'
    has "$rv" add64 '^addi [a-z0-9]+, [a-z0-9]+, 100$'
    has "$rv" sub64 '^addi [a-z0-9]+, [a-z0-9]+, -2048$'
    has "$rv" and255 '^andi '
    has "$rv" xor64 '^xori [a-z0-9]+, [a-z0-9]+, -2048$'
    # Past the immediate range the constant is staged as before.
    has "$rv" add_past '^li [a-z0-9]+, 2048$'
    has "$rv" sub64_past '^li [a-z0-9]+, 2049$'
    for f in sra3l srl3l shl40 add64 sub64 xor64; do
        lacks "$rv" "$f" '^li '
    done
    # `(int)(x >> 3)`: the short `srai` is no sext32 producer.
    has "$rv" trunc_sra '^sext\.w '
    # The identity reaches RISC-V: no branch, a sign mask.
    has "$rv" abs_lt '^sraiw [a-z0-9]+, [a-z0-9]+, 31$'
    lacks "$rv" abs_lt '^(beqz|bnez|beq|bne|blt|bge|j) '
fi
if [ -x "$bindir/lccc-arm" ]; then
    a64="$tmp/a64.s"
    "$bindir/lccc-arm" -O2 -S "$tmp/t.c" -o "$a64"
    # The system-call operands survive the peephole.
    sed -n '/^entry:/,/^\.size entry/p' "$a64" > "$tmp/entry.s"
    grep -q 'movz x8, #64' "$tmp/entry.s" || bad "AArch64 entry lost the write number (x8 = 64)"
    grep -q 'movz x8, #93' "$tmp/entry.s" || bad "AArch64 entry lost the exit number (x8 = 93)"
    has "$a64" abs_lt '^asr '
    lacks "$a64" abs_lt '^(csel|cset|cneg) '
fi
if [ -x "$bindir/lccc-i686" ]; then
    i686="$tmp/i686.s"
    "$bindir/lccc-i686" -O2 -S "$tmp/t.c" -o "$i686"
    has "$i686" abs_lt 'sarl \$31'
    lacks "$i686" abs_lt '^j[a-z]+ '
fi

[ "$fail" -eq 0 ] || exit 1
echo "check_cross_alu_abs_runtime: PASS (one digest on x86-64, i686, RISC-V and AArch64; I-type selection, sext32 producers, sign-magnitude on every target, abs_cmov, AArch64 inline-asm quarantine)"
