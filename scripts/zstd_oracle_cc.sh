#!/usr/bin/env bash
# ============================================================================
# zstd_oracle_cc.sh — one-command miscompile bisection for the preboot ZSTD
# decompressor.
#
# Compiles the userspace oracle TU from scripts/zstd_preboot_oracle.sh with an
# arbitrary set of compiler flags / environment overrides and runs it on the
# REAL piggy payload (arch/x86/boot/compressed/vmlinux.bin.zst). This is the
# harness that caught and then cleared the -O2 Tier-2 slot-packing miscompile
# (engineering/FOLLOWUP-2026-09-02-session10-kernel-boot-zstd-tier2.md §3):
# the synthetic 256 B…4 KiB pattern cases all passed while the real payload
# failed, so only the payload case is decisive.
#
# Prerequisites: run scripts/zstd_preboot_oracle.sh once (it leaves
# /tmp/zstd-oracle/{oracle.c,driver.c,cflags.rsp} behind, which this script
# reuses) and have a built kernel tree with vmlinux.bin.zst.
#
# Usage:
#   zstd_oracle_cc.sh "<label>" [VAR=val ...] [extra lccc flags...]
#
# Examples:
#   zstd_oracle_cc.sh "baseline -O2" FOO=1
#   zstd_oracle_cc.sh "no-gvn"        CCC_DISABLE_PASSES=gvn
#   zstd_oracle_cc.sh "tier2 off"     CCC_NO_TIER2_GRAPH=1
#   zstd_oracle_cc.sh "-O1"           -O1
#
# Output: "<label>  <piggy-args result>" (MATCH … or FAIL rc=… err=…)
# ============================================================================
set -uo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
OUT=${OUT:-/tmp/zo}
ZFILE=${ZFILE:-/home/user/kernel-work/linux-vm/arch/x86/boot/compressed/vmlinux.bin.zst}
PFILE=${PFILE:-/home/user/kernel-work/linux-vm/arch/x86/boot/compressed/vmlinux.bin}

mkdir -p "$OUT"
if [[ ! -f /tmp/zstd-oracle/oracle.c ]]; then
    echo "zstd_oracle_cc.sh: run scripts/zstd_preboot_oracle.sh first" >&2
    exit 2
fi

label="$1"; shift
# Split argv into VAR=val (environment) and the rest (compiler flags).
envs=(); flags=()
for a in "$@"; do
    if [[ "$a" == *=* && "$a" != -* ]]; then envs+=("$a"); else flags+=("$a"); fi
done

cp -f /tmp/zstd-oracle/oracle.c /tmp/zstd-oracle/driver.c "$OUT"/ 2>/dev/null
sed -i "s|DECOMPRESS_UNZSTD_PATH_PLACEHOLDER|$K/lib/decompress_unzstd.c|" "$OUT/oracle.c"
# The oracle.c left behind by zstd_preboot_oracle.sh may already carry a
# substituted absolute path; normalise it either way.
sed -i "s|#include \".*/lib/decompress_unzstd.c\"|#include \"$K/lib/decompress_unzstd.c\"|" "$OUT/oracle.c"

CFLAGS_COMMON="-m64 -O2 -std=gnu18 -fno-strict-aliasing -fPIE -fno-jump-tables -mcmodel=small -mno-red-zone -mno-mmx -mno-sse -ffreestanding -fno-stack-protector -fno-asynchronous-unwind-tables -fshort-wchar -Wno-pointer-sign -Wno-address-of-packed-member"

if ! env "${envs[@]}" $LCCC $CFLAGS_COMMON "${flags[@]}" @/tmp/zstd-oracle/cflags.rsp \
        -c "$OUT/oracle.c" -o "$OUT/oracle.o" 2>"$OUT/cc.err"; then
    echo "$label: CC FAIL"; head -5 "$OUT/cc.err"; exit 1
fi
gcc -O2 -fPIE -c "$OUT/driver.c" -o "$OUT/driver.o"
gcc -pie -o "$OUT/oracle" "$OUT/driver.o" "$OUT/oracle.o"
res=$("$OUT/oracle" "$ZFILE" "$PFILE" 2>&1 | grep -E "^piggy" | head -1)
printf '%-40s %s\n' "$label" "$res"
