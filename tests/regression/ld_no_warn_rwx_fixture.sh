#!/usr/bin/env bash
# Kernel Kbuild combines --fatal-warnings with --no-warn-rwx-segments.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cc=${LCCC:-$repo/target/fastbuild/lccc}
ld=${LCCC_LD:-$repo/target/fastbuild/lccc-ld}
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cat > "$tmp/a.s" <<'ASM'
.global _start
.text
_start:
  ret
.data
.long 1
.section .note.GNU-stack,"",@progbits
ASM
cat > "$tmp/a.ld" <<'LDS'
ENTRY(_start)
SECTIONS { . = 0x10000; .text : { *(.text) } .data : { *(.data) } }
LDS
"$cc" -m32 -c "$tmp/a.s" -o "$tmp/a.o"
"$ld" -m elf_i386 -T "$tmp/a.ld" "$tmp/a.o" -o "$tmp/base"
for order in first last; do
  if [[ $order == first ]]; then
    flags=(--fatal-warnings --no-warn-rwx-segments)
  else
    flags=(--no-warn-rwx-segments --fatal-warnings)
  fi
  "$ld" -m elf_i386 "${flags[@]}" -T "$tmp/a.ld" "$tmp/a.o" -o "$tmp/$order" 2>"$tmp/$order.err"
  [[ ! -s "$tmp/$order.err" ]]
  cmp "$tmp/base" "$tmp/$order"
done
# Explicitly requesting an unsupported warning must not silently become a no-op.
if "$ld" -m elf_i386 --fatal-warnings --warn-rwx-segments -T "$tmp/a.ld" "$tmp/a.o" -o "$tmp/warn" 2>"$tmp/warn.err"; then
  echo 'FAIL: unsupported enabling of RWX warning accepted silently' >&2
  exit 1
fi
grep -q 'warning treated as error' "$tmp/warn.err"
echo 'PASS: disabling RWX diagnostics is quiet under fatal warnings'
