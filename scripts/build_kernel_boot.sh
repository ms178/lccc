#!/usr/bin/env bash
# ============================================================================
# build_kernel_boot.sh — build the Linux kernel's x86 boot code (arch/x86/boot)
# entirely with LCCC + lccc-ld, and report the 32 KiB "Setup too big!" gate.
#
# Scope: the real-mode setup objects (23 .o) linked into setup.elf by lccc-ld
# against arch/x86/boot/setup.ld, whose ASSERT is `_end <= 0x8000` (32 KiB).
# This is the exact gate that must pass before bzImage can be produced.
#
# A stub zoffset.h is used: header.o's SIZE is independent of the ZO_* values
# (they are immediates); the real values come from `nm compressed/vmlinux`.
#
# Usage:
#   KERNEL_DIR=/path/to/linux LCCC=/path/to/lccc build_kernel_boot.sh
# ============================================================================
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.44}
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-/home/user/lccc/target/fastbuild/lccc-ld}
OUT=${OUT:-/tmp/bootbuild}

cd "$K"

RMF="-std=gnu11 -m16 -g -Os -march=i386 -mregparm=3 -fno-strict-aliasing -fomit-frame-pointer -fno-pic -mno-mmx -mno-sse -mpreferred-stack-boundary=2 -ffreestanding -fno-stack-protector -fno-asynchronous-unwind-tables -fcf-protection=none -fno-jump-tables -Wall -Wstrict-prototypes -Wno-address-of-packed-member"
INC="-nostdinc -Iarch/x86/boot -Iarch/x86/include -Iarch/x86/include/generated -Iinclude -Iinclude/generated -Iinclude/uapi -Iarch/x86/include/uapi -Iarch/x86/include/generated/uapi -Iinclude/generated/uapi -include include/linux/compiler-version.h -include include/linux/kconfig.h -include include/linux/compiler_types.h -D__KERNEL__ -D_SETUP -DDISABLE_BRANCH_PROFILING -D__DISABLE_EXPORTS"

mkdir -p "$OUT"

# ---- generated headers -----------------------------------------------------
# cpustr.h (needs the mkcpustr host tool)
if [[ ! -f arch/x86/boot/cpustr.h ]]; then
  gcc -O2 -Iarch/x86/include -Iarch/x86/include/generated -Iinclude \
      arch/x86/boot/mkcpustr.c -o "$OUT/mkcpustr"
  "$OUT/mkcpustr" > arch/x86/boot/cpustr.h
fi

# ---- assemble the .S files -------------------------------------------------
ASM_FILES=(header bioscall copy pmjump)
for f in "${ASM_FILES[@]}"; do
  echo "AS   $f.S"
  "$LCCC" $INC $RMF -D__ASSEMBLY__ -c "arch/x86/boot/$f.S" -o "$OUT/$f.o"
done

# ---- compile the .c files --------------------------------------------------
C_FILES=(a20 cmdline cpu cpuflags cpucheck early_serial_console edd main memory
         pm printf regs string tty video video-mode version video-vga
         video-vesa video-bios)
for f in "${C_FILES[@]}"; do
  echo "CC   $f.c"
  "$LCCC" $INC $RMF -c "arch/x86/boot/$f.c" -o "$OUT/$f.o"
done

# ---- link setup.elf with lccc-ld -------------------------------------------
SETUP_OBJS=(a20 bioscall cmdline copy cpu cpuflags cpucheck early_serial_console
            edd header main memory pm pmjump printf regs string tty video
            video-mode version video-vga video-vesa video-bios)
OBJS=()
for o in "${SETUP_OBJS[@]}"; do OBJS+=("$OUT/$o.o"); done

report_layout() { # report_layout <elf>
  local elf=$1 end_hex end_dec delta
  echo
  echo "=== $(basename "$elf") section sizes (bytes) ==="
  size -A "$elf"
  echo
  nm -n "$elf" | grep -E ' (_end|setup_size|setup_sects|__bss_start|__bss_end)$' || true
  end_hex=$(nm -n "$elf" | awk '$3 == "_end" { print $1; exit }')
  if [[ -n $end_hex ]]; then
    end_dec=$((16#$end_hex))
    delta=$((end_dec - 0x8000))
    if (( delta <= 0 )); then
      printf '32 KiB gate: PASS (_end=%d, headroom=%d bytes)\n' "$end_dec" "$((-delta))"
    else
      printf '32 KiB gate: FAIL (_end=%d, overflow=%d bytes)\n' "$end_dec" "$delta"
    fi
  fi
}

echo "LD   setup.elf (lccc-ld)"
if "$LCCC_LD" -m elf_i386 -z noexecstack -T arch/x86/boot/setup.ld \
    "${OBJS[@]}" -o "$OUT/setup.elf"; then
  report_layout "$OUT/setup.elf"
  exit 0
else
  link_status=$?
fi

# A size ASSERT aborts before an ELF exists, which used to leave the developer
# with no `_end` value and encouraged misleading sums of input sections.  Link
# a diagnostic artifact after removing ONLY the two size guards; every ABI,
# header-offset and init-section ASSERT remains active.  This artifact is never
# a boot candidate.  It exists solely to expose the exact linker-script layout
# and alignment cliffs (notably `.pecompat` at 4 KiB alignment).
diag_script="$OUT/setup-size-diagnostic.ld"
sed -e '/ASSERT(setup_sects <= 64,/d' \
    -e '/ASSERT(_end <= 0x8000,/d' \
    arch/x86/boot/setup.ld > "$diag_script"
echo "LD   setup-size-diagnostic.elf (size ASSERTs removed for measurement only)"
if "$LCCC_LD" -m elf_i386 -z noexecstack -T "$diag_script" \
    "${OBJS[@]}" -o "$OUT/setup-size-diagnostic.elf"; then
  report_layout "$OUT/setup-size-diagnostic.elf"
else
  echo "error: diagnostic link also failed; this is not solely a size-gate failure" >&2
fi
exit "$link_status"
