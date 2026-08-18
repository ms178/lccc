#!/usr/bin/env bash
# ============================================================================
# realmode_corpus.sh — measure LCCC vs GCC code size on the Linux kernel's
# 16-bit real-mode boot code (arch/x86/boot).  This is the corpus behind the
# "Setup too big!" 32K gate (setup.ld: `_end <= 0x8000`).
#
# Mirrors REALMODE_CFLAGS from arch/x86/Makefile + KBUILD_CFLAGS from
# arch/x86/boot/Makefile, plus the generic kernel include paths.
#
# Usage:  realmode_corpus.sh <lccc-binary> [outdir]
# ============================================================================
set -euo pipefail

LCCC=$(realpath "${1:?usage: realmode_corpus.sh <lccc-binary> [outdir]}")
OUT=${2:-/tmp/realmode}
K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.44}

cd "$K"

# generated/utsversion.h is only produced during the full build (init/Makefile).
# It feeds version.c's UTS_VERSION constant only; provide a benign stub so the
# corpus measurement does not depend on a full vmlinux build.
if [[ ! -f include/generated/utsversion.h ]]; then
  mkdir -p include/generated
  printf '#define UTS_VERSION "#1 SMP %s"\n' "$(date -u +%Y-%m-%d)" \
    > include/generated/utsversion.h
fi

INC=(
  -nostdinc
  -Iarch/x86/boot
  -Iarch/x86/include
  -Iarch/x86/include/generated
  -Iinclude
  -Iinclude/generated
  -Iinclude/uapi
  -Iarch/x86/include/uapi
  -Iarch/x86/include/generated/uapi
  -Iinclude/generated/uapi
  -include include/linux/compiler-version.h
  -include include/linux/kconfig.h
  -include include/linux/compiler_types.h
)

DEFS=(
  -D__KERNEL__ -D_SETUP
  -DDISABLE_BRANCH_PROFILING -D__DISABLE_EXPORTS
)

RMF=(
  -std=gnu11 -m16 -g -Os
  -march=i386 -mregparm=3
  -fno-strict-aliasing -fomit-frame-pointer -fno-pic
  -mno-mmx -mno-sse -mpreferred-stack-boundary=2
  -ffreestanding -fno-stack-protector
  -fno-asynchronous-unwind-tables
  -fcf-protection=none
  -fno-jump-tables
  -Wall -Wstrict-prototypes -Wno-address-of-packed-member
)

CFILES=(a20 bioscall cmdline copy cpu cpuflags cpucheck early_serial_console edd
        main memory pm pmjump printf regs string tty video video-mode version
        video-vga video-vesa video-bios)

mkdir -p "$OUT/lccc" "$OUT/gcc"

sum_lccc=0
sum_gcc=0
printf '%-24s %10s %10s %8s\n' "file" "lccc.text" "gcc.text" "ratio"
printf '%-24s %10s %10s %8s\n' "----" "--------" "--------" "-----"
for f in "${CFILES[@]}"; do
  src="arch/x86/boot/$f.c"
  [[ -f "$src" ]] || continue
  "$LCCC" "${INC[@]}" "${DEFS[@]}" "${RMF[@]}" -c "$src" -o "$OUT/lccc/$f.o" 2>"$OUT/lccc/$f.err" \
    && l=$(size -A "$OUT/lccc/$f.o" | awk '$1==".text"{print $2}') || l="ERR"
  gcc    "${INC[@]}" "${DEFS[@]}" "${RMF[@]}" -c "$src" -o "$OUT/gcc/$f.o" 2>"$OUT/gcc/$f.err" \
    && g=$(size -A "$OUT/gcc/$f.o" | awk '$1==".text"{print $2}') || g="ERR"
  [[ "$l" == "ERR" ]] || sum_lccc=$((sum_lccc+l))
  [[ "$g" == "ERR" ]] || sum_gcc=$((sum_gcc+g))
  ratio=""
  if [[ "$l" =~ ^[0-9]+$ && "$g" =~ ^[0-9]+$ && "$g" -gt 0 ]]; then
    ratio=$(awk "BEGIN{printf \"%.2f\", $l/$g}")
  fi
  printf '%-24s %10s %10s %8s\n' "$f" "$l" "$g" "$ratio"
done
printf '%-24s %10s %10s\n' "TOTAL" "$sum_lccc" "$sum_gcc"
awk "BEGIN{printf \"overall ratio: %.2f\n\", $sum_lccc/$sum_gcc}"
