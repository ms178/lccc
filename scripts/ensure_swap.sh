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
size=${LCCC_SWAP_SIZE:-4G}
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    privilege=()
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    privilege=(sudo -n)
else
    echo "error: no active swap and root/passwordless sudo is unavailable" >&2
    exit 1
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
"${privilege[@]}" /sbin/swapon "$swapfile"

if ! /sbin/swapon --show | grep -Fq "$swapfile"; then
    echo "error: failed to activate swap file $swapfile" >&2
    exit 1
fi
/sbin/swapon --show
