#!/usr/bin/env bash
# GAS location-counter assignment, including Linux kexec-style fixed slots.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cc=${CCC:-$repo/target/fastbuild/lccc}
as=${AS_ORACLE:-as}
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cat > "$tmp/vector.s" <<'ASM'
.text
.globl vector_base, done
vector_base:
.macro vector n
 . = vector_base + (\n * 6)
 nop
 jmp done
.endm
vector 0
vector 1
vector 2
vector 3
done:
.byte 0x5a
. = . + 2
.byte 0x66
ASM
cat > "$tmp/absolute.s" <<'ASM'
.data
.byte 1
. = 8
.byte 2
ASM
cat > "$tmp/back.s" <<'ASM'
.data
.byte 1, 2
. = 1
ASM
cat > "$tmp/cross.s" <<'ASM'
.text
other: nop
.data
. = other + 8
ASM
for bits in 32 64; do
  for fixture in vector absolute; do
    "$as" --"$bits" "$tmp/$fixture.s" -o "$tmp/gas.o"
    "$cc" -m"$bits" -c "$tmp/$fixture.s" -o "$tmp/lccc.o"
    section=.text; [[ $fixture == absolute ]] && section=.data
    objcopy -O binary --only-section="$section" "$tmp/gas.o" "$tmp/gas.bin"
    objcopy -O binary --only-section="$section" "$tmp/lccc.o" "$tmp/lccc.bin"
    cmp "$tmp/gas.bin" "$tmp/lccc.bin"
  done
  for fixture in back cross; do
    if "$as" --"$bits" "$tmp/$fixture.s" -o "$tmp/gas.o" 2>/dev/null; then
      echo "FAIL: oracle accepted $fixture" >&2; exit 1
    fi
    if "$cc" -m"$bits" -c "$tmp/$fixture.s" -o "$tmp/lccc.o" 2>/dev/null; then
      echo "FAIL: lccc accepted $fixture ($bits-bit)" >&2; exit 1
    fi
  done
done
echo 'PASS: x86 location assignment agrees with GAS for fixed slots and errors'
