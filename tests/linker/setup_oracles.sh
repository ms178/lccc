#!/usr/bin/env bash
# ============================================================================
# LCCC linker oracle provisioning
#
# Builds the reference linkers that `run_linker_tests.py`
# and `real_workloads.py` compare against.  Project policy, encoded here so it
# is honoured automatically instead of remembered:
#
#   * ALWAYS build mold and wild from git HEAD, never from a release tarball.
#     Stale oracles have already produced two *false* lccc failures
#     (wild 0.7.0 got constructor priority order and RELRO enforcement wrong;
#     lccc was right in both cases and wild-git agrees).  See
#     docs/linker/FOLLOWUP_2026-08-17_SESSION2.md §0.
#
#   * Build both with `-march=native`: the oracles are timing references, so
#     they must not be handicapped relative to lccc.
#
#   * Restrict mold to the targets we actually compare against:
#         -DMOLD_TARGETS='X86_64;I386'
#     mold instantiates its entire linker as a template over ~12 target types
#     (X86_64 I386 ARM64LE ARM64BE ARM32LE ARM32BE RV32LE RV32BE RV64LE
#     RV64BE PPC32 PPC64V1 PPC64V2 S390X SPARC64 M68K SH4LE LOONGARCH...),
#     and each one recompiles every .cc file.  Dropping the targets we never
#     test cuts the build from ~25 min to a few minutes on a 2-core box.
#
#   * Skip work we do not need: mimalloc off (we are not benchmarking mold's
#     allocator against itself), and only the `wild` binary from wild's
#     workspace (not linker-diff, benchmarks, integration tests).
#
# The script is idempotent and skips any oracle whose binary is already
# present and executable, so it is cheap to run at the start of a session.
#
# Usage:
#   tests/linker/setup_oracles.sh [--force] [--prefix DIR]
#
# Afterwards add "$PREFIX/bin" to PATH.
# ============================================================================
set -euo pipefail

PREFIX=${LCCC_ORACLE_PREFIX:-/home/user/tools}
FORCE=0
JOBS=${JOBS:-2}

while [[ $# -gt 0 ]]; do
  case $1 in
    --force)  FORCE=1; shift ;;
    --prefix) PREFIX=$2; shift 2 ;;
    -j)       JOBS=$2; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

BIN="$PREFIX/bin"
SRC="$PREFIX"
mkdir -p "$BIN"

have() { [[ -x "$BIN/$1" ]] && [[ $FORCE -eq 0 ]]; }

log() { printf '\n=== %s\n' "$*"; }

# --- native flags -----------------------------------------------------------
# -march=native is refused by some cross/QEMU environments; fall back quietly.
NATIVE="-march=native"
if ! echo 'int main(void){return 0;}' | cc -x c -march=native -o /dev/null - 2>/dev/null; then
  echo "note: -march=native unsupported by this compiler, falling back to -O2 only"
  NATIVE=""
fi

# ---------------------------------------------------------------------------
# mold
# ---------------------------------------------------------------------------
if have mold; then
  log "mold already present: $("$BIN/mold" --version | head -1)"
else
  log "building mold from git HEAD (targets: X86_64;I386)"
  command -v cmake >/dev/null || { echo "cmake is required" >&2; exit 1; }
  if [[ ! -d "$SRC/mold-src/.git" ]]; then
    rm -rf "$SRC/mold-src"
    git clone --depth 1 https://github.com/rui314/mold.git "$SRC/mold-src"
  else
    git -C "$SRC/mold-src" fetch --depth 1 origin && \
    git -C "$SRC/mold-src" reset --hard FETCH_HEAD
  fi
  cmake -S "$SRC/mold-src" -B "$SRC/mold-src/build" \
        -DCMAKE_BUILD_TYPE=Release \
        -DMOLD_TARGETS='X86_64;I386' \
        -DMOLD_USE_MIMALLOC=OFF \
        -DMOLD_LTO=OFF \
        -DCMAKE_C_FLAGS="-O2 $NATIVE" \
        -DCMAKE_CXX_FLAGS="-O2 $NATIVE" \
        -DCMAKE_INSTALL_PREFIX="$SRC/mold-inst"
  cmake --build "$SRC/mold-src/build" -j "$JOBS"
  cmake --install "$SRC/mold-src/build"
  install -m755 "$SRC/mold-inst/bin/mold" "$BIN/mold"
  ln -sf mold "$BIN/ld.mold"
  log "mold: $("$BIN/mold" --version | head -1)"
fi

# ---------------------------------------------------------------------------
# wild
# ---------------------------------------------------------------------------
if have wild; then
  log "wild already present: $("$BIN/wild" --version | head -1)"
