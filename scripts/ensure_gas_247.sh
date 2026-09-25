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
# ftp.gnu.org is NOT universally reachable from the sandbox (connection
# blackholed), so the tarball fetch walks a mirror chain and takes the
# first one that answers. The download cache and build tree are
# parametrised (GAS_DL_DIR / GAS_CACHE) so a persistent workspace can keep
# the tarball across wipes; the install prefix is arg 2.
#
# Usage: scripts/ensure_gas_247.sh [target-triple] [install-prefix]
#   target-triple defaults to riscv64-linux-gnu. The assembler is installed
#   as <prefix>/bin/as and its version is printed on success.
set -euo pipefail

target=${1:-riscv64-linux-gnu}
prefix=${2:-${HOME}/.cache/gas-2.47-${target}}
as="$prefix/bin/as"

if [[ -x "$as" ]]; then
    "$as" --version | head -1
    exit 0
fi

ver=2.47
dl_dir=${GAS_DL_DIR:-${HOME}/dl}
cache=${GAS_CACHE:-${HOME}/.cache}
tarball="$dl_dir/binutils-$ver.tar.xz"
mkdir -p "$dl_dir" "$cache"

# Mirror chain: ftp.gnu.org first (canonical), then well-known mirrors that
# answer from the sandbox network. --connect-timeout keeps a blackholed
# host from stalling the provision.
if [[ ! -f "$tarball" ]]; then
    fetched=""
    for base in \
        "https://ftp.gnu.org/gnu/binutils" \
        "https://mirrors.kernel.org/gnu/binutils" \
        "https://mirror.csclub.uwaterloo.ca/gnu/binutils"; do
        echo "fetch: trying $base/binutils-$ver.tar.xz" >&2
        if curl --connect-timeout 10 --max-time 300 -sSLo "$tarball.part" \
            "$base/binutils-$ver.tar.xz"; then
            mv -f "$tarball.part" "$tarball"
            fetched=1
            break
        fi
        rm -f "$tarball.part"
    done
    [[ -n "$fetched" ]] || { echo "FATAL: no mirror reachable for binutils-$ver" >&2; exit 1; }
fi

src="$cache/binutils-$ver"
build="$src-build-${target//-/_}"
[[ -d $src ]] || tar -xJf "$tarball" -C "$cache"
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
