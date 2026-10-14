#!/usr/bin/env bash
# Provision a temporary swap file on a Linux CI runner before memory-heavy jobs.
# The runner VM is ephemeral, so the active swap file is discarded with the job.
set -euo pipefail

if [[ $(uname -s) != Linux ]]; then
    echo "ci_enable_swap: Linux is required" >&2
    exit 2
fi

SWAP_SIZE=${LCCC_CI_SWAP_SIZE:-4G}
SWAP_DIR=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
SWAP_FILE="$SWAP_DIR/lccc-ci.swap"

if [[ ! -d $SWAP_DIR || ! -w $SWAP_DIR ]]; then
    echo "ci_enable_swap: swap directory is not writable: $SWAP_DIR" >&2
    exit 1
fi

if (( EUID == 0 )); then
    as_root() { "$@"; }
else
    command -v sudo >/dev/null 2>&1 || {
        echo "ci_enable_swap: sudo is required to activate swap" >&2
        exit 1
    }
    as_root() { sudo -- "$@"; }
fi

command -v fallocate >/dev/null 2>&1 || {
    echo "ci_enable_swap: fallocate is required" >&2
    exit 1
}

# Be idempotent if a caller explicitly invokes the setup twice in one job.
if ! awk -v path="$SWAP_FILE" '$1 == path { found = 1 } END { exit !found }' /proc/swaps; then
    if [[ -e $SWAP_FILE ]]; then
        # Only remove this script's fixed file in the runner temp directory.
        as_root rm -f -- "$SWAP_FILE"
    fi
    as_root fallocate -l "$SWAP_SIZE" "$SWAP_FILE"
    as_root chown root:root "$SWAP_FILE"
    as_root chmod 600 "$SWAP_FILE"
    as_root mkswap "$SWAP_FILE"
    as_root swapon "$SWAP_FILE"
fi

if ! awk -v path="$SWAP_FILE" '$1 == path { found = 1 } END { exit !found }' /proc/swaps; then
    echo "ci_enable_swap: swap file is not active: $SWAP_FILE" >&2
    exit 1
fi

printf '%s\n' "Active CI swap file: $SWAP_FILE ($SWAP_SIZE)"
as_root swapon --show
if command -v free >/dev/null 2>&1; then
    free -h
fi
