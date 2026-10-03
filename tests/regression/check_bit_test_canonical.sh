#!/usr/bin/env bash
# Canonical `(x >> i) & 1` lowering.
#
# This is a cross-target canonical IR operation: x86 should use BT, AArch64
# should use UBFX for a constant index, and every target must preserve the
# low-bit result for both signed and unsigned right shifts.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/bt.c" <<'EOF'
unsigned var(unsigned x, unsigned i) { return (x >> i) & 1u; }
unsigned signed_var(int x, unsigned i) { return (x >> i) & 1u; }
unsigned constbit(unsigned x) { return (x >> 3) & 1u; }
EOF

"$CCC" -O3 -S "$td/bt.c" -o "$td/x86.s"
# Register-index BT form.  BT's index is an ordinary r/m operand (it has no
# fixed count register, unlike variable shifts), so the index is consumed
# straight from its own register (`btl %edx, %r8`) rather than staged through
# %rcx.  Assert the general register-index shape, not the staging register.
grep -Eq 'bt[lq][[:space:]]+%[a-z0-9]+,[[:space:]]*%[a-z0-9]+' "$td/x86.s"
grep -Eq 'btl[[:space:]]+\$3, %' "$td/x86.s"
# The source has no main; only assemble/codegen is required here. A
# separate runtime harness links the object below when the host linker is
# available.
"$CCC" -O3 -c "$td/bt.c" -o "$td/bt.o"
if command -v cc >/dev/null 2>&1; then
  cat >"$td/driver.c" <<'DRIVER'
#include <stdio.h>
unsigned var(unsigned x, unsigned i);
unsigned constbit(unsigned x);
int main(void) {
  for (unsigned x = 0; x < 80; x += 7)
    for (unsigned i = 0; i < 40; i++)
      if (var(x, i) != ((x >> i) & 1u)) return 2;
  for (unsigned x = 0; x < 256; x++)
    if (constbit(x) != ((x >> 3) & 1u)) return 3;
  return 0;
}
DRIVER
  cc "$td/bt.o" "$td/driver.c" -o "$td/bt"
  "$td/bt"
fi

if [ -x "./target/fastbuild/lccc-arm" ]; then
  ./target/fastbuild/lccc-arm -O3 -S "$td/bt.c" -o "$td/arm.s"
  grep -q 'ubfx' "$td/arm.s"
fi
if [ -x "./target/fastbuild/lccc-riscv" ]; then
  ./target/fastbuild/lccc-riscv -O3 -S "$td/bt.c" -o "$td/riscv.s"
  grep -Eq 'srlw|srli' "$td/riscv.s"
  grep -q 'andi' "$td/riscv.s"
fi
echo "OK bit_test_canonical"
