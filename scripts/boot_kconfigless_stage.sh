#!/usr/bin/env bash
# ============================================================================
# boot_kconfigless_stage.sh — stage a kernel tree for build_kernel_boot.sh
# WITHOUT Kconfig (the sandbox has no flex/bison, so `make defconfig` and the
# generated-header build cannot run).
#
# What this stages (all derived from the pristine kernel.org tarball):
#   * the subset of the tree the boot gate reads (arch/x86/boot, includes,
#     scripts, arch/x86/{kernel/cpu,lib}, arch/x86 + top Makefiles)
#   * arch/x86/kernel/cpu/capflags.c via the in-tree mkcapflags.sh generator
#   * Kbuild-equivalent asm-generic wrappers (generated/{,uapi/}asm/*.h) for
#     the headers the boot closure requests (types, rwonce, ioctl, errno +
#     the arch Kbuild generic-y set)
#   * include/generated/{autoconf,utsrelease}.h (a defconfig-like CONFIG set;
#     the full Kconfig space is unreachable here, so this is the documented
#     approximation — A/B stays valid because both sides share the tree)
#   * arch/x86/boot/{voffset,zooffset}.h stubs (header.S uses no voffset
#     symbol; ZO_* are immediates whose values don't affect SIZE)
#   * the LCCC_BOOT_EXTRA export line spelling __init verbatim (init.h is
#     unreachable in the boot closure without the Kconfig environment; the
#     expansion below is copied token-for-token from include/linux/init.h +
#     compiler-{types,gcc}.h for __GNUC__ with sane function alignment)
#
# Usage:
#   ./boot_kconfigless_stage.sh /path/to/linux-6.18.50.tar.xz /path/to/workdir
#   eval "$(./boot_kconfigless_stage.sh ... | grep ^export)"  # sets LCCC_BOOT_EXTRA
#   KERNEL_DIR=/path/to/workdir/linux-6.18.50 LCCC=... OUT=... ./build_kernel_boot.sh
# ============================================================================
set -euo pipefail

TARBALL=${1:?usage: $0 <kernel.tar.xz> <workdir>}
WORK=${2:?usage: $0 <kernel.tar.xz> <workdir>}
mkdir -p "$WORK"
cd "$WORK"
tar -xJf "$TARBALL" \
    linux-6.18.50/arch/x86/boot \
    linux-6.18.50/arch/x86/include \
    linux-6.18.50/include \
    linux-6.18.50/scripts \
    linux-6.18.50/arch/x86/kernel/cpu \
    linux-6.18.50/arch/x86/tools \
    linux-6.18.50/arch/x86/lib \
    linux-6.18.50/arch/x86/Makefile \
    linux-6.18.50/Makefile
K="$WORK/linux-6.18.50"

# ---- capflags.c (in-tree generator) ----------------------------------------
sh "$K/arch/x86/kernel/cpu/mkcapflags.sh" \
    "$K/arch/x86/kernel/cpu/capflags.c" \
    "$K/arch/x86/include/asm/cpufeatures.h"

# ---- Kbuild-equivalent generated wrappers ----------------------------------
mkdir -p "$K/arch/x86/include/generated/asm" \
         "$K/arch/x86/include/generated/uapi/asm" \
         "$K/include/generated"
printf '#include <asm-generic/types.h>\n' \
    > "$K/arch/x86/include/generated/asm/types.h"
for h in rwonce.h early_ioremap.h fprobe.h mcs_spinlock.h mmzone.h ring_buffer.h; do
    printf '#include <asm-generic/%s>\n' "$h" \
        > "$K/arch/x86/include/generated/asm/$h"
done
for h in ioctl.h errno.h; do
    printf '#include <uapi/asm-generic/%s>\n' "$h" \
        > "$K/arch/x86/include/generated/uapi/asm/$h"
done
printf '#define UTS_RELEASE "6.18.50"\n' > "$K/include/generated/utsrelease.h"

# ---- cpufeaturemasks.h (in-tree awk generator + stub .config) ---------------
{
    echo "# kconfigless stub .config (mask-relevant entries only)"
    for f in FPU VME DE PSE TSC MSR PAE MCE CX8 APIC SEP MTRR PGE MCA CMOV \
             PAT PSE36 CLFLUSH MMX FXSR SSE SSE2 HTT; do
        echo "CONFIG_X86_REQUIRED_FEATURE_$f=y"
    done
} > "$K/.kconfigless-config"
awk -f "$K/arch/x86/tools/cpufeaturemasks.awk" \
    "$K/arch/x86/include/asm/cpufeatures.h" "$K/.kconfigless-config" \
    > "$K/arch/x86/include/generated/asm/cpufeaturemasks.h"

