#!/usr/bin/env bash
# Rebuild the binary fuzzer seeds from good.c. Binary artefacts are not
# committed: they are toolchain-specific and trivially regenerated.
set -euo pipefail
cd "$(dirname "$0")"
CC=${CC:-gcc}
$CC -c -O1 -ffunction-sections -fdata-sections good.c -o good.o
cp good.o seed_obj.o
rm -f seed_arch.a && ar rcs seed_arch.a good.o
$CC -c -O1 -g good.c -o seed_debug.o
cat > seed_script.lds <<'LDS'
ENTRY(_start)
SECTIONS {
  . = 0x400000;
  .text   : { *(.text .text.*) }
  .rodata : { *(.rodata .rodata.*) }
  .data   : { *(.data) }
  .bss    : { *(.bss) *(COMMON) }
}
LDS
echo "seeds regenerated in $PWD"
