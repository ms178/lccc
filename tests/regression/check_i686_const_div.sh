#!/usr/bin/env bash
# i686 constant division/remainder strength reduction: magic numbers and
# signed power-of-two bias must be bit-exact vs gcc (toward-zero semantics),
# and the sequences must NOT use `divl`/`idivl` at -O2 (the whole point of the
# strength reduction).
set -uo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
typedef unsigned int u32;
typedef int i32;
__attribute__((noinline)) u32 ud3(u32 x){return x/3;}
__attribute__((noinline)) u32 ur7(u32 x){return x%7;}
__attribute__((noinline)) i32 sd3(i32 x){return x/3;}
__attribute__((noinline)) i32 sr3(i32 x){return x%3;}
__attribute__((noinline)) i32 sd8(i32 x){return x/8;}
__attribute__((noinline)) i32 sdn5(i32 x){return x/-5;}
void _start(void) {
  u32 acc = 0;
  for (u32 i = 1; i < 30000; i++) {
    i32 s = (i32)i - 15000;
    acc = (acc + ud3(i) + ur7(i) + (u32)sd3(s) + (u32)sr3(s) + (u32)sd8(s) + (u32)sdn5(s)) & 255;
  }
  __asm__ volatile("movl %0, %%ebx; movl $1, %%eax; int $0x80" :: "r"(acc) : "eax","ebx");
}
EOF
"$CCC" -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/test.c" -o "$td/lccc_bin"
gcc -m32 -O2 -nostdlib -static -Wl,-e,_start "$td/test.c" -o "$td/gcc_bin"
"$td/lccc_bin"; l=$?
"$td/gcc_bin"; g=$?
if [ "$l" -ne "$g" ]; then
    echo "i686 const-div mismatch: lccc exit=$l gcc exit=$g"
    exit 1
fi
# The whole point: no divl/idivl at -O2 (magic mul / shift / and instead).
"$CCC" -m32 -O2 -nostdlib -static -S "$td/test.c" -o "$td/test.s"
body=$(sed -n '/^ud3:/,/^\.size ud3/p;/^sd3:/,/^\.size sd3/p' "$td/test.s")
if grep -Eq 'divl|idivl' <<<"$body"; then
    echo "constant division still uses divl/idivl at -O2"
    echo "--- $body"
    exit 1
fi
if ! grep -Eq 'mull|imull' <<<"$body"; then
    echo "expected magic multiply for constant division"
    exit 1
fi
