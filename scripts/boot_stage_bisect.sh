#!/usr/bin/env bash
# ============================================================================
# boot_stage_bisect.sh — bisect the Linux real-mode boot stage between LCCC and
# an oracle toolchain, object by object and linker by linker.
#
# Why this exists
# ---------------
# A bzImage that produces *no* serial output failed somewhere inside
# arch/x86/boot's 23 translation units or in the setup.elf link, and the usual
# tools are useless there: there is no console yet, and `-earlyprintk` on this
# image livelocks in 16-bit `early_serial_init`. Rebuilding the whole kernel
# for each hypothesis costs ~12 minutes, so the stage has to be isolated.
#
# This script relinks ONLY the boot stage, reusing the already-built
# compressed payload (`arch/x86/boot/vmlinux.bin`), so one
# compile -> link -> QEMU verdict costs ~20 seconds instead of ~12 minutes.
# Any subset of the .c files can be handed to the oracle compiler while the
# rest stay on LCCC, which turns "which file broke boot?" into a binary search.
#
# The commands are the ones Kbuild recorded in arch/x86/boot/.*.cmd, so the
# A/B differs in exactly one variable.
#
# Usage:
#   scripts/boot_stage_bisect.sh                 # all LCCC + lccc-ld
#   BOOT_BISECT_LD=ld.bfd scripts/boot_stage_bisect.sh
#   BOOT_BISECT_ORACLE_FILES="main tty" scripts/boot_stage_bisect.sh
# Environment:
#   KERNEL_DIR                 kernel tree (default /home/user/kernel-work/linux-6.18.47)
#   LCCC / LCCC_LD             toolchain under test
#   BOOT_BISECT_LD             lccc-ld (default) | ld.bfd | ld.lld
#   BOOT_BISECT_ORACLE_FILES   space-separated basenames compiled by $CC_ORACLE
#   CC_ORACLE                  oracle compiler (default gcc)
#   BOOT_BISECT_QEMU_TIMEOUT   seconds to wait for output (default 25)
# Exit status: 0 = boot stage produced kernel output, 1 = silent/hung.
# ============================================================================
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
LCCC=${LCCC:-$here/../target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$here/../target/fastbuild/lccc-ld}
CC_ORACLE=${CC_ORACLE:-gcc}
LD_IMPL=${BOOT_BISECT_LD:-lccc-ld}
ORACLE_FILES=${BOOT_BISECT_ORACLE_FILES:-}
OUT=${BOOT_BISECT_OUT:-/var/tmp/bootbisect}
QTIMEOUT=${BOOT_BISECT_QEMU_TIMEOUT:-25}

# shellcheck source=boot_flags.sh
. "$here/boot_flags.sh"

[[ -d $K ]] || { echo "boot_stage_bisect: kernel tree missing: $K" >&2; exit 1; }
[[ -f $K/arch/x86/boot/vmlinux.bin ]] || {
  echo "boot_stage_bisect: $K/arch/x86/boot/vmlinux.bin missing (build the kernel first)" >&2
  exit 1
}
mkdir -p "$OUT"
cd "$K"

case $LD_IMPL in
  lccc-ld) LD_BIN=$LCCC_LD ;;
  *)       LD_BIN=$LD_IMPL ;;
esac

is_oracle_file() { # is_oracle_file <basename>
  local f
  for f in $ORACLE_FILES; do [[ $f == "$1" ]] && return 0; done
  return 1
}

