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
#   * lld is pinned to the release/23.x branch (matching the Clang 23.1
#     codegen oracle channel) and built with LLVM_TARGETS_TO_BUILD=X86 only —
#     the same restrict-the-targets trick as mold, applied to LLVM.
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

# Version pins.  Default stays git HEAD (see the policy note above: stale
# oracles have produced false lccc failures).  Set these to a tag/SHA when a
# specific upstream release must be compared against -- e.g.
#     MOLD_REF=2.42.1 WILD_REF=main tests/linker/setup_oracles.sh
# A pinned checkout is cloned into its own directory so a HEAD build and a
# pinned build can coexist, and the installed binary is suffixed with the ref.
# `--only mold,wild` builds a subset (comma-separated: mold wild bfd lld).
# Default is all four.  Useful on a 2-core box where the bfd/lld source builds
# cost far more than the comparison being run needs.
ONLY=${ONLY:-mold,wild,bfd,lld}
want() { [[ ",$ONLY," == *",$1,"* ]]; }

MOLD_REF=${MOLD_REF:-HEAD}
WILD_REF=${WILD_REF:-HEAD}
ref_suffix() { [[ $1 == HEAD ]] && echo "" || echo "-$(echo "$1" | tr '/.' '__')"; }
# Check out a tag/branch/SHA, tolerating the `v` prefix that upstream uses on
# its release tags (mold tags `v2.42.1`, not `2.42.1`).
checkout_ref() {
  local dir=$1 ref=$2
  if [[ $ref == HEAD ]]; then git -C "$dir" reset -q --hard origin/main; return; fi
  git -C "$dir" checkout -q "refs/tags/$ref" 2>/dev/null && return
  git -C "$dir" checkout -q "refs/tags/v$ref" 2>/dev/null && return
  git -C "$dir" checkout -q "$ref" 2>/dev/null && return
  echo "fatal: ref '$ref' not found in $dir" >&2; exit 1
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --force)  FORCE=1; shift ;;
    --only)   ONLY=$2; shift 2 ;;
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
MOLD_NAME="mold$(ref_suffix "$MOLD_REF")"
if ! want mold; then
  :
elif have "$MOLD_NAME"; then
  log "mold already present: $("$BIN/$MOLD_NAME" --version | head -1)"
else
  log "building mold ${MOLD_REF} (targets: X86_64;I386)"
  command -v cmake >/dev/null || { echo "cmake is required" >&2; exit 1; }
  MOLD_DIR="$SRC/mold-src$(ref_suffix "$MOLD_REF")"
  if [[ ! -d "$MOLD_DIR/.git" ]]; then
    rm -rf "$MOLD_DIR"
    git clone --filter=blob:none https://github.com/rui314/mold.git "$MOLD_DIR"
  fi
  git -C "$MOLD_DIR" fetch --filter=blob:none origin --tags >/dev/null 2>&1 || true
  checkout_ref "$MOLD_DIR" "$MOLD_REF"
  log "mold source: $(git -C "$MOLD_DIR" describe --tags --always) $(git -C "$MOLD_DIR" rev-parse --short HEAD)"
  cmake -S "$MOLD_DIR" -B "$MOLD_DIR/build" \
        -DCMAKE_BUILD_TYPE=Release \
        -DMOLD_TARGETS='X86_64;I386' \
        -DMOLD_USE_MIMALLOC=OFF \
        -DMOLD_LTO=OFF \
        -DCMAKE_C_FLAGS="-O2 $NATIVE" \
        -DCMAKE_CXX_FLAGS="-O2 $NATIVE" \
        -DCMAKE_INSTALL_PREFIX="$MOLD_DIR/inst"
  cmake --build "$MOLD_DIR/build" -j "$JOBS"
  cmake --install "$MOLD_DIR/build"
  install -m755 "$MOLD_DIR/inst/bin/mold" "$BIN/$MOLD_NAME"
  ln -sf "$MOLD_NAME" "$BIN/ld$(ref_suffix "$MOLD_REF").mold"
  log "mold: $("$BIN/$MOLD_NAME" --version | head -1)"
fi

# ---------------------------------------------------------------------------
# wild
# ---------------------------------------------------------------------------
WILD_NAME="wild$(ref_suffix "$WILD_REF")"
if ! want wild; then
  :
elif have "$WILD_NAME"; then
  log "wild already present: $("$BIN/$WILD_NAME" --version | head -1)"
