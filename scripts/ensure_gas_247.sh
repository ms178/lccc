#!/usr/bin/env bash
# ensure_gas_247.sh — (re)provision the pinned binutils 2.47 oracle pair
# (GNU as + objdump) for a cross target after a harness wipe.
#
# The differential execution suites hard-gate on GNU as 2.47
# (latest-toolchains-only policy), and the encdiff corpus gate additionally
# pins the DISASSEMBLER: objdump decides BEATS/ok verdicts through
# decodes_same, so the runner image's objdump is as much an oracle as its
# `as' — whatever binutils the host ships is not a verdict authority. Both
# binaries come from the same 2.47 build, land under the same prefix, and
# are required TOGETHER: an as-only cache (a provision interrupted after
# the gas cp) must not short-circuit the gate into running with an
# unpinned objdump. Distro binutils is older, and the locally built tools
# live under the snapshot-excluded .cache/ tree, so they never survive a
# workspace restore. This script rebuilds them idempotently from the
# upstream tarball; only the binutils tools needed are configured and
# built (no ld/gold/gdb/sim), which keeps the build at a few minutes on
# the 2-vCPU sandbox.
#
# ftp.gnu.org is NOT universally reachable from the sandbox (connection
# blackholed), so the tarball fetch walks a mirror chain and takes the
# first one that answers. The download cache and build tree are
# parametrised (GAS_DL_DIR / GAS_CACHE) so a persistent workspace can keep
# the tarball across wipes; the install prefix is arg 2.
#
# Usage: scripts/ensure_gas_247.sh [target-triple] [install-prefix]
#   target-triple defaults to riscv64-linux-gnu. The tools are installed
#   as <prefix>/bin/as and <prefix>/bin/objdump, and their versions are
#   printed on success.
set -euo pipefail

target=${1:-riscv64-linux-gnu}
prefix=${2:-${HOME}/.cache/gas-2.47-${target}}
as="$prefix/bin/as"
od="$prefix/bin/objdump"

# BOTH binaries, at the PINNED VERSION, or a full rebuild. Two guards,
# two failure modes: a cache carrying only one of the pair is an
# interrupted provision, and a pair under the 2.47-NAMED prefix that is
# not 2.47 is a lie — the prefix name is not the version. A stale pair
# (say a distro 2.46 copied in, or a prefix reused by a newer provision)
# would short-circuit the gate into arbitrating every differential
# against the wrong oracle release, so the fast path re-derives what it
# is about to trust: --version's first line of BOTH binaries must say
# 2.47. Anything else falls through to the rebuild, which reinstalls
# from one source tree and re-prints both versions on success.
if [[ -x "$as" && -x "$od" ]] \
   && "$as" --version 2>/dev/null | head -1 | grep -q '2\.47' \
   && "$od" --version 2>/dev/null | head -1 | grep -q '2\.47'; then
    "$as" --version | head -1
    "$od" --version | head -1
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
# A bare -d test is not enough: an interrupted extraction (harness wipe mid
# tar, full disk, ...) leaves a tree that passes -d but has no configure,
# which would wedge the gate until manual cleanup. Reuse the cached tree only
# when its configure script is intact; otherwise re-extract atomically
# (extract to a scratch dir, then rename into place) so no partially
# extracted tree is ever visible to the build.
if [[ ! -f "$src/configure" ]]; then
    rm -rf "$src" "$src.extracting"
    mkdir -p "$src.extracting"
    tar -xJf "$tarball" -C "$src.extracting"
    mv "$src.extracting/binutils-$ver" "$src"
    rm -rf "$src.extracting"
fi
rm -rf "$build"
mkdir -p "$build"
cd "$build"
"$src/configure" --target="$target" --prefix="$prefix" \
    --disable-gdb --disable-sim --disable-gprofng --disable-nls \
    --disable-werror --disable-ld --disable-gold >configure.log 2>&1
make -j2 >make.log 2>&1
mkdir -p "$prefix/bin"
cp gas/as-new "$as"
cp binutils/objdump "$od"
"$as" --version | head -1
"$od" --version | head -1
