#!/usr/bin/env bash
# ============================================================================
# build_kernel_compressed.sh — wrap an intact vmlinux in an LCCC-built
# x86 decompressor + real-mode setup, producing arch/x86/boot/bzImage.
#
# NEVER invoke top-level `make bzImage` or a compressed *file* target:
# those leak KBUILD_CFLAGS (`-mcmodel=kernel -fno-PIE -march=native`) into
# the PIE decompressor. Compile every compressed TU with the flags from
# arch/x86/boot/compressed/Makefile:
#   -O2 -fPIE -mcmodel=small -mno-sse -mno-mmx -ffreestanding
#
# Payload: INTACT_VMLINUX (default kernel-work/intact/vmlinux). Do not relink
# that ELF. Piggy is `zstd -6 --ultra` plus a 4-byte LE uncompressed size.
#
# Usage:
#   KERNEL_DIR=... LCCC=... LCCC_LD=... INTACT_VMLINUX=... \
#     build_kernel_compressed.sh
# ============================================================================
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-/home/user/lccc/target/fastbuild/lccc-ld}
INTACT=${INTACT_VMLINUX:-/home/user/kernel-work/intact/vmlinux}
C=$K/arch/x86/boot/compressed
B=$K/arch/x86/boot
S=$K/arch/x86/boot/startup
OUT=${OUT:-/tmp/lccc-compressed}

[[ -d "$K" ]] || { echo "kernel tree missing: $K" >&2; exit 1; }
[[ -x "$LCCC" ]] || { echo "lccc missing: $LCCC" >&2; exit 1; }
[[ -x "$LCCC_LD" ]] || { echo "lccc-ld missing: $LCCC_LD" >&2; exit 1; }
[[ -f "$INTACT" ]] || { echo "intact vmlinux missing: $INTACT" >&2; exit 1; }

mkdir -p "$OUT"
cd "$K"

# ---- 1. VM config so the stub is EFI/KASLR/SEV-free (QEMU -kernel path) ----
if [[ ! -f .lccc-vm-config ]]; then
  echo "config: allnoconfig + kernel-vm.fragment"
  make ARCH=x86_64 allnoconfig >/dev/null
  scripts/kconfig/merge_config.sh -m .config \
    /home/user/lccc/scripts/kernel-vm.fragment >/dev/null
  make ARCH=x86_64 olddefconfig >/dev/null
  make ARCH=x86_64 syncconfig >/dev/null
  # timeconst.h / asm-offsets must match HZ_800 from the fragment.
  make ARCH=x86_64 HOSTCC=gcc CC=gcc prepare >/dev/null
  touch .lccc-vm-config
fi
grep -q '^CONFIG_KERNEL_ZSTD=y$' .config || {
  echo "CONFIG_KERNEL_ZSTD is not set" >&2
  exit 1
}

# Do not put intact vmlinux at $K/vmlinux — kbuild would treat it as a
# rebuildable goal. Work from a copy under $OUT.
cp -f "$INTACT" "$OUT/vmlinux"
chmod a-w "$OUT/vmlinux" || true

# ---- 2. voffset.h from the intact ELF (misc.o needs VO_* ) -----------------
sed_voffset='s/^\([0-9a-fA-F]*\) [ABbCDGRSTtVW] \(_text\|__start_rodata\|_sinittext\|__inittext_end\|__bss_start\|_end\)$/#define VO_\2 _AC(0x\1,UL)/p'
nm "$OUT/vmlinux" | sed -n "$sed_voffset" > "$B/voffset.h"
echo "voffset.h:"
cat "$B/voffset.h"

# ---- 3. piggy: vmlinux.bin + zstd -6 --ultra + 4-byte LE size --------------
echo "OBJCOPY vmlinux.bin"
objcopy -R .comment -S "$OUT/vmlinux" "$C/vmlinux.bin"
plain_sz=$(stat -c%s "$C/vmlinux.bin")
echo "ZSTD vmlinux.bin.zst (${plain_sz} bytes uncompressed)"
# size_append: 4-byte little-endian uncompressed size of vmlinux.bin.all
python3 - "$C/vmlinux.bin" "$C/vmlinux.bin.zst" <<'PY'
import struct, subprocess, sys
plain, out = sys.argv[1], sys.argv[2]
raw = subprocess.check_output(["zstd", "-6", "--ultra", "-c", plain])
size = open(plain, "rb").seek(0, 2)
open(plain, "rb").seek(0)
import os
size = os.path.getsize(plain)
open(out, "wb").write(raw + struct.pack("<I", size))
print(f"piggy payload {len(raw)} + 4-byte size {size}")
PY

if [[ ! -x "$C/mkpiggy" ]]; then
  gcc -O2 -I"$K/tools/include" "$C/mkpiggy.c" -o "$C/mkpiggy"
