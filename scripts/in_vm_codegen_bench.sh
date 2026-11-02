#!/usr/bin/env bash
# ============================================================================
# in_vm_codegen_bench.sh — deterministic, variance-free generated-code
# comparison of two kernels (or two builds of one kernel) under QEMU TCG.
#
# WHY INSTRUCTION COUNTS AND NOT SECONDS
# --------------------------------------
# The research host has no hardware PMU, and wall-clock time under TCG is
# dominated by host jitter, so neither cycles nor seconds are a usable
# code-quality signal. Under TCG *with -icount* the guest clock is virtual:
# identical inputs retire an identical instruction stream, run after run.
# Three runs of a fixed image with -icount all reported exactly 8,900,714
# instructions; the same image without -icount varied by ~0.1 %. Instruction
# count is therefore the metric: exact, reproducible, and directly
# proportional to generated-code quality for a fixed workload.
#
# WHAT IT MEASURES
# ----------------
#   * whole-boot instruction count (everything from reset to the guest's
#     poweroff), optionally
#   * per-function counts, using [start,end) ranges taken from `nm -S` of the
#     corresponding vmlinux, so a hot kernel function can be compared directly
#     between two compilers.
#
# The guest workload is a single static binary built ONCE with the host gcc and
# reused for every kernel, so the userspace contribution is a constant and any
# instruction delta is attributable to the kernel build under test.
#
# The script refuses to report a comparison if any run disagrees with its
# siblings: a non-deterministic measurement is not evidence.
#
# Usage:
#   in_vm_codegen_bench.sh --a <bzImage> [--a-vmlinux <vmlinux>] \
#                          --b <bzImage> [--b-vmlinux <vmlinux>] \
#                          [--reps N] [--functions fn1,fn2,...] \
#                          [--out <dir>]
#
# Requires: qemu-system-x86_64 (plugins enabled), a static busybox, gcc,
#           cpio, gzip, nm; scripts/qemu_icount_plugin/build.sh for the plugin.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
plugin_dir="$here/qemu_icount_plugin"
plugin="$plugin_dir/lccc_icount.so"

A=""; B=""; A_VMLINUX=""; B_VMLINUX=""
REPS=${REPS:-3}
FUNCS=""
OUT=${OUT:-/tmp/in-vm-bench}
QEMU=${QEMU:-qemu-system-x86_64}
BUSYBOX=${BUSYBOX:-/bin/busybox}
GUEST_CC=${GUEST_CC:-gcc}

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
    case $1 in
        --a) A=$2; shift 2;;
        --b) B=$2; shift 2;;
        --a-vmlinux) A_VMLINUX=$2; shift 2;;
        --b-vmlinux) B_VMLINUX=$2; shift 2;;
        --reps) REPS=$2; shift 2;;
        --functions) FUNCS=$2; shift 2;;
        --out) OUT=$2; shift 2;;
        -h|--help) sed -n '2,40p' "$0"; exit 0;;
        *) die "unknown argument: $1";;
    esac
done

[[ -n $A && -n $B ]] || die "--a and --b are required"
[[ -f $A ]] || die "bzImage A not found: $A"
[[ -f $B ]] || die "bzImage B not found: $B"
command -v "$QEMU" >/dev/null 2>&1 || die "qemu missing: $QEMU"
[[ -x $BUSYBOX ]] || die "static busybox missing: $BUSYBOX"
file "$BUSYBOX" 2>/dev/null | grep -q static || echo "warning: $BUSYBOX is not static" >&2

mkdir -p "$OUT"

# ---- 1. plugin ---------------------------------------------------------------
if [[ ! -f $plugin ]]; then
    "$plugin_dir/build.sh" "$plugin_dir" >/dev/null
fi
[[ -f $plugin ]] || die "TCG plugin not built: $plugin"

# ---- 2. guest workload: one static binary shared by every kernel -------------
# Kernel-code-heavy and reproducible: syscall traffic, memory traffic and
# integer work, all bounded so the runtime stays short under TCG.
cat > "$OUT/workload.c" <<'EOF'
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <time.h>

/* Fixed-seed PRNG so the workload is bit-identical on every run. */
static unsigned long s = 0x12345678UL;
static unsigned long rnd(void){ s ^= s << 13; s ^= s >> 7; s ^= s << 17; return s; }

#define N  (1u << 16)
static unsigned char buf[N], tmp[N];

static unsigned long crc32_tab(void){
    unsigned long c = 0;
    for (unsigned long i = 0; i < 256; i++){
        unsigned long v = i;
        for (int k = 0; k < 8; k++) v = (v & 1) ? 0xEDB88320UL ^ (v >> 1) : (v >> 1);
        c += v;
    }
    return c;
}
static unsigned long crc32(const unsigned char *p, size_t n){
    static unsigned long tab[256]; static int init = 0;
    if (!init){ for (unsigned long i=0;i<256;i++){ unsigned long v=i;
        for(int k=0;k<8;k++) v=(v&1)?0xEDB88320UL^(v>>1):(v>>1); tab[i]=v;} init=1; }
    unsigned long c = 0xFFFFFFFFUL;
    for (size_t i = 0; i < n; i++) c = tab[(c ^ p[i]) & 0xFF] ^ (c >> 8);
    return c ^ 0xFFFFFFFFUL;
}