else
  log "building wild from git HEAD (-C target-cpu=native)"
  command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
  if [[ ! -d "$SRC/wild-src/.git" ]]; then
    rm -rf "$SRC/wild-src"
    git clone --depth 1 https://github.com/davidlattimore/wild.git "$SRC/wild-src"
  else
    git -C "$SRC/wild-src" fetch --depth 1 origin && \
    git -C "$SRC/wild-src" reset --hard FETCH_HEAD
  fi
  # The binary lives in the `wild-linker` package; building the whole
  # workspace also builds linker-diff and the benchmark runner, which we
  # never invoke and which roughly doubles the build.
  ( cd "$SRC/wild-src" && \
    RUSTFLAGS="-C target-cpu=native" \
    cargo build --release -j "$JOBS" -p wild-linker --bin wild )
  install -m755 "$SRC/wild-src/target/release/wild" "$BIN/wild"
  log "wild: $("$BIN/wild" --version | head -1)"
fi

# ---------------------------------------------------------------------------
# bfd 2.47 (pinned) — the project's reference GAS/bfd version
# ---------------------------------------------------------------------------
# The codegen/oracle docs pin binutils 2.47 (scripts/README.md §"Oracles").
# The system bfd can be any distro version, so when it is NOT already 2.47 we
# build the pinned release from source (gas + bfd only — no gdb/gprof/plugins),
# which is what the differential tools compare against. This honours the
# "GAS / bfd 2.47" build preference instead of silently testing a stale oracle.
BINUTILS_VERSION=2.47
if have ld.bfd-2.47; then
  log "bfd 2.47 already present: $("$BIN/ld.bfd-2.47" --version | head -1)"
elif command -v ld.bfd >/dev/null && ld.bfd --version | grep -q "$BINUTILS_VERSION"; then
  install -m755 "$(command -v ld.bfd)" "$BIN/ld.bfd-2.47"
  install -m755 "$(command -v as)"  "$BIN/as-2.47"
  log "bfd 2.47 (system): $(ld.bfd --version | head -1)"
else
  log "building bfd 2.47 from source (gas + bfd only)"
  if [[ ! -f "$SRC/binutils-$BINUTILS_VERSION.tar.xz" ]]; then
    curl -fsSL -o "$SRC/binutils-$BINUTILS_VERSION.tar.xz" \
      "https://ftp.gnu.org/gnu/binutils/binutils-$BINUTILS_VERSION.tar.xz"
  fi
  rm -rf "$SRC/binutils-$BINUTILS_VERSION" "$SRC/bu-$BINUTILS_VERSION"
  tar -xf "$SRC/binutils-$BINUTILS_VERSION.tar.xz" -C "$SRC"
  mkdir -p "$SRC/bu-$BINUTILS_VERSION"
  ( cd "$SRC/bu-$BINUTILS_VERSION" && \
    "$SRC/binutils-$BINUTILS_VERSION/configure" --prefix="$SRC/bu-$BINUTILS_VERSION/prefix" \
      --disable-gdb --disable-gdbserver --disable-sim --disable-readline \
      --disable-libdecnumber --disable-nls --disable-werror \
      --disable-gprofng --disable-gprof --disable-plugins --with-system-zlib \
    && make -j "$JOBS" MAKEINFO=true all-gas all-binutils all-ld \
    && make MAKEINFO=true install-gas install-binutils install-ld )
  install -m755 "$SRC/bu-$BINUTILS_VERSION/prefix/bin/ld"  "$BIN/ld.bfd-2.47"
  install -m755 "$SRC/bu-$BINUTILS_VERSION/prefix/bin/as"  "$BIN/as-2.47"
  log "bfd 2.47: $("$BIN/ld.bfd-2.47" --version | head -1)"
fi

# Record the resolved oracle revisions so session docs can cite exact
# versions instead of an unreproducible "HEAD".  mold/wild are built from
# git HEAD by policy, but WHICH head must be auditable after the fact.
{
  echo "# LCCC linker oracle revisions (recorded $(date -u +%Y-%m-%dT%H:%M:%SZ))"
  if [[ -d "$SRC/mold-src/.git" ]]; then
    echo "mold:  $(git -C "$SRC/mold-src" rev-parse HEAD 2>/dev/null || echo unknown)"
  fi
  if [[ -d "$SRC/wild-src/.git" ]]; then
    echo "wild:  $(git -C "$SRC/wild-src" rev-parse HEAD 2>/dev/null || echo unknown)"
  fi
  echo "binutils (bfd/as reference): $BINUTILS_VERSION"
  echo "mold build targets: X86_64;I386"
} > "$BIN/ORACLE_REVISIONS.txt"

cat <<EOF

Oracles ready in $BIN
Add it to PATH:

    export PATH="$BIN:\$PATH"

Then:

    tests/linker/run_linker_tests.py
    tests/linker/real_workloads.py


Resolved revisions: $BIN/ORACLE_REVISIONS.txt
EOF
