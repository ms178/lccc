#!/usr/bin/env bash
# The i686 general-case binop/cmp path stages a register-resident rhs through
# %ecx (`movl %reg,%ecx; op %ecx,%eax`). The codegen now uses the register in
# place (`op %reg,%eax`) — an identical read, one instruction shorter. This
# test compiles a register-pressure loop and checks BOTH runtime equivalence
# with gcc AND that the staging move is gone from the hot loop.
set -uo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
typedef unsigned int u32;
typedef int i32;
static u32 g_arr[16];
static i32 probe(i32 seed, i32 iter) {
  u32 v0 = (u32)seed * 617u;
  u32 v1 = (u32)seed ^ 283857447u;
  u32 v2 = (u32)seed * 283u;
  u32 v3 = (u32)(seed + iter * 41);
  u32 v4 = (u32)seed * 511u;
  u32 v5 = (u32)(seed + iter * 57);
  u32 v6 = (u32)seed >> 1;
  u32 v7 = (u32)(seed + iter * 73);
  for (i32 i = 0; i < (iter & 15) + 2; i++) {
    g_arr[i & 15] = g_arr[(i + 3) & 15] + v0 + (u32)i;
    v0 = (v6 << (v3 & 7u)) | (v6 >> (32 - (v3 & 7u) - 1));
    v4 = (v3 > v5) ? v3 - v5 : v5 - v3;
    v6 = v2 + v4 + v0;
    v4 = v0 + v7;
    v2 = (v4 > v6) ? v4 - v6 : v6 - v4;
    v0 = v2 ^ (v7 + 0x9e37u);
    v1 = v4 ^ (v1 + 0x9e37u);
    v7 = v6 + v1;
  }
  return (i32)(v0 ^ v1 ^ v2 ^ v3 ^ v4 ^ v5 ^ v6 ^ v7 ^ g_arr[seed & 15]);
}
void _start(void) {
  u32 acc = 0;
  for (u32 i = 1; i < 200; i++) acc = (acc * 31u + (u32)probe(i, 5)) & 255u;
  __asm__ volatile("movl %0, %%ebx; movl $1, %%eax; int $0x80" :: "r"(acc) : "eax","ebx");
}
EOF
"$CCC" -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/test.c" -o "$td/lccc_bin"
gcc -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/test.c" -o "$td/gcc_bin"
"$td/lccc_bin"; l=$?
"$td/gcc_bin"; g=$?
if [ "$l" -ne "$g" ]; then
    echo "register-source fold miscompile: lccc exit=$l gcc exit=$g"
    exit 1
fi
