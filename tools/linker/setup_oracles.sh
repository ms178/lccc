#!/usr/bin/env bash
# Build/restore the comparison linkers used by tests/linker/run_linker_tests.py
# and the differential/ICF tooling.
#
# Pinning policy (user directive, refreshed 2026-09-27): the oracles are
#   * GNU ld (bfd) from binutils 2.47  (release tarball, ftp.gnu.org)
#   * mold 2.42.1                      (release tarball, github.com/rui314/mold),
#                                      built with -DMOLD_TARGETS='X86_64;I386'
#   * LLVM lld 23.1.x                  (apt.llvm.org llvm-toolchain-<codename>-23;
#                                      the installed version is asserted to be
#                                      23.1.* and recorded in ORACLES.lock)
#   * wild at git HEAD                 (clone of github.com/davidlattimore/wild;
#                                      skip with WITH_WILD=0)
# The distro-default ld is NOT an oracle: two false "lccc is broken" verdicts
# earlier in this series came from stale oracle revisions, and an oracle you
# cannot date is not an oracle.
#
# $ARTIFACTS/ORACLES.lock is a GENERATED INVENTORY, not an input: every run
# rewrites it (atomically) with the exact `--version` banners of the oracles
# that run resolved and exposed, and nothing reads it back — the pins are the
# variables below.  It lists exactly the oracles that have wrappers in
# $LINKDIR after the run.
#
# WITH_WILD=0 contract: wild is not part of the oracle set for this run — no
# build, no `wild` wrapper (a stale one from an earlier WITH_WILD=1 run is
# removed) and no inventory entry.  A previously built binary stays in
# $ARTIFACTS so that a later WITH_WILD=1 run restores it without rebuilding.
# Wrappers whose target is missing are likewise removed, never left dangling.
#
# Build-time preset (2-vCPU / 2 GB host, -j2 research policy):
#   * mold: MOLD_TARGETS is mold's own CMake cache variable (CMakeLists.txt,
#     v2.42.1: `set(MOLD_TARGETS X86_64 I386 ARM64LE ... CACHE STRING ...)`);
#     every source file is instantiated once per listed target, so restricting
#     it to the two x86 ELF targets cuts the template instantiation work by
#     ~10x.  X86_64 must stay first (MOLD_FIRST_TARGET).
#   * binutils: only bfd + ld are configured/built (`all-ld`,
#     `install-strip-ld`) — gas/binutils/gprof/gdb are not linker oracles
#     (GNU as 2.47 has its own provisioner, scripts/ensure_gas_247.sh).
#   * lld: a source build of LLVM is hours on this host; the apt.llvm.org
#     release build of the same 23.1 branch is used instead.
#
# Storage layout (harness snapshot caps: ~128 MB / ~10k files):
#   * installed oracle binaries -> $ARTIFACTS (= /home/user/artifacts/oracles),
#     plus one .tar.xz per prefix in $ARTIFACTS/cache (restores drop
#     executables but keep plain files), so a restore is an extraction and
#     re-pointing wrappers, never a rebuild;
#   * tarballs + build trees     -> $SRC (= /home/user/.cache/lccc-oracle-src):
#     snapshot-EXCLUDED and deleted after a successful install — a binutils
#     build tree alone is several hundred MB and would push the whole
#     workspace snapshot over its cap;
#   * lld lives in /usr (apt) and is re-installed after a wipe (~30 s).
#
# Idempotent: re-running verifies and skips whatever already checks out.
set -euo pipefail

JOBS="${JOBS:-2}"                      # research policy: -j2
ARTIFACTS="${ARTIFACTS:-/home/user/artifacts/oracles}"
PREFIX="${PREFIX:-$ARTIFACTS}"         # install root == persistent root
SRC="${SRC:-/home/user/.cache/lccc-oracle-src}"
LINKDIR="${LINKDIR:-/home/user/artifacts/bin}"

BINUTILS_VER=2.47
MOLD_VER=2.42.1
MOLD_TARGETS='X86_64;I386'
LLD_MAJOR=23
LLD_MINOR_PIN=23.1
WITH_WILD="${WITH_WILD:-1}"
WILD_REPO=https://github.com/davidlattimore/wild.git
LOCK="$ARTIFACTS/ORACLES.lock"

mkdir -p "$PREFIX" "$SRC" "$LINKDIR"

note() { printf '%s\n' "$*"; }