fi
"$C/mkpiggy" "$C/vmlinux.bin.zst" > "$C/piggy.S"
echo "MKPIGGY piggy.S"

# ---- 4. flags (compressed/Makefile; -mno-sse last, sticky in lccc) ---------
INC=(
  -nostdinc
  -I"$C" -I"$K/arch/x86/include" -I"$K/arch/x86/include/generated"
  -I"$K/include" -I"$K/include/generated"
  -I"$K/arch/x86/include/uapi" -I"$K/arch/x86/include/generated/uapi"
  -I"$K/include/uapi" -I"$K/include/generated/uapi"
  -include "$K/include/linux/compiler-version.h"
  -include "$K/include/linux/kconfig.h"
  -include "$K/include/linux/compiler_types.h"
  -include "$K/include/linux/hidden.h"
)
CF=(
  -std=gnu18 -m64 -O2 -fno-strict-aliasing -fPIE -fno-jump-tables
  -Wundef -DDISABLE_BRANCH_PROFILING -mcmodel=small -mno-red-zone
  -mno-mmx -ffreestanding -fshort-wchar -fno-stack-protector
  -Wno-pointer-sign -fno-asynchronous-unwind-tables -D__DISABLE_EXPORTS
  -D__KERNEL__ -fno-strict-overflow -mno-sse
)
cc() { # cc <stem>
  local s=$1
  echo "CC   $s.c"
  "$LCCC" "${INC[@]}" "${CF[@]}" \
    -DKBUILD_BASENAME="\"$s\"" \
    -DKBUILD_MODNAME="\"$s\"" \
    -DKBUILD_MODFILE="\"arch/x86/boot/compressed/$s\"" \
    -c "$C/$s.c" -o "$C/$s.o"
}
as() { # as <stem> [src]
  local s=$1 src=${2:-$C/$1.S}
  echo "AS   $(basename "$src")"
  "$LCCC" "${INC[@]}" "${CF[@]}" -D__ASSEMBLY__ \
    -c "$src" -o "$C/$s.o"
}

# ---- 5. startup/lib.a (x86_64 decompressor always pulls this in) -----------
echo "CC   startup"
ST_CF=(
  -std=gnu11 -m64 -Os -fPIC -mcmodel=small -fno-stack-protector
  -fno-jump-tables -ffreestanding -fno-asynchronous-unwind-tables
  -D__DISABLE_EXPORTS -DDISABLE_BRANCH_PROFILING -D__NO_FORTIFY -D__KERNEL__
  -mno-mmx -mno-sse
)
st_cc() {
  local s=$1
  "$LCCC" "${INC[@]}" "${ST_CF[@]}" \
    -DKBUILD_BASENAME="\"$s\"" -DKBUILD_MODNAME="\"$s\"" \
    -c "$S/$s.c" -o "$S/$s.o"
  objcopy --prefix-symbols=__pi_ "$S/$s.o" "$S/$s.pi.o"
}
st_cc gdt_idt
st_cc map_kernel
"$LCCC" "${INC[@]}" "${ST_CF[@]}" -D__ASSEMBLY__ -c "$S/la57toggle.S" -o "$S/la57toggle.o"
rm -f "$S/lib.a"
ar rcs "$S/lib.a" "$S/gdt_idt.pi.o" "$S/map_kernel.pi.o" "$S/la57toggle.o"
echo "AR   startup/lib.a"

# ---- 6. compressed objects -------------------------------------------------
cc misc
cc string
cc cmdline
cc error
cc cpuflags
cc ident_map_64
cc idt_64
cc pgtable_64
if grep -q '^CONFIG_EARLY_PRINTK=y$' .config; then
  cc early_serial_console
fi
as head_64
as idt_handlers_64
as kernel_info
as piggy "$C/piggy.S"

# linker script
echo "CPP  vmlinux.lds"
# Match scripts/Makefile.build cmd_cpp_lds_S: -P -U$(ARCH) -D__ASSEMBLY__
# -DLINKER_SCRIPT so ALIGN() stays a linker token, not gas `.balign`.
gcc -E -P -Ux86_64 -Ux86 -D__ASSEMBLY__ -DLINKER_SCRIPT \
  -nostdinc \
  -I"$K/arch/x86/include" -I"$K/arch/x86/include/generated" \
  -I"$K/include" -I"$K/include/generated" \
  -I"$K/arch/x86/include/uapi" -I"$K/arch/x86/include/generated/uapi" \
  -I"$K/include/uapi" -I"$K/include/generated/uapi" \
  -include "$K/include/linux/kconfig.h" \
  "$C/vmlinux.lds.S" -o "$C/vmlinux.lds"
grep -n 'balign' "$C/vmlinux.lds" && {
  echo "vmlinux.lds still contains .balign — preprocessor flags wrong" >&2
  exit 1
}

