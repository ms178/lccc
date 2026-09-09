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
! grep -q 'lock inc' <<<"$add"
grep -q 'negq %rax' <<<"$sub"
grep -q 'lock xaddb %al' <<<"$sub"
grep -q 'movsbq %al, %rax' <<<"$sub"
! grep -q 'cmpxchg' <<<"$sub"
grep -q 'andq %rdi, %rdx' <<<"$and"
grep -q 'lock cmpxchgb %dl' <<<"$and"
grep -q 'movsbq %al, %rax' <<<"$and"
grep -q 'movsbq %al, %rax' <<<"$cas"
grep -q 'xchgl %edx' <<<"$store"
! grep -q 'mfence' <<<"$store"
grep -q 'lock orq \$0, (%rsp)' <<<"$fence"
! grep -q 'mfence' <<<"$fence"

if [ -x "$arm" ]; then
    "$arm" -O2 -S "$td/probe.c" -o "$td/arm.s"
    sub=$(body "$td/arm.s" sub8)
    cas=$(body "$td/arm.s" cas8)
    release=$(body "$td/arm.s" fence_release)
    grep -q 'sxtb x0, w0' <<<"$sub"
    grep -q 'and w2, w2, #0xff' <<<"$cas"
    grep -q 'sxtb x0, w0' <<<"$cas"
    grep -q 'dmb ish' <<<"$release"
    ! grep -q 'dmb ishst' <<<"$release"
    if command -v aarch64-linux-gnu-gcc >/dev/null; then
        aarch64-linux-gnu-gcc -c "$td/arm.s" -o "$td/arm.o"
    fi
fi

if [ -x "$riscv" ]; then
    "$riscv" -O2 -S "$td/probe.c" -o "$td/riscv.s"
    casu=$(body "$td/riscv.s" casu32)
    load=$(body "$td/riscv.s" loadu32)
    store=$(body "$td/riscv.s" storeu32)
    grep -q 'sext.w t2' <<<"$casu"
    grep -q 'slli t0, t0, 32' <<<"$casu"
    grep -q 'srli t0, t0, 32' <<<"$casu"
    grep -q 'lwu t0, 0(t0)' <<<"$load"
    ! grep -q 'lr.w' <<<"$load"
    grep -q 'sw t1, 0(t0)' <<<"$store"
    ! grep -q 'amoswap' <<<"$store"
    [ "$(grep -c 'fence rw' <<<"$store")" -eq 2 ]
    if command -v riscv64-linux-gnu-gcc >/dev/null; then
        riscv64-linux-gnu-gcc -c "$td/riscv.s" -o "$td/riscv.o"
    fi
fi

echo 'cross-backend atomic code-generation contract: PASS'
