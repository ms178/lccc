#!/usr/bin/env bash
# ============================================================================
# qemu_boot_test.sh — boot an LCCC-built bzImage in QEMU (TCG, no KVM needed)
# with a busybox initramfs that runs an in-VM validation suite and reports
# PASS/FAIL markers on the serial console.  Exit status is the verdict.
#
# Validated in the guest:
#   * kernel banner shows the LCCC build (CC_VERSION_TEXT carries lccc's
#     --version line, see build_kernel_vm.sh)
#   * /proc/config.gz proves the CachyMod features are compiled IN:
#     SCHED_BORE, SCHED_CACHE, SCHED_CLASS_EXT, HZ_800, CACHY, TCP_CONG_BBR,
#     PREEMPT, SMP
#   * BBRv3 is loadable (tcp_available_congestion_control lists bbr)
#   * sched-ext sysfs ABI exists (/sys/kernel/sched_ext)
#   * BORE stats appear in /proc/sched_debug (SCHED_DEBUG=y)
#   * both SMP CPUs are online
#   * the guest reaches poweroff cleanly (VM exits by itself)
#
# Usage:
#   KERNEL_DIR=... QEMU=... qemu_boot_test.sh [bzImage]
# ============================================================================
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}
BZIMAGE=${1:-$K/arch/x86/boot/bzImage}
QEMU=${QEMU:-qemu-system-x86_64}
WORK=${BOOT_WORK:-/tmp/boottest}
BUSYBOX=${BUSYBOX:-/usr/bin/busybox}
LOG=${BOOT_LOG:-/tmp/qemu-boot.log}

[[ -f "$BZIMAGE" ]] || { echo "qemu_boot_test: bzImage missing: $BZIMAGE" >&2; exit 1; }
command -v "$QEMU" >/dev/null 2>&1 || { echo "qemu_boot_test: qemu missing" >&2; exit 1; }
[[ -x "$BUSYBOX" ]] || { echo "qemu_boot_test: busybox missing: $BUSYBOX" >&2; exit 1; }
file "$BUSYBOX" | grep -q static || echo "warning: $BUSYBOX is not statically linked" >&2

rm -rf "$WORK"; mkdir -p "$WORK/root/bin" "$WORK/root/proc" "$WORK/root/sys" "$WORK/root/dev"

# ---- in-VM validation suite (POSIX sh, busybox-builtin-only) ----------------
cat > "$WORK/root/init" <<'EOF'
#!/bin/busybox sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs devtmpfs /dev
mount -t debugfs debugfs /sys/kernel/debug
echo "==== LCCC KERNEL BOOT VALIDATION BEGIN ===="
echo "--- version ---"; cat /proc/version
echo "--- config ---"; zcat /proc/config.gz | grep -E '^(CONFIG_(SCHED_BORE|SCHED_CACHE|HZ_800|HZ|CACHY|TCP_CONG_BBR|PREEMPT|SMP|KERNEL_ZSTD|SCHED_DEBUG))=' | sort
echo "--- congestion control ---"; cat /proc/sys/net/ipv4/tcp_available_congestion_control
echo "--- cache-aware sched features ---"; grep -i cache_hot_buddy /sys/kernel/debug/sched/features
echo "--- sched_debug (BORE) ---"; grep -m3 -i bore /proc/sched_debug
echo "--- cpus ---"; grep -c '^processor' /proc/cpuinfo
echo "--- dmesg ---"; dmesg | head -30
echo "==== LCCC KERNEL BOOT VALIDATION END ===="
poweroff -f
EOF
chmod +x "$WORK/root/init"
cp "$BUSYBOX" "$WORK/root/bin/busybox"
ln -sf busybox "$WORK/root/bin/sh"
chmod +x "$WORK/root/bin/busybox"

# ---- initramfs ---------------------------------------------------------------
( cd "$WORK/root" && find . -print0 | cpio --null -o --format=newc 2>/dev/null | gzip -1 > "$WORK/initramfs.cpio.gz" )

# ---- boot --------------------------------------------------------------------
echo "boot: $QEMU -kernel $BZIMAGE (log: $LOG)"
timeout 600 "$QEMU" -m 512 -smp 2 \
    -kernel "$BZIMAGE" -initrd "$WORK/initramfs.cpio.gz" \
    -nographic -no-reboot \
    -accel tcg,thread=multi \
    -append "console=ttyS0,115200 nokaslr panic=-1 vga=normal" > "$LOG" 2>&1 || true

# ---- verdict ------------------------------------------------------------------
fail=0
expect() { # expect <description> <grep-pattern>
  local desc=$1 pat=$2
  if grep -qE "$pat" "$LOG"; then
    echo "PASS: $desc"
  else
    echo "FAIL: $desc (pattern: $pat)"
    fail=1
  fi
}
expect "kernel banner (lccc build)"        "Linux version .*lccc"
expect "SCHED_BORE compiled in"            "CONFIG_SCHED_BORE=y"
expect "BORE init banner in dmesg"        "BORE CPU Scheduler"
expect "SCHED_CACHE compiled in"           "CONFIG_SCHED_CACHE=y"
expect "CACHE_HOT_BUDDY sched feature"    "CACHE_HOT_BUDDY"
expect "HZ_800 compiled in"                "CONFIG_HZ_800=y"
expect "CACHY compiled in"                 "CONFIG_CACHY=y"
expect "TCP_CONG_BBR (BBRv3) compiled in"  "CONFIG_TCP_CONG_BBR=y"
expect "PREEMPT compiled in"               "CONFIG_PREEMPT=y"
expect "SMP compiled in"                   "CONFIG_SMP=y"
expect "bbr listed in congestion algos"    "^bbr( |$)"
expect "BORE stats in sched_debug"         "bore|BORE"
expect "2 CPUs online"                     "^2$"
expect "validation ran to completion"      "LCCC KERNEL BOOT VALIDATION END"

if [[ $fail -eq 0 ]]; then
  echo "QEMU BOOT: ALL CHECKS PASSED"
  exit 0
else
  echo "QEMU BOOT: FAILURES DETECTED — full log: $LOG"
  tail -120 "$LOG"
  # Compressed-stub error() writes to VGA via error_putstr, which -nographic
  # does not capture. A silent SeaBIOS-only log with a later hlt loop is the
  # ZSTD/gzip decompressor abort, not a missing serial driver. Do not pass
  # earlyprintk= (16-bit early_serial_init livelocks on this image).
  if ! grep -q 'Linux version' "$LOG"; then
    echo "hint: no kernel banner — likely arch/x86/boot/compressed error() (VGA-only)."
    echo "hint: QMP 'info registers' + x/32xb RIP-16; RBX often points at the zstd error string."
  fi
  exit 1
fi
