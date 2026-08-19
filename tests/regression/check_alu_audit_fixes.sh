#!/usr/bin/env bash
# i686 alu.rs audit fixes:
#  1. f32 negation must be a single `xorl $0x80000000, %eax` bit flip (the
#     old SSE round-trip was 5 instructions) and must stay bit-exact vs gcc.
#  2. shift counts >= 256 must be masked to 5 bits (imm8 encoding; GAS would
#     reject `shll $300`).
set -uo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

# --- 1. f32 negation ---
cat >"$td/neg.c" <<'EOF'
typedef float f32;
typedef unsigned int u32;
__attribute__((noinline)) f32 negf(f32 x) { return -x; }
void _start(void) {
  u32 acc = 0;
  for (u32 i = 1; i < 1000; i++) {
    f32 x = (f32)i * 1.5f;
    f32 y = negf(x);
    acc ^= *(u32*)&y; acc &= 255;
  }
  __asm__ volatile("movl %0, %%ebx; movl $1, %%eax; int $0x80" :: "r"(acc) : "eax","ebx");
}
EOF
"$CCC" -m32 -O2 -nostdlib -static -S "$td/neg.c" -o "$td/neg.s"
body=$(sed -n '/^negf:/,/^\.size negf/p' "$td/neg.s")
if ! grep -Eq 'xorl[[:space:]]+\$0x80000000,[[:space:]]*%eax' <<<"$body"; then
    echo "f32 neg is not a single xorl"
    echo "--- $body"
    exit 1
fi
if grep -Eq 'xorps|movd' <<<"$body"; then
    echo "f32 neg still uses the SSE round-trip"
    echo "--- $body"
    exit 1
fi
"$CCC" -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/neg.c" -o "$td/neg_l"
gcc -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/neg.c" -o "$td/neg_g"
"$td/neg_l"; l=$?
"$td/neg_g"; g=$?
[ "$l" -eq "$g" ] || { echo "f32 neg mismatch: lccc=$l gcc=$g"; exit 1; }

# --- 2. shift masking ---
cat >"$td/shift.c" <<'EOF'
typedef unsigned int u32;
__attribute__((noinline)) u32 shl300(u32 x) { return x << 300; }
void _start(void) {
  u32 acc = 0;
  for (u32 i = 0; i < 256; i++) acc = (acc + shl300(i)) & 255;
  __asm__ volatile("movl %0, %%ebx; movl $1, %%eax; int $0x80" :: "r"(acc) : "eax","ebx");
}
EOF
"$CCC" -m32 -O2 -nostdlib -static -S "$td/shift.c" -o "$td/shift.s"
sbody=$(sed -n '/^shl300:/,/^\.size shl300/p' "$td/shift.s")
# The encoded shift count must be 300 & 31 == 12, never a raw 300 (imm8 overflow).
if grep -Eq 'shll[[:space:]]+\$300' <<<"$sbody"; then
    echo "shift count 300 not masked (imm8 overflow)"
    exit 1
fi
"$CCC" -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/shift.c" -o "$td/shift_l"
gcc -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/shift.c" -o "$td/shift_g"
"$td/shift_l"; l=$?
"$td/shift_g"; g=$?
[ "$l" -eq "$g" ] || { echo "shift>255 mismatch: lccc=$l gcc=$g"; exit 1; }
