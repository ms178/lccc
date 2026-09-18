#!/usr/bin/env bash
# ============================================================================
# kernel_isolate.sh — boot-failure isolation matrix for an LCCC-built kernel.
#
# When a kernel boots wrong (hang, oops, bad-pte, CPU bring-up timeout),
# the first question is WHICH dimension is guilty: SMP? userspace init?
# initramfs? memory size? This script runs the SAME bzImage through a
# matrix of QEMU configurations, captures each serial log, and prints a
# marker table so the failing dimension jumps out.
#
# Cases (override with KERNEL_ISOLATE_CASES="case1 case2 ..."):
#   smp2       -smp 2, full initramfs        (the reference configuration)
#   smp1       -smp 1, full initramfs        (SMP-sensitive defects vanish?)
#   maxcpus1   -smp 2 + maxcpus=1            (AP present but not brought up:
#                                             isolates SIPI/hotplug path from
#                                             general SMP code)
#   initsh     -smp 2, init=/bin/sh          (userspace init vs kernel)
#   noinitrd   -smp 2, no initrd             (how far without any rootfs)
#
# Every case gets its own serial log + QEMU stderr under the out-dir, plus a
# grep table of the interesting markers. Exit status is 0 unless a case fails
# to even START (QEMU misconfiguration) — the point is diagnosis, not pass/fail.
#
# Usage:
#   kernel_isolate.sh [bzImage]     (default: $KERNEL_DIR/arch/x86/boot/bzImage)
# Environment:
#   KERNEL_DIR     kernel tree (default /home/user/kernel-work/linux-6.18.52)
#   QEMU           qemu binary (default qemu-system-x86_64)
#   QEMU_DATA_DIR  firmware union dir; autodetected from the qemu install
#   BUSYBOX        static busybox for the initramfs (default /usr/bin/busybox)
#   OUT_DIR        report dir (default /tmp/kernel-isolate/<timestamp>)
#   KERNEL_ISOLATE_CASES   subset of the case list above
#   PER_CASE_TIMEOUT secs per case (default 240)
# ============================================================================
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.52}
BZIMAGE=${1:-$K/arch/x86/boot/bzImage}
QEMU=${QEMU:-qemu-system-x86_64}
BUSYBOX=${BUSYBOX:-/usr/bin/busybox}
OUT_DIR=${OUT_DIR:-/tmp/kernel-isolate/$(date +%Y%m%dT%H%M%S)}
CASES=${KERNEL_ISOLATE_CASES:-"smp2 smp1 maxcpus1 initsh noinitrd"}
TMO=${PER_CASE_TIMEOUT:-240}

[[ -f "$BZIMAGE" ]] || { echo "kernel_isolate: bzImage missing: $BZIMAGE" >&2; exit 1; }
command -v "$QEMU" >/dev/null 2>&1 || { echo "kernel_isolate: qemu missing: $QEMU" >&2; exit 1; }
[[ -x "$BUSYBOX" ]] || { echo "kernel_isolate: busybox missing: $BUSYBOX" >&2; exit 1; }

mkdir -p "$OUT_DIR"

# ---- firmware dir (BIOS + option ROM union, same rules as qemu_boot_test) ----
qemu_l=()
if [[ -n ${QEMU_DATA_DIR:-} ]]; then
    [[ -f "$QEMU_DATA_DIR/bios-256k.bin" ]] || {
        echo "kernel_isolate: QEMU_DATA_DIR has no bios-256k.bin: $QEMU_DATA_DIR" >&2; exit 1; }
    qemu_l=(-L "$QEMU_DATA_DIR")
else
    qdir=$(dirname "$(command -v "$QEMU")")
    for d in "$qdir/../share/qemu" "$qdir/../share/seabios"; do
        if [[ -f "$d/bios-256k.bin" && -f "$d/linuxboot_dma.bin" \
              && -f "$d/kvmvapic.bin" && -f "$d/efi-e1000.rom" ]]; then
            qemu_l=(-L "$d"); break
        fi
    done
    ((${#qemu_l[@]} > 0)) || { echo "kernel_isolate: no firmware union dir (see qemu_boot_test.sh)" >&2; exit 1; }
fi

# ---- minimal busybox initramfs -----------------------------------------------
# Smaller than qemu_boot_test's suite: boot markers + clean poweroff only.
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/root/bin" "$WORK/root/proc" "$WORK/root/dev"
cp "$BUSYBOX" "$WORK/root/bin/busybox"
cat > "$WORK/root/init" <<'EOF'
#!/bin/busybox sh
/bin/busybox --install -s /bin
export PATH=/bin
echo "ISOLATE: init started"
/bin/busybox mount -t proc proc /proc 2>/dev/null
echo "ISOLATE: CPUs online: $(/bin/busybox cat /sys/devices/system/cpu/online 2>/dev/null || echo '?')"
echo "ISOLATE: poweroff"
/bin/busybox poweroff -f
EOF
chmod +x "$WORK/root/init"
( cd "$WORK/root" && find . | cpio -o -H newc --quiet ) | gzip -9 > "$WORK/initramfs.cpio.gz"

run_case() {
    local name=$1 smp=$2 append_extra=$3 initrd_flag=$4
    local log="$OUT_DIR/$name.log"
    local cmdline="console=ttyS0,115200 nokaslr panic=-1 vga=normal $append_extra"
    echo "== $name: -smp $smp append='$append_extra' initrd=$initrd_flag"
    timeout "$TMO" "$QEMU" -m 512 -smp "$smp" \
        "${qemu_l[@]}" \
        -kernel "$BZIMAGE" \
        $initrd_flag \
        -display none -serial file:"$log" \
        -no-reboot -accel tcg,thread=multi \
        -append "$cmdline" 2> "$log.qemu-err" || true
    # markers: last boot stage reached + defects of interest
    local markers=("Booting the kernel" "smpboot: CPU" "bad pte" "BUG:" "panic"
                   "Run /init" "ISOLATE: init started" "ISOLATE: CPUs online"
                   "ISOLATE: poweroff" "reboot: Power down")
    printf '   %-28s' "$name:"
    for m in "${markers[@]}"; do
        if grep -q "$m" "$log" 2>/dev/null; then
            case $m in
                "smpboot: CPU") printf ' cpu=%s' "$(grep -c 'smpboot: CPU' "$log")" ;;
                *) printf ' [%s]' "$m" ;;
            esac
        fi
    done
    printf ' lines=%s\n' "$(wc -l < "$log" 2>/dev/null || echo 0)"
}

echo "kernel_isolate: $BZIMAGE"
echo "logs + report: $OUT_DIR"
for c in $CASES; do
    case $c in
        smp2)     run_case smp2     2 ""                    "-initrd $WORK/initramfs.cpio.gz" ;;
        smp1)     run_case smp1     1 ""                    "-initrd $WORK/initramfs.cpio.gz" ;;
        maxcpus1) run_case maxcpus1 2 "maxcpus=1"           "-initrd $WORK/initramfs.cpio.gz" ;;
        initsh)   run_case initsh   2 "init=/bin/sh rw"     "-initrd $WORK/initramfs.cpio.gz" ;;
        noinitrd) run_case noinitrd 2 ""                    "" ;;
        *) echo "kernel_isolate: unknown case '$c' (skipped)" >&2 ;;
    esac
done

echo
echo "Full serial logs: $OUT_DIR/<case>.log — diff smp1 vs smp2 / maxcpus1 vs smp2"
echo "to localize SMP-sensitive divergence; grep 'bad pte\|BUG\|panic' for defects."
