#!/usr/bin/env bash
# Finalise a bzImage from an existing compressed/vmlinux (steps 8-9 of
# build_kernel_compressed.sh) with independent CC/LD choices.
set -euo pipefail
K=${KERNEL_DIR:-/home/user/kernel-work/linux-vm}
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-/home/user/lccc/target/fastbuild/lccc-ld}
OUT=${OUT:-/tmp/bz}
C=$K/arch/x86/boot/compressed
B=$K/arch/x86/boot
mkdir -p $OUT/setup
sed_zoffset='s/^\([0-9a-fA-F]*\) [a-zA-Z] \(startup_32\|efi.._stub_entry\|efi\(32\)\?_pe_entry\|input_data\|kernel_info\|_end\|_ehead\|_text\|_e\?data\|_e\?sbat\|z_.*\)$/#define ZO_\2 0x\1/p'
nm "$C/vmlinux" | sed -n "$sed_zoffset" > "$B/zoffset.h"
echo "zoffset.h:"; cat "$B/zoffset.h"
OUT=$OUT/setup KERNEL_DIR=$K LCCC=$LCCC LCCC_LD=$LCCC_LD bash /home/user/lccc/scripts/build_kernel_boot.sh > /tmp/setup_build.log 2>&1 || { echo "setup build failed"; tail -5 /tmp/setup_build.log; exit 1; }
grep -E "32 KiB gate|ORACLE" /tmp/setup_build.log
RMF="-std=gnu11 -m16 -g -Os -march=i386 -mregparm=3 -fno-strict-aliasing -fomit-frame-pointer -fno-pic -mno-mmx -mno-sse -mpreferred-stack-boundary=2 -ffreestanding -ffunction-sections -fno-stack-protector -fno-asynchronous-unwind-tables -fcf-protection=none -fno-jump-tables -DSVGA_MODE=NORMAL_VGA"
SINC="-nostdinc -I$B -I$K/arch/x86/include -I$K/arch/x86/include/generated -I$K/include -I$K/include/generated -I$K/include/uapi -I$K/arch/x86/include/uapi -I$K/arch/x86/include/generated/uapi -I$K/include/generated/uapi -include $K/include/linux/compiler-version.h -include $K/include/linux/kconfig.h -include $K/include/linux/compiler_types.h -D__KERNEL__ -D_SETUP -DDISABLE_BRANCH_PROFILING -D__DISABLE_EXPORTS"
$LCCC $SINC $RMF -D__ASSEMBLY__ -I"$B" -c "$B/header.S" -o "$OUT/setup/header.o"
SETUP_LD="$OUT/setup/setup-gc.ld"
SETUP_OBJS=(a20 bioscall cmdline copy cpu cpuflags cpucheck early_serial_console edd header main memory pm pmjump printf regs string tty video video-mode version video-vga video-vesa video-bios)
SO=(); for o in "${SETUP_OBJS[@]}"; do SO+=("$OUT/setup/$o.o"); done
$LCCC_LD --gc-sections -m elf_i386 -z noexecstack -T "$SETUP_LD" "${SO[@]}" -o "$B/setup.elf"
objcopy -O binary "$B/setup.elf" "$B/setup.bin"
objcopy -O binary -R .note -R .comment -S "$C/vmlinux" "$B/vmlinux.bin"
(dd if="$B/setup.bin" bs=4k conv=sync status=none; cat "$B/vmlinux.bin") > "$OUT/bzImage"
ls -l "$OUT/bzImage"; sha256sum "$OUT/bzImage"
