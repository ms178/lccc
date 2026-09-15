#!/usr/bin/env bash
# Build/restore the comparison linkers used by tests/linker/run_linker_tests.py
# and the differential/ICF tooling.
#
# Pinning policy (2026-09-15, user directive): the ONLY oracles are
#   * GNU ld from binutils 2.47   (release tarball, ftp.gnu.org)
#   * mold 2.42.1                 (release tarball, github.com/rui314/mold)
#   * wild at git HEAD            (clone of github.com/davidlattimore/wild)
# lld and the distro-default ld are NOT oracles: two false "lccc is broken"
# verdicts earlier in this series came from stale oracle revisions, and an
# oracle you cannot date is not an oracle. binutils/mold are pinned releases so
# every verdict is reproducible; wild stays at HEAD because the project ships no
# releases and we deliberately track its moving target (its REVISION file
# records the exact commit every build/restore came from).
#
# The harness wipes everything outside /home/user mid-session, so binaries live
# under ARTIFACTS (= /home/user/artifacts/oracles by default): that tree is
# part of the workspace snapshot and survives. Restoring = re-pointing symlinks;
# full source rebuilds happen only when the pinned binaries are missing or the
# checksums in ORACLES.md changed.
#
# Idempotent: re-running verifies and skips whatever already checks out.
set -euo pipefail

JOBS="${JOBS:-2}"                      # research policy: -j2
ARTIFACTS="${ARTIFACTS:-/home/user/artifacts/oracles}"
PREFIX="${PREFIX:-$ARTIFACTS}"         # install root == persistent root
SRC="${SRC:-$ARTIFACTS/src}"
LINKDIR="${LINKDIR:-/home/user/artifacts/bin}"

BINUTILS_VER=2.47
MOLD_VER=2.42.1
WILD_REPO=https://github.com/davidlattimore/wild.git

mkdir -p "$PREFIX" "$SRC" "$LINKDIR"

note() { printf '%s\n' "$*"; }

# ── GNU ld 2.47 (primary oracle) ────────────────────────────────────────────
if [ -x "$PREFIX/bfd-$BINUTILS_VER/bin/ld" ]; then
    note "ld    : $("$PREFIX/bfd-$BINUTILS_VER/bin/ld" --version | head -1) (restored from $ARTIFACTS)"
else
    TARBALL="$SRC/binutils-$BINUTILS_VER.tar.xz"
    [ -f "$TARBALL" ] || { note "ld    : fetching binutils $BINUTILS_VER"; \
        curl -fsSL -o "$TARBALL" "https://ftp.gnu.org/gnu/binutils/binutils-$BINUTILS_VER.tar.xz"; }
    BT="$SRC/binutils-$BINUTILS_VER"
    rm -rf "$BT"; tar -xf "$TARBALL" -C "$SRC"
    ( cd "$BT"
      ./configure --prefix="$PREFIX/bfd-$BINUTILS_VER" \
          --disable-nls --disable-gdb --disable-gdbserver --disable-sim \
          --disable-libquadmath --enable-64-bit-bfd --disable-werror \
          CFLAGS='-O2 -g0' CXXFLAGS='-O2 -g0'
      make -j"$JOBS" && make install-strip )
    note "ld    : $("$PREFIX/bfd-$BINUTILS_VER/bin/ld" --version | head -1) (built)"
fi

# ── mold 2.42.1, MOLD_USE_SYSTEM_* defaults (self-contained binary) ─────────
if [ -x "$PREFIX/mold-$MOLD_VER/bin/mold" ]; then
    note "mold  : $("$PREFIX/mold-$MOLD_VER/bin/mold" --version) (restored from $ARTIFACTS)"
else
    TARBALL="$SRC/mold-$MOLD_VER.tar.gz"
    [ -f "$TARBALL" ] || { note "mold  : fetching mold $MOLD_VER"; \
        curl -fsSL -o "$TARBALL" "https://github.com/rui314/mold/archive/refs/tags/v$MOLD_VER.tar.gz"; }
    MT="$SRC/mold-$MOLD_VER"
    rm -rf "$MT"; tar -xzf "$TARBALL" -C "$SRC"
    ( cd "$MT"
      cmake -B build -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
            -DCMAKE_CXX_FLAGS='-march=native' \
            -DMOLD_TARGETS='X86_64;I386' \
            -DCMAKE_INSTALL_PREFIX="$PREFIX/mold-$MOLD_VER"
      cmake --build build -j "$JOBS" && cmake --install build
      strip "build/mold" 2>/dev/null || true )
    note "mold  : $("$PREFIX/mold-$MOLD_VER/bin/mold" --version) (built)"
fi

# ── wild, git HEAD (Rust; revision stamped in REVISION) ─────────────────────
if [ -x "$PREFIX/wild-git/bin/wild" ] && [ -f "$PREFIX/wild-git/REVISION" ]; then
    note "wild  : $("$PREFIX/wild-git/bin/wild" --version 2>&1 | head -1) (restored from $ARTIFACTS; rev $(cat "$PREFIX/wild-git/REVISION"))"
else
    WT="$SRC/wild"
    rm -rf "$WT"; git clone --depth 1 "$WILD_REPO" "$WT"
    ( cd "$WT"
      REV="$(git rev-parse --short HEAD)"
      note "wild  : building HEAD = $REV"
      RUSTFLAGS='-C target-cpu=native' cargo build --release --locked -j "$JOBS"
      mkdir -p "$PREFIX/wild-git/bin" && install -s target/release/wild "$PREFIX/wild-git/bin/wild"
      echo "$REV" > "$PREFIX/wild-git/REVISION" )
    note "wild  : $("$PREFIX/wild-git/bin/wild" --version 2>&1 | head -1) (built)"
fi

# ── convenience wrappers (stable names on PATH) ─────────────────────────────
for pair in "ld-2.47:$PREFIX/bfd-$BINUTILS_VER/bin/ld" \
            "mold-2.42.1:$PREFIX/mold-$MOLD_VER/bin/mold" \
            "wild:$PREFIX/wild-git/bin/wild"; do
    name="${pair%%:*}"; target="${pair#*:}"
    printf '#!/bin/sh\nexec %s "$@"\n' "$target" > "$LINKDIR/$name"
    chmod +x "$LINKDIR/$name"
done

echo
note "Oracles under $PREFIX (wipe-safe); wrappers in $LINKDIR."
note "Record new checksums in $PREFIX/ORACLES.md whenever a pin moves."
