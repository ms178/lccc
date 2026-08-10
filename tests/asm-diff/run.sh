#!/usr/bin/env bash
set -u
LCCC=${1:-./target/release/lccc}; GAS=${2:-as}; dir=$(cd "$(dirname "$0")" && pwd)
tool=$(dirname "$GAS"); OBJCOPY=${OBJCOPY:-$tool/../binutils/objcopy}; [ -x "$OBJCOPY" ] || OBJCOPY=objcopy
fail=0
for src in "$dir"/*.s; do
 b=$(basename "$src" .s); lo=/tmp/lccc-$b.o; go=/tmp/gas-$b.o
 "$LCCC" -c "$src" -o "$lo" || { echo "FAIL $b LCCC"; fail=1; continue; }
 "$GAS" --64 -o "$go" "$src" || { echo "FAIL $b GAS"; fail=1; continue; }
 "$OBJCOPY" -O binary --only-section=.text "$lo" /tmp/lccc-$b.text
 "$OBJCOPY" -O binary --only-section=.text "$go" /tmp/gas-$b.text
 if cmp -s /tmp/lccc-$b.text /tmp/gas-$b.text; then echo "PASS $b raw .text"; else echo "FAIL $b bytes"; fail=1; fi
done
[ $fail -eq 0 ] && echo '=== ALL RAW .text BYTES EXACT ==='
# Malformed operands must be rejected just as GAS rejects them.
invalid=(
  'paddb %mm2, %xmm1'
  'vpaddb %ymm1, %ymm0'
  'vbroadcasti128 %xmm0, %ymm0'
  'vmovdqu %ymm0, %xmm1'
  'paddb %xmm32, %xmm1'
)
for inst in "${invalid[@]}"; do
  printf '.text\n%s\n' "$inst" >/tmp/asm-invalid.s
  if "$LCCC" -c /tmp/asm-invalid.s -o /tmp/asm-invalid-lccc.o >/dev/null 2>&1; then
    echo "FAIL false accept: $inst"; fail=1
  elif "$GAS" --64 -o /tmp/asm-invalid-gas.o /tmp/asm-invalid.s >/dev/null 2>&1; then
    echo "FAIL GAS accepted control: $inst"; fail=1
  else
    echo "PASS reject: $inst"
  fi
done
exit $fail
