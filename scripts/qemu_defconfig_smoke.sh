#!/usr/bin/env bash
# Minimal QEMU smoke test for the *actual* LCCC-built x86_64 defconfig image.
# Unlike qemu_boot_test.sh this does not require the custom VM config fragment.
# The static BusyBox is initramfs user space, not kernel or setup code.
set -euo pipefail
image=${1:?supply the LCCC-built bzImage}
QEMU=${QEMU:-qemu-system-x86_64}
BUSYBOX=${BUSYBOX:-/usr/bin/busybox}
QEMU_CPU=${QEMU_CPU:-qemu64}
BOOT_LOG=${BOOT_LOG:-/tmp/lccc-defconfig-qemu.log}
BOOT_TIMEOUT=${BOOT_TIMEOUT:-240}
BOOT_TRACE_INITCALLS=${BOOT_TRACE_INITCALLS-0}
case $BOOT_TRACE_INITCALLS in
  0|1) ;;
  *) echo 'BOOT_TRACE_INITCALLS must be 0 or 1' >&2; exit 1 ;;
esac
kernel_cmdline='console=ttyS0,115200 nokaslr panic=-1 vga=normal'
if [[ $BOOT_TRACE_INITCALLS == 1 ]]; then
  kernel_cmdline+=' initcall_debug ignore_loglevel'
fi
[[ -f "$image" ]] || { echo "missing kernel image: $image" >&2; exit 1; }
command -v "$QEMU" >/dev/null || { echo "missing QEMU: $QEMU" >&2; exit 1; }
[[ -x "$BUSYBOX" ]] && file "$BUSYBOX" | grep -q 'statically linked' || {
  echo "need a statically linked BusyBox: $BUSYBOX" >&2; exit 1;
}
[[ $BOOT_TIMEOUT =~ ^[1-9][0-9]*$ ]] || { echo 'BOOT_TIMEOUT must be a positive integer' >&2; exit 1; }
# A rootless `find | cpio` cannot create /dev/console. Linux opens that node
# before running /init, so it must already exist in the archive, not be made by
# the init script. Use the genuine host tool built alongside the kernel.
image=$(realpath -- "$image")
kernel_root=$(dirname -- "$(dirname -- "$(dirname -- "$(dirname -- "$image")")")")
GEN_INIT_CPIO=${GEN_INIT_CPIO:-$kernel_root/usr/gen_init_cpio}
[[ -x "$GEN_INIT_CPIO" ]] || {
  echo "need the kernel host tool gen_init_cpio (set GEN_INIT_CPIO): $GEN_INIT_CPIO" >&2
  exit 1
}
GEN_INIT_CPIO=$(realpath -- "$GEN_INIT_CPIO")
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-defconfig-boot.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/root/bin" "$work/firmware"
cp "$BUSYBOX" "$work/root/bin/busybox"
cat > "$work/root/init" <<'INIT'
#!/bin/busybox sh
/bin/busybox --install -s /bin
export PATH=/bin
mount -t proc proc /proc || { echo LCCC_SMOKE_FAIL_PROC; poweroff -f; }
mount -t sysfs sys /sys || { echo LCCC_SMOKE_FAIL_SYS; poweroff -f; }
echo LCCC_DEFCFG_BOOT_BEGIN
cat /proc/version
printf 'LCCC_DEFCFG_CPUS='; grep -c '^processor' /proc/cpuinfo
echo 'LCCC_DEFCFG_SERIAL=the quick brown fox 0123456789 !?'
# Exercise two runnable processes, demand-zero pages, tmpfs allocation, VFS
# writes/reads and data integrity. Each worker's status is checked separately:
# plain `wait` would conceal a failure in any worker except the last one.
fail() { echo "LCCC_SMOKE_FAIL_$1"; poweroff -f; exit 1; }
mount -t devtmpfs devtmpfs /dev || fail DEVTMPFS
mkdir -p /tmp || fail MKDIR
mount -t tmpfs -o size=32m tmpfs /tmp || fail TMPFS
pids=
for worker in a b; do
  (
    dd if=/dev/zero of="/tmp/data-$worker" bs=4096 count=2048 2>/dev/null &&
    echo "2daeb1f36095b44b318410b3f4e8b5d989dcc7bb023d1426c492dab0a3053e74  /tmp/data-$worker" |
      sha256sum -c - >/dev/null
  ) &
  pids="$pids $!"
