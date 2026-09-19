#!/usr/bin/env bash
# ============================================================================
# build.sh — compile lccc_icount.so, the deterministic TCG instruction counter.
#
# The research host exposes no hardware PMU (virtualised x86-64, no perf-event
# passthrough), and wall-clock timing under TCG is dominated by host jitter, so
# neither cycles nor seconds are usable code-quality signals here.
#
# TCG with `-icount` makes guest execution deterministic: identical inputs
# retire an identical instruction stream, run after run (verified: three runs
# of a fixed boot image all reported exactly 8,900,714 instructions, whereas
# the same image without `-icount` varied by ~0.1 %).  Counting those
# instructions therefore yields an exact, zero-variance metric that is directly
# proportional to generated-code quality for a fixed workload.
#
# The plugin is loaded with:
#     qemu-system-x86_64 -plugin ./lccc_icount.so,out=counts.json[,NAME=0xS:0xE]
# and must be paired with -icount to be deterministic.
#
# Note: QEMU *user*-mode (qemu-x86_64) in the reference distro build has no
# plugin support ("unknown option 'plugin'"); this is system-mode only.
#
# Requirements: gcc, glib-2.0 headers (libglib2.0-dev), curl.
# Usage: build.sh [output-dir]
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-$here}
qemu_ver=${QEMU_VERSION:-v10.0.11}
header="$here/qemu-plugin.h"

mkdir -p "$out"

# The QEMU plugin API header is versioned against the QEMU binary: a mismatch
# makes QEMU reject the .so at load time. Fetch the header matching the
# installed QEMU by default and allow an explicit override.
#
# It is deliberately NOT vendored: qemu-plugin.h is GPLv2+ while this tree is
# CC0 / MIT-OR-Apache-2.0-OR-BSD-2-Clause, and the header is only needed at
# build time. .gitignore keeps a fetched copy out of `git add -A`.
if [[ ! -f "$header" ]]; then
    if command -v qemu-system-x86_64 >/dev/null 2>&1; then
        installed=$(qemu-system-x86_64 --version | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
        [[ -n $installed ]] && qemu_ver="v$installed"
    fi
    printf 'fetching qemu-plugin.h for %s\n' "$qemu_ver"
    curl -sSfL -o "$header" \
        "https://raw.githubusercontent.com/qemu/qemu/${qemu_ver}/include/qemu/qemu-plugin.h" \
        || { printf 'error: cannot fetch qemu-plugin.h (set QEMU_VERSION or place it next to this script)\n' >&2; exit 1; }
fi

if ! pkg-config --exists glib-2.0; then
    printf 'error: glib-2.0 development headers are required (apt: libglib2.0-dev)\n' >&2
    exit 1
fi

gcc -shared -fPIC -O2 -Wall -Wextra \
    -I"$here" $(pkg-config --cflags glib-2.0) \
    -o "$out/lccc_icount.so" "$here/lccc_icount.c"

printf 'built: %s\n' "$out/lccc_icount.so"
