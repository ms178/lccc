#!/usr/bin/env bash
# Small integer returns are in %eax with bits 32..63 zero (SysV AMD64).
# Reject 64-bit GPR copies in a leaf boolean/classifier return path; they
# introduce unnecessary REX.W and a 64-bit return dependency.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/r32.c" <<'EOF'
int f(int c) { return c >= 65; }
int g(int c) { if (c >= 65) return 1; return 0; }
EOF
"$CCC" -O3 -march=x86-64-v3 -S "$td/r32.c" -o "$td/r32.s"
if sed -n '/^f:/,/^\.size[[:space:]]*f/p' "$td/r32.s" | grep -Eq 'movq[[:space:]]+%r[a-z0-9]+,[[:space:]]*%rax'; then
  echo "32-bit return used 64-bit GPR copy:" >&2
  sed -n '/^f:/,/^\.size/p' "$td/r32.s" >&2
  exit 1
fi
# Constant-return lowering should already use 32-bit moves.
grep -q 'movl $1, %eax' "$td/r32.s"
grep -q 'xorl %eax, %eax' "$td/r32.s"
cat >"$td/run.c" <<'EOF'
#include <stdio.h>
int f(int c);
int main(void) {
  int ok = (f(64) == 0 && f(65) == 1 && f(127) == 1);
  puts(ok ? "OK return_zeroext" : "BAD return_zeroext");
  return ok ? 0 : 2;
}
EOF
"$CCC" -O3 "$td/r32.c" "$td/run.c" -o "$td/run"
"$td/run"
