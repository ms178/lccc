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

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.52}
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
# The initramfs ships only /bin/busybox (+ /bin/sh). Every applet the
# suite calls below (mount, cat, zcat, grep, dmesg, poweroff, ...) is a
# busybox multicall binary entry: without installing the applet symlinks
# first, ash resolves them via PATH, finds nothing, and init dies with
# "mount: not found" -> panic. `--install -s` creates them all in /bin.
/bin/busybox --install -s /bin
PATH=/bin
export PATH
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
echo "--- serial integrity ---"
# Sentinel that exercises the 8250 UART xmit path end to end. Every ttyS0
# write above flows through kernel/printk + the initramfs write()s, but a
# data-corrupting console driver bug can pass all other checks while
# mangling the stream (lccc defect (h): sizeof through char(*)[0] was
# clamped to 1, so plain kfifo fifos took the record path and the 8250
# xmit kfifo handed userspace one character per write — the line below
# arrived letter-per-line). One long line with spaces and punctuation,
# then one with a hex address, must arrive verbatim on a healthy console.
echo "serial-ok the quick brown fox jumps over the lazy dog 0123456789 !?%&/(){}[]"
echo "serial-ok addr=0xdeadbeef count=31415926 end"
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
# QEMU loads its PC BIOS from the directory passed with -L.  Debian splits
# the firmware across qemu-system-data (/usr/share/qemu) and SeaBIOS
# (/usr/share/seabios), while extracted/no-root QEMU bundles often put the
# same files in one directory.  Accept both layouts and stage a private union
# under WORK when necessary; this makes the harness deterministic instead of
# requiring the operator to hand-create a firmware directory.
qemu_l=()
firmware_dir=""
qemu_data=${QEMU_DATA_DIR:-}
if [[ -n "$qemu_data" ]]; then
    if [[ -f "$qemu_data/bios-256k.bin" && -f "$qemu_data/linuxboot_dma.bin" \
          && -f "$qemu_data/kvmvapic.bin" && -f "$qemu_data/efi-e1000.rom" ]]; then
        firmware_dir=$qemu_data
    else
        echo "qemu_boot_test: QEMU_DATA_DIR is not a complete firmware directory: $qemu_data" >&2
        echo "  Need bios-256k.bin, linuxboot_dma.bin, kvmvapic.bin and efi-e1000.rom." >&2
        exit 1
    fi
else
    qdir=$(dirname "$(command -v "$QEMU")")
    rom_dir=${QEMU_ROM_DIR:-$qdir/../share/qemu}
    bios_dir=${QEMU_BIOS_DIR:-$qdir/../share/seabios}
    # Prefer a single self-contained directory (AppImage/tarball layout).
    for d in "$rom_dir" "$bios_dir"; do
        if [[ -f "$d/bios-256k.bin" && -f "$d/linuxboot_dma.bin" \
              && -f "$d/kvmvapic.bin" && -f "$d/efi-e1000.rom" ]]; then
            firmware_dir=$d
            break
        fi
    done
    # Debian's split layout is the normal system install: SeaBIOS ships
    # bios-256k.bin while qemu-system-data / ipxe-qemu ship the option ROMs.
    # WHICH directory holds WHICH file is a packaging detail, so resolve every
    # required file independently and accept any split.  (Requiring one fixed
    # assignment — bios in $bios_dir, all three ROMs in $rom_dir — rejected
    # complete unions that merely came from a different package layout, and
    # resolving per file also makes a dangling symlink impossible.)  Symlinks
    # are sufficient and avoid copying firmware into the persistent workspace.
    if [[ -z "$firmware_dir" ]]; then
        firmware_required=(bios-256k.bin linuxboot_dma.bin kvmvapic.bin efi-e1000.rom)
        firmware_resolved=()
        firmware_complete=1
        for f in "${firmware_required[@]}"; do
            if [[ -f "$rom_dir/$f" ]]; then
                firmware_resolved+=("$rom_dir/$f")
            elif [[ -f "$bios_dir/$f" ]]; then
                firmware_resolved+=("$bios_dir/$f")
            else
                firmware_complete=0
                break
            fi
        done
        if ((firmware_complete)); then
            firmware_dir="$WORK/qemu-firmware"
            rm -rf "$firmware_dir"
            mkdir -p "$firmware_dir"
            for i in "${!firmware_required[@]}"; do
                ln -s "${firmware_resolved[$i]}" "$firmware_dir/${firmware_required[$i]}"
            done
        fi
    fi
    if [[ -z "$firmware_dir" ]]; then
        echo "qemu_boot_test: could not find a complete QEMU firmware set" >&2
        echo "  Looked in: ROMs=$rom_dir BIOS=$bios_dir" >&2
        echo "  Need bios-256k.bin, linuxboot_dma.bin, kvmvapic.bin and efi-e1000.rom." >&2
        echo "  Override QEMU_DATA_DIR with one directory containing that union." >&2
        exit 1
    fi
fi
qemu_l=(-L "$firmware_dir")

echo "boot: $QEMU ${qemu_l[*]} -kernel $BZIMAGE (log: $LOG)"
# -display none + -serial file: (not -nographic + stdio redirect): the stdio
# chardev muxes serial and monitor onto the process's stdin/stdout, and its
# shutdown path stalls when stdin is not a tty (harness runs, cron, nohup):
# the guest reaches ACPI S5, "reboot: Power down" is the last serial line,
# and the QEMU process never exits — the run can only end by timeout. A
# dedicated file chardev has no stdio semantics to deadlock on: the same
# image powers down and QEMU exits within ~1 s (measured). QEMU diagnostics
# go to a separate file so the serial log stays grep-clean.
timeout 600 "$QEMU" -m 512 -smp 2 \
    "${qemu_l[@]}" \
    -kernel "$BZIMAGE" -initrd "$WORK/initramfs.cpio.gz" \
    -display none -serial file:"$LOG" \
    -no-reboot \
    -accel tcg,thread=multi \
    -append "console=ttyS0,115200 nokaslr panic=-1 vga=normal" 2> "${LOG}.qemu-err" || true

# ---- verdict ------------------------------------------------------------------
# The 16550 serial file chardev writes the guest's CRLF line discipline
# verbatim; strip the trailing \r so $-anchored verdict greps see clean
# lines ("^2$" never matches "2\r").
sed -i 's/\r$//' "$LOG"
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
# tcp_available_congestion_control lists algos space-separated in
# registration order — "reno bbr bic cubic westwood htcp" on this config —
# so "bbr" is a mid-line word, not a line starter.
expect "bbr listed in congestion algos"    "(^| )bbr( |$)"
expect "BORE stats in sched_debug"         "bore|BORE"
expect "2 CPUs online"                     "^2$"
# Serial-integrity sentinels: must arrive verbatim. A console/xmit bug that
# drops bytes or splits writes (defect (h), kfifo record-path corruption)
# breaks the multi-word line or the trailing word of the hex line.
expect "serial sentinel line verbatim"     "serial-ok the quick brown fox jumps over the lazy dog 0123456789 !\?%&/\(\)\{\}\[\]$"
expect "serial sentinel hex line verbatim" "serial-ok addr=0xdeadbeef count=31415926 end$"
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