# ---- compile the 20 C objects (the 4 .S objects are reused from the tree) ----
declare -A OBJPATH=()
for f in "${LCCC_BOOT_C_FILES[@]}"; do
  if is_oracle_file "$f"; then
    cc=$CC_ORACLE
  else
    cc=$LCCC
  fi
  # The kernel's own flags for this TU, read back from Kbuild's recorded
  # command so the only difference between arms is the compiler binary.
  cmdfile="arch/x86/boot/.$f.o.cmd"
  if [[ -f $cmdfile ]]; then
    # Strip the recorded compiler and the trailing `-c -o ... <src>`.
    args=$(sed -n "s|^savedcmd_arch/x86/boot/$f\.o := [^ ]* \(.*\) -c -o arch/x86/boot/$f\.o arch/x86/boot/$f\.c.*|\1|p" "$cmdfile")
    if [[ -n $args ]]; then
      # shellcheck disable=SC2086
      "$cc" $args -c "arch/x86/boot/$f.c" -o "$OUT/$f.o"
      OBJPATH[$f]="$OUT/$f.o"
      continue
    fi
  fi
  # Fallback: the shared flag module (identical flags, no Kbuild extras).
  # shellcheck disable=SC2086
  "$cc" $LCCC_BOOT_CPPFLAGS $LCCC_BOOT_CFLAGS -c "arch/x86/boot/$f.c" -o "$OUT/$f.o"
  OBJPATH[$f]="$OUT/$f.o"
done
# The four .S objects are assembled by the compiler under test by default:
# LCCC has its own integrated assembler, so the assembler is part of what this
# stage has to get right. BOOT_BISECT_ASM_CC hands them to the oracle instead,
# which separates "bad assembly" from "bad object code" in one more step.
ASM_CC=${BOOT_BISECT_ASM_CC:-$LCCC}
for f in "${LCCC_BOOT_ASM_FILES[@]}"; do
  cmdfile="arch/x86/boot/.$f.o.cmd"
  args=''
  if [[ -f $cmdfile ]]; then
    args=$(sed -n "s|^savedcmd_arch/x86/boot/$f\.o := [^ ]* \(.*\) -c -o arch/x86/boot/$f\.o arch/x86/boot/$f\.S.*|\1|p" "$cmdfile")
  fi
  if [[ -n $args ]]; then
    # shellcheck disable=SC2086
    "$ASM_CC" $args -c "arch/x86/boot/$f.S" -o "$OUT/$f.o"
  else
    # shellcheck disable=SC2086
    "$ASM_CC" $LCCC_BOOT_CPPFLAGS $LCCC_BOOT_CFLAGS -D__ASSEMBLY__ \
      -c "arch/x86/boot/$f.S" -o "$OUT/$f.o"
  fi
  OBJPATH[$f]="$OUT/$f.o"
done

# ---- link setup.elf with Kbuild's recorded flags ----------------------------
objs=()
for o in "${LCCC_BOOT_OBJS[@]}"; do objs+=("${OBJPATH[$o]}"); done
# shellcheck disable=SC2086
"$LD_BIN" -m elf_x86_64 -z noexecstack --no-warn-rwx-segments -m elf_i386 \
  -z noexecstack -T arch/x86/boot/setup.ld "${objs[@]}" -o "$OUT/setup.elf"
objcopy -O binary "$OUT/setup.elf" "$OUT/setup.bin"

# ---- assemble the bzImage exactly like arch/x86/boot/Makefile ---------------
( dd if="$OUT/setup.bin" bs=4k conv=sync status=none; cat arch/x86/boot/vmlinux.bin ) > "$OUT/bzImage"
end=$(nm -n "$OUT/setup.elf" | awk '$3 == "_end" { print $1; exit }')
printf 'setup _end=0x%s (%d bytes, gate %d)  ld=%s  oracle-files=[%s]\n' \
  "$end" "$((16#$end))" "$LCCC_BOOT_GATE_BYTES" "$LD_IMPL" "$ORACLE_FILES"

# ---- boot verdict -----------------------------------------------------------
log="$OUT/serial.log"
timeout "$QTIMEOUT" qemu-system-x86_64 -m 512 -smp 2 -L /usr/share/seabios \
  -kernel "$OUT/bzImage" -nographic -no-reboot -accel tcg,thread=multi \
  -append "console=ttyS0,115200 nokaslr panic=-1" > "$log" 2>&1 || true
if grep -aq 'Linux version' "$log"; then
  echo "VERDICT: BOOT (kernel banner present)"
  exit 0
fi
if grep -aqE 'Probing EDD|Decompressing Linux' "$log"; then
  echo "VERDICT: PARTIAL (setup ran, later stage silent)"
  exit 1
fi
echo "VERDICT: SILENT (boot stage produced no output)"
exit 1
