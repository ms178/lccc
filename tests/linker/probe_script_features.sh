#!/bin/bash
# Linker-script feature probe: lccc-ld vs ld.bfd.
#
# Each probe is a minimal script exercising ONE construct that real kernel,
# firmware or ld-generated scripts depend on. The point is to find constructs
# lccc REJECTS that bfd accepts -- a class of gap that unit tests cannot reveal,
# because you only write a test for a feature you already know you have.
#
# This is how the session-11 blindspot sweep found MEMORY regions, OVERLAY,
# SEGMENT_START, DATA_SEGMENT_ALIGN and HIDDEN(). Re-run it after any parser
# change; every line should read [both-ok].
#
#   [both-ok] both linkers accept it
#   [GAP    ] bfd accepts, lccc rejects  <-- a blindspot; fix it
#   [lccc-ok] lccc accepts, bfd rejects  <-- check we are not being too lax
#   [both-no] neither accepts (usually a malformed probe)
#
# Usage: bash tests/linker/probe_script_features.sh [path-to-lccc-ld]
set -u
LD="${1:-/home/user/lccc/target/fastbuild/lccc-ld}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 1
cat > o.c <<'EOF'
int datum = 42;
const int rodatum = 7;
int bss_datum;
void _start(void){ }
void other(void){ }
EOF
gcc -c -O1 -ffreestanding o.c -o o.o 2>/dev/null

probe() {
  local name="$1"; local script="$2"
  printf '%s\n' "$script" > "p_$name.lds"
  local le lb
  $LD -T "p_$name.lds" o.o -o "out_$name.lccc" 2>"err_$name.lccc"; le=$?
  ld.bfd -T "p_$name.lds" o.o -o "out_$name.bfd" 2>"err_$name.bfd"; lb=$?
  if [ $le -eq 0 ] && [ $lb -eq 0 ]; then
    echo "  [both-ok] $name" | tee -a "$WORK/.results"
  elif [ $le -ne 0 ] && [ $lb -eq 0 ]; then
    echo "  [GAP    ] $name :: $(head -1 err_$name.lccc | cut -c1-110)" | tee -a "$WORK/.results"
  elif [ $le -eq 0 ] && [ $lb -ne 0 ]; then
    echo "  [lccc-ok] $name (bfd rejects)" | tee -a "$WORK/.results"
  else
    echo "  [both-no] $name :: lccc=$(head -1 err_$name.lccc|cut -c1-70)" | tee -a "$WORK/.results"
  fi
}

echo "== linker script feature probes =="
probe overlay 'SECTIONS { . = 0x100000; .text : { *(.text) } OVERLAY 0x200000 : AT (0x300000) { .ov1 { *(.data) } .ov2 { *(.rodata) } } .rest : { *(.bss) } }'
probe atsym 'SECTIONS { . = 0x100000; .text : AT(ADDR(.text) + 0x1000) { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe region 'MEMORY { ram (rwx) : ORIGIN = 0x100000, LENGTH = 1M } SECTIONS { .text : { *(.text) } > ram .data : { *(.data) *(.rodata) *(.bss) } > ram }'
probe fill 'SECTIONS { . = 0x100000; .text : { *(.text) . = ALIGN(64); } =0x90909090 .data : { *(.data) *(.rodata) *(.bss) } }'
probe sortname 'SECTIONS { . = 0x100000; .text : { *(SORT_BY_NAME(.text*)) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe sortalign 'SECTIONS { . = 0x100000; .text : { *(SORT_BY_ALIGNMENT(.text*)) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe excludefile 'SECTIONS { . = 0x100000; .text : { *(EXCLUDE_FILE(*crt*.o) .text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe keep 'SECTIONS { . = 0x100000; .text : { KEEP(*(.text)) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe provide 'SECTIONS { . = 0x100000; .text : { *(.text) } PROVIDE(mysym = .); PROVIDE_HIDDEN(myhid = .); .data : { *(.data) *(.rodata) *(.bss) } }'
probe subalign 'SECTIONS { . = 0x100000; .text : SUBALIGN(32) { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe nocrossref 'SECTIONS { . = 0x100000; .text : { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } } NOCROSSREFS(.text .data)'
probe segstart 'SECTIONS { . = SEGMENT_START("text-segment", 0x100000); .text : { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe datasegalign 'SECTIONS { . = 0x100000; .text : { *(.text) } . = DATA_SEGMENT_ALIGN(4096, 4096); .data : { *(.data) *(.rodata) *(.bss) } . = DATA_SEGMENT_END(.); }'
probe insert 'SECTIONS { . = 0x100000; .text : { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe entryexpr 'ENTRY(_start) SECTIONS { . = 0x100000; .text : { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe loadaddr 'SECTIONS { . = 0x100000; .text : { *(.text) } _lma = LOADADDR(.text); .data : { *(.data) *(.rodata) *(.bss) } }'
probe ternary 'SECTIONS { . = 0x100000; _x = (1 > 0) ? 8 : 16; .text : { *(.text) } .data : { *(.data) *(.rodata) *(.bss) } }'
probe hidden 'SECTIONS { . = 0x100000; .text : { *(.text) } HIDDEN(hsym = .); .data : { *(.data) *(.rodata) *(.bss) } }'

echo
gaps=$(grep -c "\[GAP" "$WORK/.results" 2>/dev/null || true)
echo "== script feature probe: $(grep -c "both-ok" "$WORK/.results") ok, ${gaps:-0} gap(s) =="
if [ "${gaps:-0}" -ne 0 ]; then exit 1; fi
exit 0
