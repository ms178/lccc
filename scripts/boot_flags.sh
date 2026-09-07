#!/usr/bin/env bash
# ============================================================================
# boot_flags.sh — single source of truth for the Linux x86 real-mode boot
# translation-unit flags (arch/x86/boot, -m16).
#
# Rationale: build_kernel_boot.sh, boot_size_oracle.sh and the i686/m16
# torture corpus all need *byte-identical* command lines, otherwise an
# A/B size comparison silently measures a flag difference instead of a
# code-generation difference.  These two variables mirror
# arch/x86/boot/Makefile + arch/x86/Makefile's realmode rules verbatim.
#
# Sourced, never executed:  . "$(dirname "$0")/boot_flags.sh"
# ============================================================================

# KBUILD_CFLAGS for arch/x86/boot (real mode, 16-bit, i386 baseline).
LCCC_BOOT_CFLAGS="-std=gnu11 -m16 -g -Os -march=i386 -mregparm=3 -fno-strict-aliasing -fomit-frame-pointer -fno-pic -mno-mmx -mno-sse -mpreferred-stack-boundary=2 -ffreestanding -ffunction-sections -fno-stack-protector -fno-asynchronous-unwind-tables -fcf-protection=none -fno-jump-tables -Wall -Wstrict-prototypes -Wno-address-of-packed-member -DSVGA_MODE=NORMAL_VGA"

# KBUILD_CPPFLAGS for arch/x86/boot.
LCCC_BOOT_CPPFLAGS="-nostdinc -Iarch/x86/boot -Iarch/x86/include -Iarch/x86/include/generated -Iinclude -Iinclude/generated -Iinclude/uapi -Iarch/x86/include/uapi -Iarch/x86/include/generated/uapi -Iinclude/generated/uapi -include include/linux/compiler-version.h -include include/linux/kconfig.h -include include/linux/compiler_types.h -D__KERNEL__ -D_SETUP -DDISABLE_BRANCH_PROFILING -D__DISABLE_EXPORTS"

# The 23 objects that arch/x86/boot/Makefile links into setup.elf, in link
# order.  setupobjs = $(boot-y); the .S members are in boot-y too.
LCCC_BOOT_ASM_FILES=(header bioscall copy pmjump)
LCCC_BOOT_C_FILES=(a20 cmdline cpu cpuflags cpucheck early_serial_console edd
                   main memory pm printf regs string tty video video-mode
                   version video-vga video-vesa video-bios)
LCCC_BOOT_OBJS=(a20 bioscall cmdline copy cpu cpuflags cpucheck
                early_serial_console edd header main memory pm pmjump printf
                regs string tty video video-mode version video-vga video-vesa
                video-bios)

# The 32 KiB setup gate: setup.ld asserts `_end <= 0x8000`.
LCCC_BOOT_GATE_BYTES=32768