int main(void){
    unsigned long acc = crc32_tab();
    for (int r = 0; r < 8; r++){
        for (size_t i = 0; i < N; i++) buf[i] = (unsigned char)(rnd() >> 13);
        acc ^= crc32(buf, N);
        memcpy(tmp, buf, N);            /* exercises the kernel's copy paths   */
        acc ^= crc32(tmp, N) + memcmp(tmp, buf, N);
    }
    /* syscall traffic: page faults, brk, file I/O */
    for (int r = 0; r < 32; r++){
        int fd = open("/proc/self/stat", O_RDONLY);
        if (fd >= 0){ char b[512]; ssize_t n = read(fd, b, sizeof b); if (n > 0) acc ^= crc32((unsigned char*)b, (size_t)n); close(fd); }
        void *p = malloc(1u << 16); memset(p, (int)(r & 0xFF), 1u << 16);
        acc ^= crc32((unsigned char*)p, 1u << 16); free(p);
    }
    printf("WORKLOAD_CHECKSUM %lu\n", acc);
    return 0;
}
EOF
if [[ ! -x "$OUT/workload" ]] || [[ "$OUT/workload.c" -nt "$OUT/workload" ]]; then
    "$GUEST_CC" -O2 -static -o "$OUT/workload" "$OUT/workload.c"
fi

# ---- 3. initramfs ------------------------------------------------------------
rm -rf "$OUT/root"; mkdir -p "$OUT/root/bin" "$OUT/root/proc" "$OUT/root/sys" "$OUT/root/dev"
cat > "$OUT/root/init" <<'EOF'
#!/bin/busybox sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs devtmpfs /dev
echo "==== IN-VM BENCH BEGIN ===="
/bin/workload
echo "==== IN-VM BENCH END ===="
poweroff -f
EOF
chmod +x "$OUT/root/init"
cp "$BUSYBOX"  "$OUT/root/bin/busybox"
cp "$OUT/workload" "$OUT/root/bin/workload"
ln -sf busybox "$OUT/root/bin/sh"
( cd "$OUT/root" && find . -print0 | cpio --null -o --format=newc 2>/dev/null | gzip -1 > "$OUT/initramfs.cpio.gz" )

# ---- 4. optional per-function ranges ----------------------------------------
range_args() { # range_args <vmlinux>
    local vmlinux=$1 out_args="" fn
    [[ -n $FUNCS && -n $vmlinux && -f $vmlinux ]] || { printf ''; return 0; }
    IFS=',' read -ra fns <<< "$FUNCS"
    for fn in "${fns[@]}"; do
        local line
        line=$(nm -S --defined-only "$vmlinux" 2>/dev/null | awk -v f="$fn" '$4==f {print $1" "$2; exit}')
        if [[ -z $line ]]; then echo "warning: symbol not found in $vmlinux: $fn" >&2; continue; fi
        local start size end
        start=$(awk '{print $1}' <<<"$line")
        size=$(awk '{print $2}' <<<"$line")
        end=$(printf '0x%x\n' $(( 0x$start + 0x$size )))
        out_args+=",$fn=0x$start:$end"
    done
    printf '%s' "$out_args"
}

# ---- 5. run one kernel REPS times and assert determinism --------------------
measure() { # measure <label> <bzImage> <vmlinux>
    local label=$1 bz=$2 vmlinux=$3
    local ranges i counts=() json
    ranges=$(range_args "$vmlinux")
    for ((i = 1; i <= REPS; i++)); do
        timeout 1800 "$QEMU" -m 512 -smp 1 \
            -kernel "$bz" -initrd "$OUT/initramfs.cpio.gz" \
            -nographic -no-reboot -display none \
            -accel tcg,thread=single \
            -icount shift=0,align=off,sleep=off \
            -plugin "$plugin${ranges}",out="$OUT/$label.$i.json" \
            -append "console=ttyS0,115200 nokaslr panic=-1" \
            </dev/null > "$OUT/$label.$i.log" 2>&1 || true
        json="$OUT/$label.$i.json"
        [[ -f $json ]] || { echo "  run $i: NO RESULT"; return 1; }
        counts+=( "$(awk -F'[:,]' '/total_insns/{gsub(/[ "]/,"",$2); print $2}' "$json")" )
    done
    # Determinism is mandatory: a varying count is not a measurement.
    local c first=${counts[0]}
    for c in "${counts[@]}"; do
        if [[ $c != "$first" ]]; then
            printf '  %s: NON-DETERMINISTIC across %d reps: %s\n' "$label" "$REPS" "${counts[*]}" >&2
            return 2
        fi
    done
    printf '%s' "$first"
}

echo "in-vm codegen benchmark (deterministic instruction counts, reps=$REPS)"
printf '  A: %s\n' "$A"
printf '  B: %s\n' "$B"

A_INSNS=$(measure A "$A" "$A_VMLINUX") || die "measurement A failed"
B_INSNS=$(measure B "$B" "$B_VMLINUX") || die "measurement B failed"

echo
printf '  A instructions: %s\n' "$A_INSNS"
printf '  B instructions: %s\n' "$B_INSNS"
python3 - "$A_INSNS" "$B_INSNS" <<'PY'
import sys
a, b = int(sys.argv[1]), int(sys.argv[2])
d = b - a
pct = 100.0 * d / b if b else 0.0
print(f"  delta (A-B): {d:+d}  ({pct:+.3f} % vs B)")
print(f"  A/B ratio  : {a/b:.6f}" if b else "  A/B ratio  : n/a")
PY
echo
echo "artifacts: $OUT"
