#!/usr/bin/env bash
# ============================================================================
# boot_size_oracle.sh — measure the Linux x86 boot code's 32 KiB budget
# against a reference toolchain, object by object.
#
# Why this exists
# ---------------
# `build_kernel_boot.sh` answers "does lccc fit under the 32 KiB gate?" — a
# pass/fail.  That is necessary but not sufficient: with only ~1.6 KiB of
# headroom, a silent code-size regression in one of the 23 boot objects eats
# the margin long before anything fails, and every byte spent there is a byte
# the compressed kernel cannot use.  This script answers the *comparative*
# question with data:
#
#   * per-object `.text` size, lccc vs GCC (vs Clang), sorted by delta;
#   * aggregate content end (the value the gate actually measures);
#   * the resulting headroom for each toolchain;
#   * a byte-identical flat-image check when the object sets agree.
#
# Both sides are compiled with the *identical* command lines from
# scripts/boot_flags.sh, so a delta is a code-generation delta and nothing
# else.  Objects are emitted to separate directories, so the two builds never
# touch the same file.
#
# Usage:
#   KERNEL_DIR=... scripts/boot_size_oracle.sh            # gcc oracle
#   CC_ORACLE=clang scripts/boot_size_oracle.sh           # clang oracle
#   CC_ORACLE="clang gcc" scripts/boot_size_oracle.sh     # both
# Environment:
#   KERNEL_DIR   kernel tree        (default /home/user/kernel-work/linux-6.18.52)
#   LCCC         lccc compiler      (default <repo>/target/fastbuild/lccc)
#   LCCC_LD      lccc linker        (default <repo>/target/fastbuild/lccc-ld)
#   OUT          work directory     (default /var/tmp/bootbuild)
#   OOUT         oracle work dir    (default $OUT/oracle-<cc>)
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.52}
LCCC=${LCCC:-$here/../target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$here/../target/fastbuild/lccc-ld}
OUT=${OUT:-/var/tmp/bootbuild}
CC_ORACLE=${CC_ORACLE:-gcc}

# shellcheck source=boot_flags.sh
. "$here/boot_flags.sh"
# shellcheck source=boot_offsets.sh
. "$here/boot_offsets.sh"
# shellcheck source=elf_sections.sh
. "$here/elf_sections.sh"

[[ -d $K ]] || { echo "boot_size_oracle: kernel tree missing: $K" >&2; exit 1; }
[[ -x $LCCC ]] || { echo "boot_size_oracle: lccc missing: $LCCC" >&2; exit 1; }
cd "$K"
# `LCCC_BOOT_OBJS` includes `header`, and header.S includes zoffset.h/voffset.h
# for ZO_efi*_stub_entry / VO__text.  A setup-only link never regenerates them,
# so this harness was sourcing boot_offsets.sh without ever calling it and then
# dying on the missing header instead of reporting a size delta.  Restore the
# same deterministic immediates build_kernel_boot.sh uses; both toolchains
# compile the identical stubbed header, so the lccc-vs-oracle delta stays
# codegen-only.
ensure_boot_offset_stubs

# The KEEP-annotated copy of setup.ld used by build_kernel_boot.sh; regenerate
# identically so both links see the same script.
SETUP_LD="$OUT/setup-gc.ld"
mkdir -p "$OUT"
python3 - "arch/x86/boot/setup.ld" "$SETUP_LD" <<'PY'
from pathlib import Path
import sys

source, output = map(Path, sys.argv[1:])
text = source.read_text()
for section in (".bstext", ".header", ".entrytext", ".inittext", ".initdata",
                ".text32", ".pecompat", ".videocards"):
    plain, kept = f"*({section})", f"KEEP(*({section}))"
    if kept in text:
        continue
    if text.count(plain) != 1:
        raise SystemExit(f"cannot add KEEP for {section}")
    text = text.replace(plain, kept)
output.write_text(text)
PY

# ---- compile one object set with one compiler ------------------------------
compile_set() { # compile_set <cc> <outdir> <extra-args...>
  local cc=$1 dir=$2; shift 2
  mkdir -p "$dir"
  # Per-compiler spelling of the shared flag set (see boot_flags.sh): the
  # oracle may be clang, which rejects GCC's -mpreferred-stack-boundary.  Both
  # sides still get the SAME flags for the same compiler, so a delta stays a
  # code-generation delta.
  local cflags
  cflags=$(lccc_boot_cflags_for "$cc")
  local f
  for f in "${LCCC_BOOT_ASM_FILES[@]}"; do
    "$cc" $LCCC_BOOT_CPPFLAGS $cflags "$@" -D__ASSEMBLY__ \
      -c "arch/x86/boot/$f.S" -o "$dir/$f.o"
  done
  for f in "${LCCC_BOOT_C_FILES[@]}"; do
    "$cc" $LCCC_BOOT_CPPFLAGS $cflags "$@" \
      -c "arch/x86/boot/$f.c" -o "$dir/$f.o"
  done
}

