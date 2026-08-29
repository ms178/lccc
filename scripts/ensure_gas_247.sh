#!/usr/bin/env bash
# ensure_gas_247.sh — (re)provision GNU as 2.47 for a cross target after a
# harness wipe.
#
# The differential execution suites hard-gate on GNU as 2.47
# (latest-toolchains-only policy). Distro binutils is older, and the
# locally built assembler lives under the snapshot-excluded .cache/ tree,
# so it never survives a workspace restore. This script rebuilds it
# idempotently from the upstream tarball; only the assembler is configured
# and built (no ld/gold/gdb/sim), which keeps the build at a few minutes
# on the 2-vCPU sandbox.
#
# Usage: scripts/ensure_gas_247.sh [target-triple] [install-prefix]
#   target-triple defaults to riscv64-linux-gnu. The assembler is installed
#   as <prefix>/bin/as and its version is printed on success.
set -euo pipefail

target=${1:-riscv64-linux-gnu}
prefix=${2:-/home/user/.cache/gas-2.47-${target}}
as="$prefix/bin/as"

if [[ -x "$as" ]]; then
    "$as" --version | head -1
    exit 0
fi

ver=2.47
tarball=/home/user/dl/binutils-$ver.tar.xz
if [[ ! -f "$tarball" ]]; then
    mkdir -p "$(dirname "$tarball")"
    curl -sSLo "$tarball" "https://ftp.gnu.org/gnu/binutils/binutils-$ver.tar.xz"
fi

src=/home/user/.cache/binutils-$ver
build="$src-build-${target//-/_}"
mkdir -p /home/user/.cache
[[ -d $src ]] || tar -xJf "$tarball" -C /home/user/.cache
rm -rf "$build"
mkdir -p "$build"
cd "$build"
"$src/configure" --target="$target" --prefix="$prefix" \
    --disable-gdb --disable-sim --disable-gprofng --disable-nls \
    --disable-werror --disable-ld --disable-gold >configure.log 2>&1
make -j2 >make.log 2>&1
mkdir -p "$prefix/bin"
cp gas/as-new "$as"
"$as" --version | head -1
