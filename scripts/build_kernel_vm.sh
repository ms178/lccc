#!/usr/bin/env bash
# ============================================================================
# build_kernel_vm.sh — build a full x86_64 bzImage of linux-cachymod-6.18
# with LCCC as compiler and lccc-ld as linker, using a QEMU-bootable minimal
# config (allnoconfig + scripts/kernel-vm.fragment) that keeps every CachyMod
# patch functional (BORE, 800 Hz, cache-aware scheduling, sched-ext, CACHY,
# BBRv3, full preemption).
#
# Kbuild integration points:
#   * CC=lccc        — every .c and .S translation unit (lccc passes itself
#                      off as GCC 14.2.0 for --version, so Kbuild selects the
#                      GCC code paths: CC_IS_GCC=y).
#   * LD=lccc-ld     — arch/x86/boot/setup.elf, arch/x86/boot/compressed/
#                      vmlinux and the final vmlinux link (link-vmlinux.sh).
#   * HOSTCC=gcc     — host tools (Kconfig, modpost, objcopy wrappers, relocs,
#                      mkcpustr, ...) stay on the system toolchain; they are
#                      build machines, not generated code.
#
# Usage:
#   KERNEL_DIR=... LCCC=... LCCC_LD=... build_kernel_vm.sh [bzImage]
# ============================================================================
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-/home/user/lccc/target/fastbuild/lccc-ld}
LOG=${BUILD_LOG:-/tmp/kernel-build-lccc.log}
FRAGMENT=${FRAGMENT:-/home/user/lccc/scripts/kernel-vm.fragment}
JOBS=${JOBS:-2}

[[ -d "$K" ]] || { echo "build_kernel_vm: kernel tree missing: $K" >&2; exit 1; }
[[ -x "$LCCC" ]] || { echo "build_kernel_vm: lccc missing: $LCCC" >&2; exit 1; }
[[ -x "$LCCC_LD" ]] || { echo "build_kernel_vm: lccc-ld missing: $LCCC_LD" >&2; exit 1; }
cd "$K"

if [[ ! -f .lccc-vm-config ]]; then
  echo "config: allnoconfig + kernel-vm.fragment + olddefconfig"
  make ARCH=x86_64 allnoconfig >/dev/null
  scripts/kconfig/merge_config.sh -m .config "$FRAGMENT" >/dev/null
  make ARCH=x86_64 olddefconfig >/dev/null

  # Every CachyMod patch must stay FUNCTIONAL, not merely applied.
  # (SCHED_CLASS_EXT needs DEBUG_INFO_BTF on 6.18 — deferred to milestone 2,
  #  see engineering/tasks/; the ms178-custom patches are all covered here.)
  REQUIRED=(CACHY SCHED_BORE SCHED_CACHE HZ_800 PREEMPT
            TCP_CONG_BBR SMP SERIAL_8250_CONSOLE IKCONFIG_PROC BLK_DEV_INITRD
            DEVTMPFS KERNEL_ZSTD)
  missing=()
  for sym in "${REQUIRED[@]}"; do
    grep -q "^CONFIG_${sym}=y$" .config || missing+=("$sym")
  done
  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "build_kernel_vm: fragment failed to enable: ${missing[*]}" >&2
    exit 1
  fi
  # A disabled symbol may be spelled `CONFIG_X=n` or `# CONFIG_X is not set`.
  off() { grep -qE "^CONFIG_${1}=n$|^# CONFIG_${1} is not set$" .config; }
  # MODULES must stay off for milestone 1 (no module linking yet).  OBJTOOL is
  # arch-forced =y on x86_64 (HAVE_STATIC_CALL_INLINE/HAVE_UACCESS_VALIDATION
  # select it); the objtool HOST tool then validates every LCCC-produced
  # object, which is a correctness feature, not a problem.
  off MODULES || {
    echo "build_kernel_vm: modules must stay disabled for milestone 1" >&2
    exit 1
  }
  # A `choice` symbol without a default (RANDSTRUCT on 6.18.4x) cannot be
  # auto-answered by a mid-build `syncconfig` re-run: with stdin closed it
  # dies with "Error in reading or end of file" AFTER hundreds of objects
  # compiled. olddefconfig answers it, but the kernel re-runs syncconfig
  # whenever the config is touched, so VERIFY the config is fully answered
  # up front: a syncconfig dry-run here fails in seconds, not after the
  # compile phase.
  make ARCH=x86_64 syncconfig >/dev/null || {
    echo "build_kernel_vm: syncconfig cannot auto-answer the config (stale .config?); regenerating" >&2
    make ARCH=x86_64 allnoconfig >/dev/null
    scripts/kconfig/merge_config.sh -m .config "$FRAGMENT" >/dev/null
    make ARCH=x86_64 olddefconfig >/dev/null
    make ARCH=x86_64 syncconfig >/dev/null || {
      echo "build_kernel_vm: config still fails syncconfig after regeneration" >&2
      exit 1
    }
  }
  touch .lccc-vm-config