# ---- defconfig-like autoconf (documented approximation) ---------------------
cat > "$K/include/generated/autoconf.h" << 'EOF'
/* Rich x86_64-defconfig-like autoconf for the lccc boot gate (no Kconfig in sandbox). */
#define CONFIG_64BIT 1
#define CONFIG_X86_64 1
#define CONFIG_X86 1
#define CONFIG_MMU 1
#define CONFIG_SMP 1
#define CONFIG_X86_SMP 1
#define CONFIG_NR_CPUS 8192
#define CONFIG_X86_NEED_RELOCS 1
#define CONFIG_PHYSICAL_ALIGN 0x1000000
#define CONFIG_PHYSICAL_START 0x1000000
#define CONFIG_RELOCATABLE 1
#define CONFIG_RANDOMIZE_BASE 1
#define CONFIG_X86_MINIMUM_CPU_FAMILY 64
#define CONFIG_X86_CPUID 1
#define CONFIG_X86_MSR 1
#define CONFIG_X86_PAE 1
#define CONFIG_X86_PAT 1
#define CONFIG_X86_MTRR 1
#define CONFIG_EFI 1
#define CONFIG_EFI_STUB 1
#define CONFIG_EFI_MIXED 1
#define CONFIG_EFI_HANDOVER_PROTOCOL 1
#define CONFIG_EDD 1
#define CONFIG_FIRMWARE_EDID 1
#define CONFIG_BOOT_VESA_SUPPORT 1
#define CONFIG_KEXEC_CORE 1
#define CONFIG_CRASH_CORE 1
#define CONFIG_PRINTK 1
#define CONFIG_EARLY_PRINTK 1
#define CONFIG_TTY 1
#define CONFIG_MODULES 1
#define CONFIG_PCI 1
#define CONFIG_ACPI 1
#define CONFIG_PM 1
#define CONFIG_NET 1
#define CONFIG_INET 1
#define CONFIG_BLOCK 1
#define CONFIG_BLK_DEV_INITRD 1
#define CONFIG_CC_HAS_SANE_FUNCTION_ALIGNMENT 1
#define CONFIG_FUNCTION_ALIGNMENT 16
#define CONFIG_PAGE_SHIFT 12
#define CONFIG_JUMP_LABEL 1
#define CONFIG_NUMA 1
#define CONFIG_SHMEM 1
#define CONFIG_PROC_FS 1
#define CONFIG_SYSFS 1
#define CONFIG_SERIAL_8250 1
#define CONFIG_SERIAL_8250_CONSOLE 1
#define CONFIG_DRM 1
#define CONFIG_EXT4_FS 1
#define CONFIG_CGROUPS 1
#define CONFIG_SECURITY 1
#define CONFIG_CRYPTO 1
#define CONFIG_DECOMPRESS_GZIP 1
#define CONFIG_DECOMPRESS_XZ 1
#define CONFIG_DECOMPRESS_ZSTD 1
#define CONFIG_SWIOTLB 1
#define CONFIG_X86_IOMMU 1
#define CONFIG_MIGRATION 1
#define CONFIG_CMA 1
#define CONFIG_VIRTIO 1
#define CONFIG_DMI 1
#define CONFIG_MICROCODE 1
#define CONFIG_X86_MCE 1
#define CONFIG_X86_LOCAL_APIC 1
#define CONFIG_X86_IO_APIC 1
#define CONFIG_PARAVIRT 1
#define CONFIG_KVM_GUEST 1
EOF

# ---- .config mirror (build_kernel_boot.sh greps it for payload checks) -----
grep -o '^#define \(CONFIG_[A-Z0-9_]*\) 1$' "$K/include/generated/autoconf.h" \
    | sed 's/^#define \(.*\) 1$/\1=y/' > "$K/.config"

# ---- compile.h + utsversion.h (fixed strings: deterministic A/B) ------------
cat > "$K/include/generated/compile.h" << 'EOF'
#define UTS_MACHINE "x86_64"
#define LINUX_COMPILE_BY "lccc"
#define LINUX_COMPILE_HOST "kconfigless"
#define LINUX_COMPILE_TIME "00:00:00"
#define LINUX_COMPILE_DOMAIN "local"
EOF
printf '#define UTS_VERSION "#1 SMP PREEMPT_DYNAMIC kconfigless"\n' \
    > "$K/include/generated/utsversion.h"

# ---- boot stubs (SIZE-independent; see header comment) ----------------------
printf '/* stub: header.S uses no voffset symbol */\n' > "$K/arch/x86/boot/voffset.h"
{
    printf '/* stub: SIZE independent of ZO_* (immediates) */\n'
    for s in ZO__data ZO__edata ZO__ehead ZO__end ZO__esbat ZO__sbat \
             ZO_efi32_pe_entry ZO_efi32_stub_entry \
             ZO_efi_pe_entry ZO_INIT_SIZE ZO_RANGE ZO_input_data \
             ZO_kernel_info ZO_startup_32 ZO_z_extra_bytes \
             ZO_z_extract_offset ZO_z_input_len ZO_z_min_extract_offset \
             ZO_z_output_len; do
        printf '#define %s 0\n' "$s"
    done
    # header.S asserts ZO_efi32_stub_entry == ZO_efi64_stub_entry - 0x200.
    printf '#define ZO_efi64_stub_entry 0x200\n'
} > "$K/arch/x86/boot/zoffset.h"

echo "STAGED $K"
echo "export LCCC_BOOT_EXTRA='-D__init=__attribute__((__section__(\".init.text\")))__attribute__((__cold__))__attribute__((latent_entropy))'"
