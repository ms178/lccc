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
[[ -f "$image" ]] || { echo "missing kernel image: $image" >&2; exit 1; }
command -v "$QEMU" >/dev/null || { echo "missing QEMU: $QEMU" >&2; exit 1; }
[[ -x "$BUSYBOX" ]] && file "$BUSYBOX" | grep -q 'statically linked' || {
  echo "need a statically linked BusyBox: $BUSYBOX" >&2; exit 1;
}
[[ $BOOT_TIMEOUT =~ ^[1-9][0-9]*$ ]] || { echo 'BOOT_TIMEOUT must be a positive integer' >&2; exit 1; }
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-defconfig-boot.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/root/bin" "$work/root/proc" "$work/root/sys" "$work/root/dev" "$work/firmware"
cp "$BUSYBOX" "$work/root/bin/busybox"
ln -s busybox "$work/root/bin/sh"
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
echo LCCC_DEFCFG_BOOT_PASS
poweroff -f
INIT
chmod +x "$work/root/init"
( cd "$work/root" && find . -print0 | cpio --null -o --format=newc 2>/dev/null | gzip -1 > "$work/initramfs.cpio.gz" )
# Debian splits the required firmware across QEMU and SeaBIOS. An explicit
# QEMU_DATA_DIR can instead name a complete self-contained firmware directory.
for f in bios-256k.bin linuxboot_dma.bin kvmvapic.bin efi-e1000.rom; do
  found=0
  for d in ${QEMU_DATA_DIR:+"$QEMU_DATA_DIR"} /usr/share/qemu /usr/share/seabios; do
    if [[ -f "$d/$f" ]]; then ln -s "$d/$f" "$work/firmware/$f"; found=1; break; fi
  done
  ((found)) || { echo "missing QEMU firmware: $f" >&2; exit 1; }
done
# Write into the private directory, never through a predictable /tmp log path.
# A stale output log cannot make a failed QEMU invocation pass. Publish via an
# atomic same-directory rename (replacing a symlink, not following it).
serial="$work/serial.log"
set +e
timeout "$BOOT_TIMEOUT" "$QEMU" -L "$work/firmware" -machine pc -cpu "$QEMU_CPU" -m 768 -smp 2 \
  -kernel "$image" -initrd "$work/initramfs.cpio.gz" -display none -serial "file:$serial" \
  -no-reboot -accel tcg,thread=multi \
  -append 'console=ttyS0,115200 nokaslr panic=-1 vga=normal' 2>"$work/qemu.err"
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
for marker in 'Linux version .*lccc' '^LCCC_DEFCFG_BOOT_BEGIN$' '^LCCC_DEFCFG_CPUS=2$' '^LCCC_DEFCFG_SERIAL=the quick brown fox 0123456789 !\?$' '^LCCC_DEFCFG_BOOT_PASS$' 'reboot: Power down'; do
  grep -qE "$marker" "$BOOT_LOG" || { echo "missing marker: $marker" >&2; tail -100 "$BOOT_LOG"; exit 1; }
done
if grep -qE 'LCCC_SMOKE_FAIL|Kernel panic|Oops:' "$BOOT_LOG"; then echo 'kernel error in serial log' >&2; exit 1; fi
[[ $rc -eq 0 ]] || { echo "QEMU exit status $rc" >&2; exit 1; }
echo "DEFCFG QEMU SMOKE PASS: $BOOT_LOG"
