#!/usr/bin/env bash
# Cross-backend atomic correctness/code-quality contract.
set -euo pipefail
cd "$(dirname "$0")/../.."
bindir=$(dirname "${LCCC_BIN:-target/fastbuild/lccc}")
x86=${LCCC_BIN:-$bindir/lccc}
arm=$bindir/lccc-arm
riscv=$bindir/lccc-riscv
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/probe.c" <<'EOF'
typedef signed char i8;
typedef unsigned char u8;
typedef unsigned u32;
typedef unsigned long long u64;
__attribute__((noinline)) u64 add_one(u64 *p) {
    return __atomic_fetch_add(p, 1, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) i8 sub8(i8 *p, i8 v) {
    return __atomic_fetch_sub(p, v, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) i8 and8(i8 *p, i8 v) {
    return __atomic_fetch_and(p, v, __ATOMIC_RELAXED);
}
__attribute__((noinline)) i8 cas8(i8 *p, i8 e, i8 d) {
    return __sync_val_compare_and_swap(p, e, d);
}
__attribute__((noinline)) u32 casu32(u32 *p, u32 e, u32 d) {
    return __sync_val_compare_and_swap(p, e, d);
}
__attribute__((noinline)) u64 casu32_wide(u32 *p, u32 e, u32 d) {
    return __sync_val_compare_and_swap(p, e, d);
}
__attribute__((noinline)) u32 loadu32(u32 *p) {
    return __atomic_load_n(p, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) void storeu32(u32 *p, u32 v) {
    __atomic_store_n(p, v, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) void fence_release(void) {
    __atomic_thread_fence(__ATOMIC_RELEASE);
}
__attribute__((noinline)) void fence_sc(void) {
    __atomic_thread_fence(__ATOMIC_SEQ_CST);
}
__attribute__((noinline)) __int128 ld128(__int128 *p) {
    return __atomic_load_n(p, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) __int128 add128(__int128 *p, __int128 v) {
    return __atomic_fetch_add(p, v, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) int cas128(__int128 *p, __int128 *e, __int128 d) {
    return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) float xchgf(float *p, float v) {
    float o;
    __atomic_exchange(p, &v, &o, __ATOMIC_SEQ_CST);
    return o;
}
__attribute__((noinline)) void unlock(int *p) {
    __sync_lock_release(p);
}
EOF

body() {
    local file=$1 name=$2
    sed -n "/^${name}:/,/^\\.size[[:space:]]\+${name},/p" "$file"
}

"$x86" -O2 -fno-pic -S "$td/probe.c" -o "$td/x86.s"
gcc -c "$td/x86.s" -o "$td/x86.o"
add=$(body "$td/x86.s" add_one)
sub=$(body "$td/x86.s" sub8)
and=$(body "$td/x86.s" and8)
cas=$(body "$td/x86.s" cas8)
store=$(body "$td/x86.s" storeu32)
fence=$(body "$td/x86.s" fence_sc)
grep -q 'lock xaddq' <<<"$add"
! grep -q 'lock inc' <<<"$add" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
grep -q 'negq %rax' <<<"$sub"
grep -q 'lock xaddb %al' <<<"$sub"
# need/deny: report the failing pin instead of a silent `set -e` exit.
need() { grep -qE -- "$2" <<<"$3" || { echo "FAIL: $1 lacks /$2/" >&2; printf '%s\n' "$3" >&2; exit 1; }; }
deny() { ! grep -qE -- "$2" <<<"$3" || { echo "FAIL: $1 has /$2/" >&2; printf '%s\n' "$3" >&2; exit 1; }; }
# An i8 result is sign-extended once: `movsbl %al, %eax` (3 bytes) is the
# whole canonicalisation; `movsbq` + `movl %eax, %eax` was two instructions.
need sub8 '^[[:space:]]*movsb[lq] %al, %[er]ax$' "$sub"
deny sub8 'cmpxchg' "$sub"
# The bitwise CMPXCHG loop reads its operand in place: `v` stays in its ABI
# register (%rsi), nothing is parked in %rdi, and no callee-saved register
# or frame is bought (GCC 16.2: 8 instructions, no stack).
need and8 '^[[:space:]]*andq %rsi, %rdx$' "$and"
need and8 'lock cmpxchgb %dl' "$and"
need and8 '^[[:space:]]*movsb[lq] %al, %[er]ax$' "$and"
deny and8 '(pushq|subq \$[0-9]+, %rsp|, %rdi$|, %edi$)' "$and"
need cas8 '^[[:space:]]*movsb[lq] %al, %[er]ax$' "$cas"
deny cas8 '(pushq|subq \$[0-9]+, %rsp)' "$cas"
grep -q 'xchgl %edx' <<<"$store"
! grep -q 'mfence' <<<"$store" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
grep -q 'lock orq \$0, (%rsp)' <<<"$fence"
! grep -q 'mfence' <<<"$fence" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }

if [ -x "$arm" ]; then
    "$arm" -O2 -S "$td/probe.c" -o "$td/arm.s"
    sub=$(body "$td/arm.s" sub8)
    cas=$(body "$td/arm.s" cas8)
    release=$(body "$td/arm.s" fence_release)
    # AAPCS64 leaves the bits of x0 above an 8-bit result unspecified (C.16
    # and "Result return"), so the callee returns the byte the exclusive
    # load left in w0 without re-extending it -- Clang 23.1.5 ends both
    # functions with `mov w0, w8; ret` -- and every caller extends it
    # (executed by int_mul_const_and_fp_int_cast_cross check 16).
    grep -q 'ldaxrb' <<<"$sub" && grep -q 'stlxrb' <<<"$sub"
    ! grep -Eq '^[[:space:]]*sxtb x0, w0$' <<<"$sub" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
    # The expected value is masked to its byte (`and wN, wM, #0xff`, read
    # from whichever register holds it): the CAS loop compares the
    # zero-extended `ldaxrb` result against it.
    need cas8 'and w[0-9]+, w[0-9]+, #0xff' "$cas"
    ! grep -Eq '^[[:space:]]*sxtb x0, w0$' <<<"$cas" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
    grep -q 'dmb ish' <<<"$release"
    ! grep -q 'dmb ishst' <<<"$release" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
    # The pointer of every atomic is a register-homed value read in place:
    # no frame, no stack round trip of the pointer, as Clang 23.1.5 and
    # GCC 16.2 (`ldar w0, [x0]; ret`).
    frame='(stp x29|\[sp|ldr x[0-9]+, \[sp)'
    load=$(body "$td/arm.s" loadu32)
    store=$(body "$td/arm.s" storeu32)
    casu=$(body "$td/arm.s" casu32)
    casw=$(body "$td/arm.s" casu32_wide)
    add=$(body "$td/arm.s" add_one)
    need loadu32 '^[[:space:]]*ldar w0, \[x0\]$' "$load"
    deny loadu32 "$frame" "$load"
    need storeu32 '^[[:space:]]*stlr w1, \[x0\]$' "$store"
    deny storeu32 "$frame" "$store"
    need casu32 'ldaxr w0, \[x[0-9]+\]' "$casu"
    need casu32 'stlxr w4, w2, \[x[0-9]+\]' "$casu"
    deny casu32 "$frame" "$casu"
    # GCC 16.2 and the Linux LL/SC cmpxchg leave the monitor armed on
    # failure; the success path pays no `b` around a CLREX.
    deny casu32 '(clrex|^[[:space:]]*b \.)' "$casu"
    # The exclusive W load zero-extends: the u32 -> u64 widening is free.
    deny casu32_wide 'mov w0, w0' "$casw"
    # An imm12 addend is the ADD's immediate.
    need add_one '^[[:space:]]*add x3, x0, #1$' "$add"
    deny add_one "$frame" "$add"
    # Zero is stored from the zero register.
    need unlock '^[[:space:]]*stlr wzr, \[x0\]$' "$(body "$td/arm.s" unlock)"
    # 16 bytes: LDAXP/STLXP loops touching both halves (Clang's expansion
    # without LSE); the low doubleword alone used to be loaded/added and a
    # stack slot returned as the high half.
    ld128=$(body "$td/arm.s" ld128)
    add128=$(body "$td/arm.s" add128)
    cas128=$(body "$td/arm.s" cas128)
    need ld128 'ldaxp x0, x1, \[x9\]' "$ld128"
    need ld128 'stlxp w12, x0, x1, \[x9\]' "$ld128"
    need add128 'adds x16, x0, x2' "$add128"
    need add128 'adc x17, x1, x3' "$add128"
    need add128 'stlxp w12, x16, x17, \[x9\]' "$add128"
    need cas128 'ccmp x1, x11, #0, eq' "$cas128"
    # A 4-byte float exchanges 4 bytes (it used to take an X pair).
    xf=$(body "$td/arm.s" xchgf)
    need xchgf 'ldaxr w0, ' "$xf"
    need xchgf 'stlxr w4, w[0-9]+, ' "$xf"
    deny xchgf 'ldaxr x0' "$xf"
    if command -v aarch64-linux-gnu-gcc >/dev/null; then
        aarch64-linux-gnu-gcc -c "$td/arm.s" -o "$td/arm.o"
    fi
fi

if [ -x "$riscv" ]; then
    "$riscv" -O2 -S "$td/probe.c" -o "$td/riscv.s"
    casu=$(body "$td/riscv.s" casu32)
    casw=$(body "$td/riscv.s" casu32_wide)
    load=$(body "$td/riscv.s" loadu32)
    store=$(body "$td/riscv.s" storeu32)
    add=$(body "$td/riscv.s" add_one)
    frame='addi sp, sp'
    # Word/doubleword atomics read their operands in place, so an atomic
    # leaf is frameless (the pointer used to be parked in a stack slot).
    # LR/SC orderings follow the RVWMO mapping (Table A.6): `lr.w.aqrl` +
    # `sc.w.rl` for seq_cst.  `lr.w` sign-extends the loaded word, and the
    # expected value is compared in that form: a u32 argument arrives
    # sign-extended under the psABI, so a1 is compared as is -- and the
    # loaded word is already the psABI form of the `unsigned` result.
    need casu32 'lr\.w\.aqrl t0, \(a0\)' "$casu"
    need casu32 'bne t0, a1, ' "$casu"
    need casu32 'sc\.w\.rl t4, a2, \(a0\)' "$casu"
    need casu32 '^[[:space:]]+mv a0, t0$' "$casu"
    deny casu32 "($frame|sext\.w|andi|lwu|sc\.w\.aq)" "$casu"
    # Widened to u64, the u32 truncation is the two-shift idiom; the
    # desired value's own zero extension is undemanded (`sc.w` stores its
    # low word).
    need casu32_wide 'slli a0, t0, 32' "$casw"
    need casu32_wide 'srli a0, a0, 32' "$casw"
    deny casu32_wide "($frame|slli a2|andi|lwu)" "$casw"
    # An acquire/seq_cst word load is `lw` between the fences: the u32's
    # psABI result form is the sign-extended word.
    need loadu32 '^[[:space:]]+lw a0, 0\(a0\)$' "$load"
    deny loadu32 "($frame|lwu|lr\.w|sext\.w)" "$load"
    need storeu32 '^[[:space:]]+sw a1, 0\(a0\)$' "$store"
    deny storeu32 "($frame|amoswap)" "$store"
    [ "$(grep -c 'fence rw' <<<"$store")" -eq 2 ]
    need add_one 'amoadd\.d\.aqrl [a-z0-9]+, t2, \(a0\)' "$add"
    deny add_one "$frame" "$add"
    need unlock '^[[:space:]]+sw zero, 0\(a0\)$' "$(body "$td/riscv.s" unlock)"
    # 16 bytes: libatomic calls, as GCC 16.2 and Clang 23.1.5 (the
    # low doubleword alone used to be loaded or added inline).
    need ld128 'call __atomic_load_16' "$(body "$td/riscv.s" ld128)"
    need add128 'call __atomic_fetch_add_16' "$(body "$td/riscv.s" add128)"
    need cas128 'call __atomic_compare_exchange_16' "$(body "$td/riscv.s" cas128)"
    # A 4-byte float exchanges 4 bytes (it used to take `amoswap.d`).
    need xchgf 'amoswap\.w\.aqrl ' "$(body "$td/riscv.s" xchgf)"
    if command -v riscv64-linux-gnu-gcc >/dev/null; then
        riscv64-linux-gnu-gcc -c "$td/riscv.s" -o "$td/riscv.o"
    fi
fi

# TRACE must observe the same execution as GEN: CHECK expressions include
# exchanges and compare-exchanges, so evaluating one twice changes the state
# and corrupts every subsequent diagnostic. Compare all group hashes, not
# just the first, and require actual trace rows to catch a vacuous pass.
fixture=tests/regression/atomic_ops_cross.c
"$x86" -O2 -DGEN "$fixture" -latomic -o "$td/gen"
"$x86" -O2 -DGEN -DTRACE "$fixture" -latomic -o "$td/trace"
"$td/gen" > "$td/gen.out"
"$td/trace" > "$td/trace.out"
grep -E '^    0x[0-9a-f]{16}ull,$' "$td/gen.out" > "$td/gen.hash"
grep -E '^    0x[0-9a-f]{16}ull,$' "$td/trace.out" > "$td/trace.hash"
grep -qE '^[0-9]+: -?[0-9]+$' "$td/trace.out"
[ "$(wc -l < "$td/gen.hash")" -eq 38 ]
cmp "$td/gen.hash" "$td/trace.hash"

echo 'cross-backend atomic code-generation and trace contract: PASS'