else
  log "building wild ${WILD_REF} (-C target-cpu=native)"
  command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
  WILD_DIR="$SRC/wild-src$(ref_suffix "$WILD_REF")"
  if [[ ! -d "$WILD_DIR/.git" ]]; then
    rm -rf "$WILD_DIR"
    git clone --filter=blob:none https://github.com/davidlattimore/wild.git "$WILD_DIR"
  fi
  git -C "$WILD_DIR" fetch --filter=blob:none origin --tags >/dev/null 2>&1 || true
  checkout_ref "$WILD_DIR" "$WILD_REF"
  log "wild source: $(git -C "$WILD_DIR" describe --tags --always) $(git -C "$WILD_DIR" rev-parse --short HEAD)"
  # The binary lives in the `wild-linker` package; building the whole
  # workspace also builds linker-diff and the benchmark runner, which we
  # never invoke and which roughly doubles the build.
  ( cd "$WILD_DIR" && \
    RUSTFLAGS="-C target-cpu=native" \
    cargo build --release -j "$JOBS" -p wild-linker --bin wild )
  install -m755 "$WILD_DIR/target/release/wild" "$BIN/$WILD_NAME"
  log "wild: $("$BIN/$WILD_NAME" --version | head -1)"
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
if ! want bfd; then
  :
elif have ld.bfd-2.47; then
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

# ---------------------------------------------------------------------------
# lld 23.1 (pinned major) — matches the Clang 23.1 codegen oracle channel
# ---------------------------------------------------------------------------
# Built with the same "restrict the targets" trick as mold: lld itself is
# target-generic, but it links against LLVM libraries that instantiate every
# backend.  Restricting LLVM_TARGETS_TO_BUILD to X86 (the only target the
# local linker comparisons run on) cuts the build from ~1 h to ~15 min on a
# 2-core box.  A system lld whose major version matches the pin is accepted
# as-is; anything else is built from the release/23.x branch so the oracle
# tracks the same LLVM major as the pinned Compiler Explorer clang.
LLD_MAJOR=23
# `lld --version` output varies by distro ("lld version 23.1.0",
# "Ubuntu lld version 14.0.6", "LLD 23.1.0"); match the major anywhere.
if ! want lld; then
  :
elif have lld; then
  log "lld already present: $("$BIN/lld" --version | head -1)"
elif command -v lld >/dev/null && lld --version 2>/dev/null | grep -Eq "(lld|LLD)[^-]*${LLD_MAJOR}\.[0-9]"; then
  install -m755 "$(command -v lld)" "$BIN/lld"
  ln -sf lld "$BIN/ld.lld"
  log "lld (system): $(lld --version | head -1)"
else
  log "building lld from LLVM release/${LLD_MAJOR}.x (X86 backend only)"
  command -v cmake >/dev/null || { echo "cmake is required" >&2; exit 1; }
  command -v ninja >/dev/null || { echo "ninja is required for the lld oracle build" >&2; exit 1; }
  if [[ ! -d "$SRC/llvm-src/.git" ]]; then
    rm -rf "$SRC/llvm-src"
    git clone --depth 1 --branch "release/${LLD_MAJOR}.x" \
      https://github.com/llvm/llvm-project.git "$SRC/llvm-src"
  else
    git -C "$SRC/llvm-src" fetch --depth 1 origin "release/${LLD_MAJOR}.x" && \
    git -C "$SRC/llvm-src" reset --hard FETCH_HEAD
  fi
  cmake -S "$SRC/llvm-src/lld" -B "$SRC/llvm-src/lld/build" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DLLVM_TARGETS_TO_BUILD=X86 \
        -DLLVM_ENABLE_PROJECTS=lld \
        -DLLVM_INCLUDE_TESTS=OFF \
        -DLLVM_INCLUDE_BENCHMARKS=OFF \
        -DLLVM_INCLUDE_EXAMPLES=OFF \
        -DLLVM_ENABLE_ASSERTIONS=OFF \
        -DCMAKE_C_FLAGS="-O2 $NATIVE" \
        -DCMAKE_CXX_FLAGS="-O2 $NATIVE" \
        -DCMAKE_INSTALL_PREFIX="$SRC/llvm-inst"
  cmake --build "$SRC/llvm-src/lld/build" -j "$JOBS"
  cmake --install "$SRC/llvm-src/lld/build"
  install -m755 "$SRC/llvm-inst/bin/lld" "$BIN/lld"
  ln -sf lld "$BIN/ld.lld"
  log "lld: $(\"$BIN/lld\" --version | head -1)"
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
  if [[ -d "$SRC/llvm-src/.git" ]]; then
    echo "lld:   $(git -C "$SRC/llvm-src" rev-parse HEAD 2>/dev/null || echo unknown) (release/${LLD_MAJOR}.x, X86 backend only)"
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
