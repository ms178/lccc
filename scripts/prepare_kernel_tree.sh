#!/usr/bin/env bash
# ============================================================================
# prepare_kernel_tree.sh — regenerate the patched linux-cachymod-6.18.47 tree
# used by build_kernel_boot.sh / realmode_corpus.sh.
#
# The Arena workspace snapshot is capped (~128 MiB / 10k files), so the
# ~55k-file kernel tree and the 155 MiB tarball do NOT survive a harness wipe
# between turns even though /home/user is persisted.  This script makes the
# tree cheap to regenerate deterministically (~3 min): download, extract,
# apply the 26 CachyMod patches in PKGBUILD source order, configure with the
# package's real config, run `make prepare` (host tooling = gcc), and create
# the boot-code stubs (capflags.c, utsversion.h, zoffset.h, voffset.h) that a
# full Kbuild would otherwise generate.
#
# Idempotent: a completed tree is stamped with .lccc-prepared and skipped.
# The canary set below must all exist; the ~10k-file snapshot cap truncates
# large trees, so a damaged tree regenerates instead of half-working.
LCCC_PREPARED_CANARIES=(
  arch/x86/boot/setup.ld
  include/generated/autoconf.h
  include/generated/utsversion.h
  arch/x86/include/generated/asm/rwonce.h
  arch/x86/kernel/cpu/capflags.c
  arch/x86/boot/zoffset.h
  arch/x86/boot/voffset.h
  arch/x86/boot/cpustr.h
)
#
# Usage:
#   prepare_kernel_tree.sh [kernel-dir]          (default: /home/user/kernel-work/linux-6.18.47)
# Environment:
#   PKGDIR   archpkgbuilds sparse checkout of packages/linux-cachymod-6.18
#            (default: /home/user/archpkgbuilds/packages/linux-cachymod-6.18)
#   KVER     kernel version (default 6.18.47)
# ============================================================================
set -euo pipefail

KDIR=${1:-${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}}
PKGDIR=${PKGDIR:-/home/user/archpkgbuilds/packages/linux-cachymod-6.18}
KVER=${KVER:-6.18.47}
WORK=$(dirname "$KDIR")
TARBALL="$WORK/linux-$KVER.tar.xz"

[[ -d $PKGDIR ]] || { echo "prepare_kernel_tree: PKGDIR not found: $PKGDIR" >&2; exit 1; }

# Already prepared?  Verify the stamp AND a canary file the snapshot truncation
# removed last time (setup.ld); a damaged tree must regenerate, not half-work.
if [[ -f "$KDIR/.lccc-prepared" ]] && [[ -f "$KDIR/${LCCC_PREPARED_CANARIES[0]}" ]]; then
  ok=1
  for c in "${LCCC_PREPARED_CANARIES[@]}"; do
    [[ -f "$KDIR/$c" ]] || { ok=0; echo "prepare_kernel_tree: canary missing: $c" >&2; break; }
  done
  if [[ $ok == 1 ]]; then
    echo "prepare_kernel_tree: $KDIR already prepared (stamp + canaries OK)"
    exit 0
  fi
fi

mkdir -p "$WORK"
cd "$WORK"

# ---- 1. source tarball ------------------------------------------------------
# The workspace snapshot truncates large files (~128 MB cap): a stale tarball
# can be present but truncated. `xz -t` validates integrity cheaply (~2 s);
# a corrupt archive is re-downloaded instead of failing mid-extract.
if [[ ! -f "linux-$KVER.tar.xz" ]] || ! xz -t "linux-$KVER.tar.xz" 2>/dev/null; then
  echo "prepare_kernel_tree: downloading linux-$KVER.tar.xz"
  curl -sSL -o "linux-$KVER.tar.xz" "https://cdn.kernel.org/pub/linux/kernel/v${KVER%%.*}.x/linux-$KVER.tar.xz"
  xz -t "linux-$KVER.tar.xz" || { echo "prepare_kernel_tree: download corrupt" >&2; exit 1; }
fi

# ---- 2. extract -------------------------------------------------------------
echo "prepare_kernel_tree: extracting"
rm -rf "$KDIR"
# tar may exit non-zero on benign "Directory renamed before its status could
# be extracted" warnings (a GNU tar quirk when a directory's metadata changes
# between tar's open and its later utime/chmod pass, seen under load on
# FUSE/overlayfs). Real corruption is caught by the `xz -t` above and by the
# sentinel check below; tolerate the warning stream but verify the tree.
tar_err=$(mktemp)
tar -xf "linux-$KVER.tar.xz" 2>"$tar_err" || true
grep -v "Directory renamed before its status could be extracted" "$tar_err" >&2 || true
rm -f "$tar_err"
# Post-extract integrity: the archive passed `xz -t`, so a short tree means a
# filesystem-level extraction problem. Check sentinel files spread across the
# tree (not just the top-level Makefile) before trusting it.
for sentinel in Makefile init/main.c arch/x86/Makefile kernel/sched/core.c include/linux/sched.h; do
  if [ ! -f "$KDIR/$sentinel" ]; then
    echo "prepare_kernel_tree: extraction incomplete (missing $sentinel); retrying" >&2
    rm -rf "$KDIR"
    tar -xf "linux-$KVER.tar.xz" || { echo "prepare_kernel_tree: tar failed on retry" >&2; exit 1; }
    break
  fi
done
for sentinel in Makefile init/main.c arch/x86/Makefile kernel/sched/core.c include/linux/sched.h; do
  [ -f "$KDIR/$sentinel" ] || { echo "prepare_kernel_tree: tar extraction incomplete ($sentinel missing)" >&2; exit 1; }