# Fail in seconds, not after a 15-minute binutils build, when a tool a later
# step needs is missing (a fresh container has no cmake, so mold used to die
# with exit 127 only after bfd had been built).
need() {
    local t missing=()
    for t in "$@"; do command -v "$t" >/dev/null 2>&1 || missing+=("$t"); done
    [ ${#missing[@]} -eq 0 ] && return 0
    note "setup_oracles: missing required tool(s): ${missing[*]}"
    note "  e.g. sudo apt-get install -y build-essential cmake xz-utils curl git"
    exit 1
}

# Wipe-resilient install cache.  Harness restores keep an installed prefix's
# headers but drop its executables, so every wipe used to cost a full
# rebuild (bfd ~15 min, mold ~40 min at -j2).  Each freshly installed prefix
# is also archived as a plain, non-executable tarball under $CACHE, and a
# missing executable is re-extracted from it before anything is rebuilt.
CACHE="$ARTIFACTS/cache"
restore_cached() {  # <prefix dir> <executable path inside it>
    [ -x "$PREFIX/$1/$2" ] && return 0
    [ -f "$CACHE/$1.tar.xz" ] || return 1
    tar -xJf "$CACHE/$1.tar.xz" -C "$PREFIX" && [ -x "$PREFIX/$1/$2" ]
}
cache_prefix() {    # <prefix dir>
    mkdir -p "$CACHE"
    tar -cJf "$CACHE/$1.tar.xz.tmp" -C "$PREFIX" "$1"
    mv -f "$CACHE/$1.tar.xz.tmp" "$CACHE/$1.tar.xz"
}

need curl tar xz
restore_cached "bfd-$BINUTILS_VER" bin/ld || need make gcc g++
restore_cached "mold-$MOLD_VER" bin/mold || need cmake make g++
if [ "$WITH_WILD" = 1 ]; then
    # rustup installs cargo outside the default PATH.
    [ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
    restore_cached wild-git bin/wild || need git cargo
fi

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
      make -j"$JOBS" all-ld && make install-strip-ld )
    rm -rf "$BT"
    cache_prefix "bfd-$BINUTILS_VER"
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
            -DMOLD_TARGETS="$MOLD_TARGETS" \
            -DCMAKE_INSTALL_PREFIX="$PREFIX/mold-$MOLD_VER"
      cmake --build build -j "$JOBS" && cmake --install build
      strip "$PREFIX/mold-$MOLD_VER/bin/mold" 2>/dev/null || true )
    rm -rf "$MT"
    cache_prefix "mold-$MOLD_VER"
    note "mold  : $("$PREFIX/mold-$MOLD_VER/bin/mold" --version) (built, targets $MOLD_TARGETS)"
fi

# ── LLVM lld 23.1.x (apt.llvm.org release build) ───────────────────────────
LLD_BIN=/usr/bin/ld.lld-$LLD_MAJOR
if [ ! -x "$LLD_BIN" ]; then
    codename=$( . /etc/os-release && echo "${VERSION_CODENAME:-}" )
    [ -n "$codename" ] || { note "lld   : cannot determine distro codename"; exit 1; }
    note "lld   : installing lld-$LLD_MAJOR from apt.llvm.org ($codename)"
    sudo install -d -m 0755 /etc/apt/keyrings
    curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key \
        | sudo tee /etc/apt/keyrings/apt.llvm.org.asc >/dev/null
    echo "deb [signed-by=/etc/apt/keyrings/apt.llvm.org.asc] https://apt.llvm.org/$codename/ llvm-toolchain-$codename-$LLD_MAJOR main" \
        | sudo tee /etc/apt/sources.list.d/llvm-$LLD_MAJOR.list >/dev/null
    sudo apt-get update -qq -o Dir::Etc::sourcelist=sources.list.d/llvm-$LLD_MAJOR.list \
        -o Dir::Etc::sourceparts=- -o APT::Get::List-Cleanup=0 >/dev/null
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "lld-$LLD_MAJOR" >/dev/null
fi
lld_banner=$("$LLD_BIN" --version | head -1)
case "$lld_banner" in
    *" $LLD_MINOR_PIN."*) note "lld   : $lld_banner" ;;
    *) note "lld   : version pin violated (want $LLD_MINOR_PIN.*): $lld_banner"; exit 1 ;;
esac

# ── wild, git HEAD (Rust; revision stamped in REVISION) ─────────────────────
if [ "$WITH_WILD" != 1 ]; then
    note "wild  : skipped (WITH_WILD=0)"
elif [ -x "$PREFIX/wild-git/bin/wild" ] && [ -f "$PREFIX/wild-git/REVISION" ]; then
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
    rm -rf "$WT"
    cache_prefix wild-git
    note "wild  : $("$PREFIX/wild-git/bin/wild" --version 2>&1 | head -1) (built)"
fi

# ── convenience wrappers (stable names on PATH) ─────────────────────────────
WILD_BIN="$PREFIX/wild-git/bin/wild"
wild_enabled=0
[ "$WITH_WILD" = 1 ] && [ -x "$WILD_BIN" ] && wild_enabled=1
for pair in "ld-$BINUTILS_VER:$PREFIX/bfd-$BINUTILS_VER/bin/ld" \
            "mold-$MOLD_VER:$PREFIX/mold-$MOLD_VER/bin/mold" \
            "ld.lld-$LLD_MINOR_PIN:$LLD_BIN" \
            "wild:$WILD_BIN"; do
    name="${pair%%:*}"; target="${pair#*:}"
    if [ ! -x "$target" ] || { [ "$name" = wild ] && [ "$wild_enabled" != 1 ]; }; then
        rm -f "$LINKDIR/$name"
        continue
    fi
    printf '#!/bin/sh\nexec %s "$@"\n' "$target" > "$LINKDIR/$name.tmp"
    chmod +x "$LINKDIR/$name.tmp"
    mv -f "$LINKDIR/$name.tmp" "$LINKDIR/$name"
done

# ── ORACLES.lock: generated inventory of the oracles exposed above ─────────
{
    printf '# %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ) setup_oracles.sh"
    printf 'ld      %s\n' "$("$PREFIX/bfd-$BINUTILS_VER/bin/ld" --version | head -1)"
    printf 'mold    %s (MOLD_TARGETS=%s)\n' "$("$PREFIX/mold-$MOLD_VER/bin/mold" --version)" "$MOLD_TARGETS"
    printf 'ld.lld  %s\n' "$lld_banner"
    if [ "$wild_enabled" = 1 ]; then
        printf 'wild    %s (rev %s)\n' "$("$WILD_BIN" --version 2>&1 | head -1)" \
            "$(cat "$PREFIX/wild-git/REVISION" 2>/dev/null || echo unknown)"
    fi
} > "$LOCK.tmp" && mv -f "$LOCK.tmp" "$LOCK"

echo
note "Oracles under $PREFIX (wipe-safe); wrappers in $LINKDIR; banners in $LOCK."