fi

# ---- host tool preflight ----------------------------------------------------
# bzImage's last step shells out to a host compression tool selected by
# CONFIG_KERNEL_*. A missing one fails AFTER the whole kernel has compiled
# (observed: `zstd: not found` at arch/x86/boot/compressed/vmlinux.bin.zst,
# ~20 minutes into the build). Check every host tool the remaining steps need
# up front so the failure costs seconds instead of a full build.
preflight_host_tools() {
  local missing=() tool sym
  # Compression tool implied by the config (Kbuild's compressed/Makefile).
  for sym in GZIP BZIP2 LZMA XZ LZO LZ4 ZSTD; do
    if grep -q "^CONFIG_KERNEL_${sym}=y$" .config; then
      case $sym in
        GZIP) tool=gzip ;;
        BZIP2) tool=bzip2 ;;
        LZMA) tool=lzma ;;
        XZ) tool=xz ;;
        LZO) tool=lzop ;;
        LZ4) tool=lz4 ;;
        ZSTD) tool=zstd ;;
      esac
      command -v "$tool" >/dev/null 2>&1 || missing+=("$tool (CONFIG_KERNEL_$sym)")
    fi
  done
  # Unconditional build-time host tools for a bzImage link.
  # flex/bison drive kconfig, pahole drives CONFIG_DEBUG_INFO_BTF; both are
  # host-only and both are absent on a minimal image.
  for tool in nm objcopy ar size perl awk sed bc cpio flex bison; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if grep -q "^CONFIG_DEBUG_INFO_BTF=y$" .config 2>/dev/null; then
    command -v pahole >/dev/null 2>&1 || missing+=("pahole (CONFIG_DEBUG_INFO_BTF)")
  fi
  if ((${#missing[@]})); then
    echo "build_kernel_vm: missing host tools: ${missing[*]}" >&2
    echo "  These are build machines only; they are never built with LCCC." >&2
    echo "  Debian/Ubuntu: apt-get install build-essential flex bison libelf-dev libssl-dev \\" >&2
    echo "    zstd xz-utils lz4 lzop bzip2 binutils cpio bc kmod dwarves" >&2
    return 1
  fi
  echo "preflight: host tools OK"
}
preflight_host_tools || exit 1

start=$(date +%s)
# Refresh include/config/auto.conf SERIALLY before the parallel build.  A
# -j2 `bzImage` re-runs syncconfig as part of `prepare`, and when auto.conf
# is stale that re-run can prompt for NEW symbols (cc-option-dependent
# visibility differs between the config-generation and build contexts) and
# die with "Error in reading or end of file" — after the config phase
# already passed.  A serial syncconfig here makes the build's own
# syncconfig a no-op (auto.conf is fresh), eliminating the race entirely.
make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" HOSTCC=gcc syncconfig >/dev/null || {
  echo "build_kernel_vm: serial syncconfig refresh failed" >&2
  exit 1
}
echo "build: make CC=$LCCC LD=$LCCC_LD HOSTCC=gcc -j$JOBS bzImage (log: $LOG)"
set +e
make ARCH=x86_64 \
     CC="$LCCC" LD="$LCCC_LD" HOSTCC=gcc \
     CC_VERSION_TEXT="$( "$LCCC" --version 2>/dev/null | head -1 )" \
     -j"$JOBS" V=1 bzImage > "$LOG" 2>&1
rc=$?
set -e
elapsed=$(( $(date +%s) - start ))
if [[ $rc -ne 0 ]]; then
  echo "build: FAILED after ${elapsed}s (exit $rc)"
  echo "---- first errors in $LOG ----"
  grep -E -m 12 -B2 -A8 '(^|: )(error:|fatal error:|Error|ld: |undefined reference|segment fault|internal compiler|assertion)' "$LOG" | head -80 || tail -60 "$LOG"
  exit $rc
fi

echo "build: SUCCESS in ${elapsed}s"
ls -l arch/x86/boot/bzImage vmlinux
size vmlinux
# The 32 KiB setup gate must hold for a bootable bzImage.
# (mawk has no strtonum — convert via bash arithmetic.)
while read -r hex; do
  dec=$((16#$hex))
  if (( dec > 32768 )); then
    echo "setup _end: 0x$hex ($dec bytes, OVER the 32768-byte gate)"
    exit 1
  fi
  echo "setup _end: 0x$hex ($dec bytes, ok of 32768)"
done < <(nm -n arch/x86/boot/setup.elf 2>/dev/null | awk '$3=="_end"{print $1}')
sha256sum arch/x86/boot/bzImage
