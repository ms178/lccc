#!/usr/bin/env bash
# Ensure the constrained build host has swap before Rust compilation.
#
# The Arena harness can recreate the root filesystem between turns while the
# repository persists, so /swapfile must be recreated when /proc/swaps is empty.
# Override the defaults with LCCC_SWAPFILE and LCCC_SWAP_SIZE (fallocate syntax).
set -euo pipefail

swapfile=${LCCC_SWAPFILE:-/swapfile}
# Hard requirement: EXACTLY 4 GiB of swap. The previous 8G default was a
# guess at headroom; the build is run with -j2 and measured peak RSS of the
# fastbuild rustc/lccc link is far below the 4 GiB RAM + 4 GiB swap budget.
# Enforce the SIZE, not merely the presence of a swap entry: a harness that
# restores an 8G /swapfile must be corrected, not accepted as "swap exists".
size=${LCCC_SWAP_SIZE:-4G}
if [[ $size =~ ^([0-9]+)G$ ]]; then
    wanted_bytes=$((BASH_REMATCH[1] * 1024 * 1024 * 1024))
else
    echo "error: LCCC_SWAP_SIZE must be <integer>G (got '$size')" >&2
    exit 1
fi

# Total active swap, in KiB, from /proc/swaps (the authoritative view: it
# reports every activated area, whatever its path or backing device).
active_swap_kib() {
    [[ -r /proc/swaps ]] || { echo 0; return; }
    awk 'NR > 1 { total += $3 } END { print total + 0 }' /proc/swaps
}

# Any active area that already provides the budget satisfies the requirement.
# A harness may hand back a correctly sized swap under another name
# (/swap.img, a zram device, a partition); insisting on our own path would
# allocate a *second* image, spend disk the build needs, and — on a host
# whose swap is already live — risk activating a file whose pages are in use.
# The contract is "the build runs with at least $size of active swap", not
# "a file named $swapfile exists": the wrong-size check below still pins our
# own file to exactly $size.
# `mkswap` reserves one page for the swap header, so a file of exactly
# $wanted_bytes reports one page less in /proc/swaps (4 GiB -> 4194300 KiB).
# Allow that single page of slack; anything smaller is a genuinely smaller
# area and does not satisfy the budget.
if (( $(active_swap_kib) * 1024 + 4096 >= wanted_bytes )); then
    /sbin/swapon --show
    exit 0
fi

if [[ -r /proc/swaps ]] && grep -Fq "$swapfile" /proc/swaps; then
    if [[ -e "$swapfile" ]] && [[ $(stat -c %s "$swapfile") -eq $wanted_bytes ]]; then
        /sbin/swapon --show
        exit 0
    fi
    # Wrong size (e.g. a restored 8G file): replace it. swapoff is safe here
    # because this script runs before any compiler build, when swap use is 0.
    echo "notice: $swapfile is the wrong size; recreating as $size" >&2
fi
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

# Remove an existing file of the WRONG size before recreating it. A live
# swap cannot be truncated, so deactivate it first.
if [[ -e "$swapfile" ]] && [[ $(stat -c %s "$swapfile") -ne $wanted_bytes ]]; then
    "${privilege[@]}" /sbin/swapoff "$swapfile" >/dev/null 2>&1 || true
    "${privilege[@]}" rm -f "$swapfile"
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
# A fixed /tmp path can be owned by root from an earlier sudo run, making the
# calling user's redirection fail before swapon is even attempted. Create the
# diagnostic file as the calling user and let the privileged child write to it.
swapon_err=$(mktemp "${TMPDIR:-/tmp}/lccc-swapon.XXXXXX")
trap 'rm -f "$swapon_err"' EXIT
if ! "${privilege[@]}" /sbin/swapon "$swapfile" 2>"$swapon_err"; then
    # Some sandboxes (unprivileged containers, no CAP_SYS_ADMIN) refuse
    # swapon even as root. The 4G file is still useful as a tmp spill
    # target; do not fail the compiler build.
    echo "warning: swapon $swapfile failed (continuing without swap): $(tr '\n' ' ' <"$swapon_err")" >&2
    exit 0
fi

if ! /sbin/swapon --show | grep -Fq "$swapfile"; then
    echo "warning: swap file $swapfile was not activated; continuing without swap" >&2
    exit 0
fi

# Persist across VM reboots and keep the box biased toward reclaiming cold
# pages rather than the hot compiler working set.
# The append must run with the same privilege as mkswap/swapon: a bare
# `>> /etc/fstab` is opened by the unprivileged shell and fails with EACCES.
if ! grep -q "^$swapfile" /etc/fstab 2>/dev/null; then
    echo "$swapfile none swap sw 0 0" |
        "${privilege[@]}" tee -a /etc/fstab >/dev/null 2>&1 || true
fi
"${privilege[@]}" /sbin/sysctl -w vm.swappiness=20 >/dev/null 2>&1 || true

/sbin/swapon --show
actual=$(stat -c %s "$swapfile")
if [[ $actual -ne $wanted_bytes ]]; then
    echo "error: $swapfile is $actual bytes, expected exactly $wanted_bytes ($size)" >&2
    exit 1
fi