# ---- 7. link compressed/vmlinux with lccc-ld --------------------------------
OBJS=(
  "$C/kernel_info.o" "$C/head_64.o" "$C/misc.o" "$C/string.o"
  "$C/cmdline.o" "$C/error.o" "$C/piggy.o" "$C/cpuflags.o"
  "$C/ident_map_64.o" "$C/idt_64.o" "$C/idt_handlers_64.o"
  "$C/pgtable_64.o"
)
grep -q '^CONFIG_EARLY_PRINTK=y$' .config && OBJS+=("$C/early_serial_console.o")

echo "LD   compressed/vmlinux"
"$LCCC_LD" -m elf_x86_64 --no-ld-generated-unwind-info \
  -pie --no-dynamic-linker -z noexecstack \
  -T "$C/vmlinux.lds" "${OBJS[@]}" "$S/lib.a" \
  -o "$C/vmlinux"

echo "compressed/vmlinux symbols:"
nm -n "$C/vmlinux" | grep -E ' (startup_64|input_data|_text|_end|z_input_len)$' || true
# SIMD audit of the decompressor (CR4.OSFXSR=0 until extract_kernel finishes)
if objdump -d "$C/vmlinux" | grep -E '[[:space:]](v?mov[ua]p[sd]|v?movdqu|v?movaps|addps|mulps|xmm|ymm)' | grep -v 'objdump' | head; then
  echo "WARNING: SIMD ops in compressed/vmlinux" >&2
else
  echo "SIMD audit: no xmm/ymm in compressed/vmlinux"
fi

# ---- 8. zoffset.h + real-mode setup ----------------------------------------
sed_zoffset='s/^\([0-9a-fA-F]*\) [a-zA-Z] \(startup_32\|efi.._stub_entry\|efi\(32\)\?_pe_entry\|input_data\|kernel_info\|_end\|_ehead\|_text\|_e\?data\|_e\?sbat\|z_.*\)$/#define ZO_\2 0x\1/p'
nm "$C/vmlinux" | sed -n "$sed_zoffset" > "$B/zoffset.h"
echo "zoffset.h:"
cat "$B/zoffset.h"

echo "setup (build_kernel_boot.sh)"
OUT="$OUT/setup" /home/user/lccc/scripts/build_kernel_boot.sh
# header.o was compiled against a stub zoffset in that script if it ran
# before zoffset existed; recompile header.o with the real one and relink.
RMF="-std=gnu11 -m16 -g -Os -march=i386 -mregparm=3 -fno-strict-aliasing -fomit-frame-pointer -fno-pic -mno-mmx -mno-sse -mpreferred-stack-boundary=2 -ffreestanding -ffunction-sections -fno-stack-protector -fno-asynchronous-unwind-tables -fcf-protection=none -fno-jump-tables -DSVGA_MODE=NORMAL_VGA"
SINC="-nostdinc -I$B -I$K/arch/x86/include -I$K/arch/x86/include/generated -I$K/include -I$K/include/generated -I$K/include/uapi -I$K/arch/x86/include/uapi -I$K/arch/x86/include/generated/uapi -I$K/include/generated/uapi -include $K/include/linux/compiler-version.h -include $K/include/linux/kconfig.h -include $K/include/linux/compiler_types.h -D__KERNEL__ -D_SETUP -DDISABLE_BRANCH_PROFILING -D__DISABLE_EXPORTS"
echo "CC   header.S (real zoffset.h)"
"$LCCC" $SINC $RMF -D__ASSEMBLY__ -I"$B" -c "$B/header.S" -o "$OUT/setup/header.o"
SETUP_LD="$OUT/setup/setup-gc.ld"
SETUP_OBJS=(a20 bioscall cmdline copy cpu cpuflags cpucheck early_serial_console
            edd header main memory pm pmjump printf regs string tty video
            video-mode version video-vga video-vesa video-bios)
SO=()
for o in "${SETUP_OBJS[@]}"; do SO+=("$OUT/setup/$o.o"); done
"$LCCC_LD" --gc-sections -m elf_i386 -z noexecstack -T "$SETUP_LD" \
  "${SO[@]}" -o "$B/setup.elf"
objcopy -O binary "$B/setup.elf" "$B/setup.bin"
objcopy -O binary -R .note -R .comment -S "$C/vmlinux" "$B/vmlinux.bin"

# ---- 9. bzImage = setup.bin (4k padded) + compressed vmlinux.bin -----------
echo "BUILD bzImage"
(dd if="$B/setup.bin" bs=4k conv=sync status=none; cat "$B/vmlinux.bin") > "$B/bzImage"
ls -l "$B/bzImage" "$C/vmlinux" "$B/setup.bin"
sha256sum "$B/bzImage"
echo "build_kernel_compressed: SUCCESS"
