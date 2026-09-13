#!/usr/bin/env bash
# Build the comparison linkers used by tests/linker/run_linker_tests.py.
#
# Policy, learned the hard way: mold and wild are built from git HEAD, never from
# a release tarball. Two false "lccc is broken" verdicts earlier in this series
# came from a tarball oracle that predated an upstream fix; an oracle you cannot
# date is not an oracle. lld is pinned to release/23.x because that is the branch
# the clang 23.1 driver on this machine expects.
#
# Everything is built -march=native: these are comparison tools for one machine,
# and a generic-baseline oracle can be slow enough that a differential sweep
# stops being practical.
#
# Idempotent -- re-running skips whatever is already on PATH.
set -euo pipefail

JOBS="$(nproc)"
PREFIX="${PREFIX:-$HOME/.local}"
SRC="${SRC:-$HOME/oracles/src}"
mkdir -p "$PREFIX/bin" "$SRC"

have() { command -v "$1" >/dev/null 2>&1; }

# ── GNU ld: the primary oracle, assumed installed ───────────────────────────
# Every verdict in this series that claims "matches GNU ld" was produced with
# this one, because it is the only oracle guaranteed to be present. The others
# below are corroborating, not required.
if have ld; then
    echo "ld    : $(ld --version | head -1)"
else
    echo "ld    : MISSING. Install binutils; it is the primary oracle." >&2
    exit 1
fi

# ── mold, from git HEAD ─────────────────────────────────────────────────────
if have mold; then
    echo "mold  : $(mold --version) (already on PATH, skipping)"
else
    [ -d "$SRC/mold" ] || git clone https://github.com/rui314/mold "$SRC/mold"
    (
        cd "$SRC/mold"
        git fetch origin main && git checkout main && git reset --hard origin/main
        echo "mold  : building $(git rev-parse --short HEAD)"
        cmake -B build -DCMAKE_BUILD_TYPE=Release \
              -DMOLD_TARGETS='X86_64;I386' \
              -DCMAKE_CXX_FLAGS='-march=native' \
              -DCMAKE_INSTALL_PREFIX="$PREFIX"
        cmake --build build -j "$JOBS"
        cmake --install build
    )
    echo "mold  : $("$PREFIX/bin/mold" --version)"
fi

# ── wild, from git HEAD (Rust) ──────────────────────────────────────────────
if have wild; then
    echo "wild  : $(wild --version 2>&1 | head -1) (already on PATH, skipping)"
else
    [ -d "$SRC/wild" ] || git clone https://github.com/davidlattimore/wild "$SRC/wild"
    (
        cd "$SRC/wild"
        git fetch origin main && git checkout main && git reset --hard origin/main
        echo "wild  : building $(git rev-parse --short HEAD)"
        RUSTFLAGS='-C target-cpu=native' cargo build --release
        install -m 0755 target/release/wild "$PREFIX/bin/wild"
    )
    echo "wild  : $("$PREFIX/bin/wild" --version 2>&1 | head -1)"
fi

# ── lld, pinned to release/23.x ─────────────────────────────────────────────
# X86 only: the tree's differential tests are x86-64/i386, and building every
# LLVM target on a 2-core box costs hours for nothing.
if have ld.lld; then
    echo "lld   : $(ld.lld --version) (already on PATH, skipping)"
else
    [ -d "$SRC/llvm-project" ] ||
        git clone --filter=blob:none https://github.com/llvm/llvm-project "$SRC/llvm-project"
    (
        cd "$SRC/llvm-project"
        git fetch origin release/23.x && git checkout release/23.x
        echo "lld   : building $(git rev-parse --short HEAD)"
        cmake -B build -S llvm -G Ninja -DCMAKE_BUILD_TYPE=Release \
              -DLLVM_ENABLE_PROJECTS=lld -DLLVM_TARGETS_TO_BUILD=X86 \
              -DCMAKE_C_FLAGS='-march=native' -DCMAKE_CXX_FLAGS='-march=native' \
              -DCMAKE_INSTALL_PREFIX="$PREFIX"
        ninja -C build -j "$JOBS" install
    )
    echo "lld   : $("$PREFIX/bin/ld.lld" --version)"
fi

echo
echo "Oracles installed under $PREFIX/bin. Put it on PATH and re-run:"
echo "  python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc-x86"