done
cd "$KDIR"

# ---- 3. apply the CachyMod patch series (PKGBUILD source order) -------------
# Mirrors prepare() of the linux-cachymod-6.18 PKGBUILD with the default
# options: _cpusched=eevdf, _prevent_avx2=no (0300 patch is not in source=).
PATCHES=(
  0000-rt.patch
  0004-bbr3.patch
  0005-cachy.patch
  0006-crypto.patch
  0007-fixes.patch
  0008-hdmi.patch
  0010-sched-ext.patch
  0040-revert-dot5-sched-change.patch
  0050-misc-sched-fixes.patch
  0060-fair-drm-sched.patch
  0100-kconfig-add-800Hz.patch
  0200-clearlinux-extras.patch
  0210-cachymod-misc.patch
  0260-fair-update-cachy-mods.patch
  0280-prefer-prevcpu-for-wakeup.patch
  0001-ms178.patch
  0002-ms178-stringopts.patch
  0001-raptorlake-ms178.patch
  0001-vega-ms178.patch
  0001-bore.patch
  0001-6.18.35-nap-v0.4.0.patch
  0001-amd-vram-ms178.patch
  0001-amd-explicit-sync-ms178.patch
  0400-cache-aware-scheduling-v4-ms178.patch
  0410-cache-aware-scheduling-cluster-aware-raptorlake.patch
  1020-r8169-rtl8125-multi-queue-godlike.patch
)
n=0
for p in "${PATCHES[@]}"; do
  if patch -Np1 --silent --forward < "$PKGDIR/$p" >/tmp/prepare-patch.log 2>&1; then
    n=$((n+1))
  elif grep -q "previously applied" /tmp/prepare-patch.log; then
    n=$((n+1))
  else
    echo "prepare_kernel_tree: patch FAILED: $p" >&2
    tail -5 /tmp/prepare-patch.log >&2
    exit 1
  fi
done
echo "prepare_kernel_tree: applied $n/${#PATCHES[@]} patches"

# ---- 4. config + generated headers (host gcc) -------------------------------
echo "prepare_kernel_tree: configuring (package config + olddefconfig + prepare)"
cp "$PKGDIR/config" .config
# olddefconfig answers new symbols with defaults; it reads no stdin.  (A
# `yes '' | make` here would abort the script under `set -o pipefail`: when
# make exits, `yes` dies of SIGPIPE and its 141 poisons the pipeline status.)
make ARCH=x86_64 olddefconfig >/dev/null
# NOTE: stdout must stay visible — a silently-short `make prepare` once left
# arch/x86/include/generated/asm/rwonce.h missing and every boot compile broke.
make ARCH=x86_64 prepare -j"$(nproc)"

# ---- 5. boot-code stubs a full Kbuild would generate ------------------------
# capflags.c (mkcapflags.sh over cpufeatures.h)
( cd arch/x86/kernel/cpu && sh mkcapflags.sh capflags.c \
    ../../include/asm/cpufeatures.h ../../include/asm/vmxfeatures.h )

# cpustr.h (arch/x86/boot/Makefile builds it with the mkcpustr host tool;
# realmode_corpus.sh needs it for cpu.c but does not drive Kbuild)
if [[ ! -f arch/x86/boot/cpustr.h ]]; then
  gcc -O2 -Iarch/x86/include -Iarch/x86/include/generated -Iinclude \
      arch/x86/boot/mkcpustr.c -o "$WORK/mkcpustr"
  "$WORK/mkcpustr" > arch/x86/boot/cpustr.h
fi

# utsversion.h (init/Makefile normally builds it from the build banner)
mkdir -p include/generated
printf '#define UTS_VERSION "#1 SMP PREEMPT_DYNAMIC"\n' > include/generated/utsversion.h

# zoffset.h / voffset.h (normally nm of compressed/vmlinux and vmlinux).
# header.o only consumes them as immediates, so code size is unaffected;
# efi32/efi64 entries must satisfy the CONFIG_EFI_MIXED .if in header.S.
cat > arch/x86/boot/zoffset.h <<'EOF'
/* STUB zoffset.h — normally generated from compressed/vmlinux (nm).
 * Values are immediates only; they do not affect setup.elf size. */
#define ZO_startup_32 0x1000
#define ZO_efi32_stub_entry 0x1000
#define ZO_efi64_stub_entry 0x1200
#define ZO_efi_pe_entry 0x1300
#define ZO_efi32_pe_entry 0x1400
#define ZO_input_data 0x200000
#define ZO_kernel_info 0x1500
#define ZO__end 0x800000
#define ZO__ehead 0x200
#define ZO__text 0x210
#define ZO__data 0x300
#define ZO__edata 0x800000
#define ZO__sbat 0x400
#define ZO__esbat 0x410
#define ZO_z_input_len 0x100000
#define ZO_z_output_len 0x400000
#define ZO_z_extract_offset 0x0
#define ZO_z_min_extract_offset 0x1000
#define ZO_z_extra_bytes 0x10000
#define ZO_INIT_SIZE 0x500000
EOF
cat > arch/x86/boot/voffset.h <<'EOF'
/* STUB voffset.h — normally generated from vmlinux (nm). Immediates only. */
#define VO__text 0x100000
#define VO__end  0x800000
EOF

# Verify every canary before stamping: a half-prepared tree must never pass.
for c in "${LCCC_PREPARED_CANARIES[@]}"; do
  [[ -f "$c" ]] || { echo "prepare_kernel_tree: post-check failed, missing: $c" >&2; exit 1; }
done
touch .lccc-prepared
echo "prepare_kernel_tree: $KDIR ready (stamp written)"