# Executable bytes per object come from elf_sections.sh.  The local sum this
# replaced matched `/^\.text/` and so reported header.o and bioscall.o as 0
# bytes -- the boot stage keeps their code in .bstext, .entrytext and .inittext
# -- hiding 918 of lccc's 25299 executable bytes (791 of gcc's, 815 of clang's)
# and shrinking tty.o's real +184-byte gap against gcc to +57 in the ranking
# below, whose whole purpose is to surface the objects worth optimizing.

gate_report() { # gate_report <elf> <label>
  local elf=$1 label=$2 end dec head
  end=$(nm -n "$elf" 2>/dev/null | awk '$3 == "_end" { print $1; exit }')
  [[ -n $end ]] || { printf '%-10s %s\n' "$label" "no _end"; return 1; }
  dec=$((16#$end))
  head=$((LCCC_BOOT_GATE_BYTES - dec))
  if (( dec <= LCCC_BOOT_GATE_BYTES )); then
    printf '%-10s _end=%-7d (%d KiB)  headroom=%d bytes  PASS\n' \
      "$label" "$dec" "$((dec / 1024))" "$head"
  else
    printf '%-10s _end=%-7d (%d KiB)  OVERFLOW=%d bytes  FAIL\n' \
      "$label" "$dec" "$((dec / 1024))" "$((-head))"
  fi
}

# ---- lccc side --------------------------------------------------------------
if [[ ${SKIP_LCCC:-0} != 1 ]]; then
  echo "== compiling boot objects with lccc =="
  compile_set "$LCCC" "$OUT"
  lccc_objs=(); for o in "${LCCC_BOOT_OBJS[@]}"; do lccc_objs+=("$OUT/$o.o"); done
  "$LCCC_LD" --gc-sections -m elf_i386 -z noexecstack -T "$SETUP_LD" \
    "${lccc_objs[@]}" -o "$OUT/setup.elf"
fi

rc=0
for cc in $CC_ORACLE; do
  command -v "$cc" >/dev/null 2>&1 || { echo "warning: $cc not found; skipped" >&2; continue; }
  odir="$OUT/oracle-$cc"
  echo
  echo "== compiling boot objects with $cc ($($cc --version | head -1)) =="
  compile_set "$cc" "$odir"
  oobjs=(); for o in "${LCCC_BOOT_OBJS[@]}"; do oobjs+=("$odir/$o.o"); done
  # Link the oracle objects with GNU ld so the layout is the reference layout.
  if ! ld.bfd --gc-sections -m elf_i386 -z noexecstack -T "$SETUP_LD" \
       "${oobjs[@]}" -o "$odir/setup.elf" 2>"$odir/ld.err"; then
    echo "error: ld.bfd could not link the $cc objects:" >&2
    tail -5 "$odir/ld.err" >&2
    rc=1
    continue
  fi

  rows=''; total_l=0; total_o=0
  for o in "${LCCC_BOOT_OBJS[@]}"; do
    tl=$(lccc_elf_code_bytes "$OUT/$o.o")
    to=$(lccc_elf_code_bytes "$odir/$o.o")
    total_l=$((total_l + tl)); total_o=$((total_o + to))
    d=$((tl - to))
    note=''
    (( d > 0 )) && note='lccc larger'
    (( d < 0 )) && note='lccc smaller'
    rows+=$(printf '%-10s %8d %8d %+8d   %s\n' "$o" "$tl" "$to" "$d" "$note")$'\n'
  done
  printf '%-10s %8s %8s %8s   %s\n' OBJECT LCCC "$cc" DELTA 'note'
  # Largest lccc excess first: the objects worth optimizing bubble to the top.
  # The ranking lives in elf_sections.sh: `sort -n` reads a `%+8d` delta as 0,
  # so sorting the printed column here tied every row and emitted the table in
  # reverse-lexicographic order by object name instead of by excess.
  printf '%s' "$rows" | lccc_rank_size_rows
  echo
  printf '%-10s %8d %8d %+8d\n' TOTAL "$total_l" "$total_o" "$((total_l - total_o))"
  echo
  gate_report "$OUT/setup.elf" lccc
  gate_report "$odir/setup.elf" "$cc"
  # Byte-identical flat images are the strongest available statement: same
  # sections, same layout, same bytes.  Different sizes make this moot, so
  # only compare when the content ends agree.
  if command -v objcopy >/dev/null 2>&1; then
    objcopy -O binary "$OUT/setup.elf" "$OUT/setup.lccc.bin"
    objcopy -O binary "$odir/setup.elf" "$odir/setup.$cc.bin"
    if cmp -s "$OUT/setup.lccc.bin" "$odir/setup.$cc.bin"; then
      echo "flat image: byte-identical to $cc"
    else
      echo "flat image: differs from $cc (expected when code sizes differ)"
    fi
  fi
done
exit $rc
