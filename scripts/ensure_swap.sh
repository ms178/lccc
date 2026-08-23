#!/usr/bin/env bash
# Ensure the constrained build host has swap before Rust compilation.
#
# The Arena harness can recreate the root filesystem between turns while the
# repository persists, so /swapfile must be recreated when /proc/swaps is empty.
# Override the defaults with LCCC_SWAPFILE and LCCC_SWAP_SIZE (fallocate syntax).
set -euo pipefail

if [[ -r /proc/swaps ]] && [[ $(wc -l </proc/swaps) -gt 1 ]]; then
    /sbin/swapon --show
    exit 0
fi

swapfile=${LCCC_SWAPFILE:-/swapfile}
# 8G default: the full-kernel vmlinux link with lccc/lccc-ld and the Rust
# bootstrap exceed 4G of combined working set on a 2G VM; 8G gives headroom.
size=${LCCC_SWAP_SIZE:-8G}
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    privilege=()
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    privilege=(sudo -n)
else
    # Unprivileged sandbox (no root, no passwordless sudo, no CAP_SYS_ADMIN):
    # mkswap/swapon are impossible, so there is nothing this script can do.
    # A hard failure here would break every compiler build in such an
    # environment even though the build itself only needs swap as overflow
    # insurance. Warn loudly and let the caller decide (the build wrappers
    # run with -j2 precisely to stay inside the RAM budget without swap).
    echo "warning: no root privileges and no active swap; continuing without swap" >&2
    echo "warning: keep build parallelism at -j2 and monitor memory (OOM killer)" >&2
    exit 0
fi

if [[ ! -e "$swapfile" ]]; then
    if command -v fallocate >/dev/null 2>&1; then
        "${privilege[@]}" fallocate -l "$size" "$swapfile"
    else
        # Portable fallback. LCCC_SWAP_SIZE must be an integer GiB here.
        gib=${size%G}
        [[ $gib =~ ^[1-9][0-9]*$ ]] || {
            echo "error: dd fallback requires LCCC_SWAP_SIZE=<integer>G" >&2
            exit 1
        }
        "${privilege[@]}" dd if=/dev/zero of="$swapfile" bs=1M \
            count=$((gib * 1024)) status=progress
    fi
fi
"${privilege[@]}" chmod 600 "$swapfile"
# mkswap is idempotent for our disposable file and refreshes a partially
# created header after an interrupted harness turn.
"${privilege[@]}" /sbin/mkswap "$swapfile" >/dev/null
if ! "${privilege[@]}" /sbin/swapon "$swapfile" 2>/tmp/lccc-swapon.err; then
    # Some sandboxes (unprivileged containers, no CAP_SYS_ADMIN) refuse
    # swapon even as root. The 8G file is still useful as a tmp spill
    # target; do not fail the compiler build.
    echo "warning: swapon $swapfile failed (continuing without swap): $(tr '\n' ' ' </tmp/lccc-swapon.err)" >&2
    exit 0
fi

if ! /sbin/swapon --show | grep -Fq "$swapfile"; then
    echo "warning: swap file $swapfile was not activated; continuing without swap" >&2
    exit 0
fi

# Persist across VM reboots and keep the box biased toward reclaiming cold
# pages rather than the hot compiler working set.
if ! grep -q "^$swapfile" /etc/fstab 2>/dev/null; then
    echo "$swapfile none swap sw 0 0" >> /etc/fstab 2>/dev/null || true
fi
"${privilege[@]}" /sbin/sysctl -w vm.swappiness=20 >/dev/null 2>&1 || true

/sbin/swapon --show
