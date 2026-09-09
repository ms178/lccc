#!/usr/bin/env bash
# Structural oracle for the i686 atomic lowering. Runtime semantics are covered
# by i686_atomics.c; this gate prevents correct but materially worse sequences
# (CAS subtraction and MOV+MFENCE SeqCst stores) from returning.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/probe.c" <<'EOF'
typedef signed char i8;
typedef unsigned char u8;
typedef unsigned short u16;
__attribute__((noinline)) i8 sub8(i8 *p, i8 v) {
    return __atomic_fetch_sub(p, v, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) u16 and16(u16 *p, u16 v) {
    return __atomic_fetch_and(p, v, __ATOMIC_RELAXED);
}
__attribute__((noinline)) u8 cas_value8(u8 *p, u8 e, u8 d) {
    return __sync_val_compare_and_swap(p, e, d);
}
__attribute__((noinline)) void store8(u8 *p, u8 v) {
    __atomic_store_n(p, v, __ATOMIC_SEQ_CST);
}
__attribute__((noinline)) void fence_all(void) {
    __atomic_thread_fence(__ATOMIC_ACQUIRE);
    __atomic_thread_fence(__ATOMIC_RELEASE);
    __atomic_thread_fence(__ATOMIC_SEQ_CST);
}
EOF

"$ccc" -m32 -O2 -fno-pic -S "$td/probe.c" -o "$td/probe.s"
gcc -m32 -c "$td/probe.s" -o "$td/probe.o"

function_body() {
    local name=$1
    sed -n "/^${name}:/,/^\\.size ${name},/p" "$td/probe.s"
}

sub=$(function_body sub8)
and=$(function_body and16)
cas=$(function_body cas_value8)
store=$(function_body store8)
fences=$(function_body fence_all)

grep -q 'negl %edx' <<<"$sub"
grep -q 'lock xaddb %dl' <<<"$sub"
! grep -q 'cmpxchg' <<<"$sub"

grep -q 'andl (%esp), %edx' <<<"$and"
grep -q 'lock cmpxchgw %dx' <<<"$and"
! grep -qE 'andw|andb' <<<"$and"

grep -q 'lock cmpxchgb %dl' <<<"$cas"
grep -q 'movzbl %al, %eax' <<<"$cas"

grep -q 'xchgb %dl' <<<"$store"
! grep -q 'mfence' <<<"$store"

# Three non-relaxed source fences, each using the generic-i686 locked stack
# operation used by GCC and Clang. No SSE2-only MFENCE or scratch-register
# sequence may leak in.
[ "$(grep -c 'lock orl \$0, (%esp)' <<<"$fences")" -eq 3 ]
! grep -qE 'mfence|xchgl|pushfl|popfl' <<<"$fences"

echo 'i686 atomic code-generation contract: PASS'