done
for pid in $pids; do wait "$pid" || fail WORKER; done
cmp /tmp/data-a /tmp/data-b || fail COMPARE
sync || fail SYNC
echo LCCC_DEFCFG_MEMORY_IO_PASS
echo LCCC_DEFCFG_BOOT_PASS
poweroff -f
INIT
# Directory, symlink and executable-mode metadata belong to the archive;
# don't create privileged nodes or redundant staging-tree metadata on the host.
cat > "$work/initramfs.list" <<'CPIO'
dir /bin 755 0 0
dir /proc 555 0 0
dir /sys 555 0 0
dir /dev 755 0 0
nod /dev/console 600 0 0 c 5 1
nod /dev/null 666 0 0 c 1 3
file /bin/busybox bin/busybox 755 0 0
slink /bin/sh busybox 777 0 0
file /init init 755 0 0
CPIO
# Relative source paths in the manifest also handle whitespace in TMPDIR.
( cd "$work/root" && "$GEN_INIT_CPIO" -t 0 "$work/initramfs.list" | gzip -1 > "$work/initramfs.cpio.gz" )
# Debian splits the required firmware across QEMU and SeaBIOS. An explicit
# QEMU_DATA_DIR can instead name a complete self-contained firmware directory.
# Debian's iPXE ROMs live in /usr/lib/ipxe/qemu, not /usr/share/qemu.
# shellcheck source=qemu_firmware.sh
source "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/qemu_firmware.sh"
lccc_qemu_stage_firmware "$work/firmware"
# Write into the private directory, never through a predictable /tmp log path.
# A stale output log cannot make a failed QEMU invocation pass. Publish via an
# atomic same-directory rename (replacing a symlink, not following it).
serial="$work/serial.log"
set +e
timeout "$BOOT_TIMEOUT" "$QEMU" -L "$work/firmware" -machine pc -cpu "$QEMU_CPU" -m 768 -smp 2 \
  -kernel "$image" -initrd "$work/initramfs.cpio.gz" -display none -serial "file:$serial" \
  -no-reboot -accel tcg,thread=multi \
  -append "$kernel_cmdline" 2>"$work/qemu.err"
rc=$?
set -e
publish() { # publish <private-source> <requested-output>
  local temp
  temp=$(mktemp "${2}.tmp.XXXXXXXX") || return 1
  if ! cat "$1" > "$temp" || ! mv -f -- "$temp" "$2"; then
    rm -f -- "$temp"
    return 1
  fi
}
publish "$work/qemu.err" "$BOOT_LOG.qemu-err"
[[ -f "$serial" ]] || { echo "QEMU produced no serial log (status=$rc)" >&2; exit 1; }
sed -i 's/\r$//' "$serial"
publish "$serial" "$BOOT_LOG"
for marker in 'Linux version .*lccc' '^LCCC_DEFCFG_BOOT_BEGIN$' '^LCCC_DEFCFG_CPUS=2$' '^LCCC_DEFCFG_SERIAL=the quick brown fox 0123456789 !\?$' '^LCCC_DEFCFG_MEMORY_IO_PASS$' '^LCCC_DEFCFG_BOOT_PASS$' 'reboot: Power down'; do
  grep -qE "$marker" "$BOOT_LOG" || { echo "missing marker: $marker" >&2; tail -100 "$BOOT_LOG"; exit 1; }
done
if grep -qE 'LCCC_SMOKE_FAIL|Kernel panic|Oops:|WARNING:|BUG:|unable to open an initial console' "$BOOT_LOG"; then echo 'kernel error in serial log' >&2; exit 1; fi
[[ $rc -eq 0 ]] || { echo "QEMU exit status $rc" >&2; exit 1; }
echo "DEFCFG QEMU SMOKE PASS: $BOOT_LOG"
