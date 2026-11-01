#!/usr/bin/env bash
# Fast smoke for the RWX LOAD diagnostic. The comprehensive ELF32/ELF64,
# driver, flat-binary and bfd-oracle matrix lives in
# tests/linker/check_rwx_segments.py.
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
.section .note.GNU-stack,"",@progbits
ASM
cat > "$tmp/rwx.ld" <<'LDS'
ENTRY(_start)
PHDRS { all PT_LOAD FLAGS(7); }
SECTIONS { . = 0x400000; .text : { *(.text) } :all }
LDS
"$cc" -m32 -c "$tmp/a.s" -o "$tmp/a.o"
base=("$ld" -m elf_i386 -T "$tmp/rwx.ld" "$tmp/a.o")
"${base[@]}" -o "$tmp/base" 2>"$tmp/base.err"
grep -q 'RWX permissions' "$tmp/base.err"
if "${base[@]}" --fatal-warnings -o "$tmp/fatal" 2>"$tmp/fatal.err"; then
  echo 'FAIL: RWX LOAD survived --fatal-warnings' >&2
  exit 1
fi
grep -q 'warning treated as error' "$tmp/fatal.err"
"${base[@]}" --fatal-warnings --no-warn-rwx-segments -o "$tmp/quiet" 2>"$tmp/quiet.err"
[[ ! -s "$tmp/quiet.err" ]]
cmp "$tmp/base" "$tmp/quiet"
echo 'PASS: RWX warning, fatal policy, suppression and byte identity'
