#!/usr/bin/env bash
# ============================================================================
# Fetch and build the real-world workloads used by real_workloads.py.
#
# Why a script: the differential real-workload test needs *object files* from
# genuine projects (they exercise -ffunction-sections + --gc-sections, hidden
# visibility, weak aliases, constructors, archive member selection, COMDAT
# groups and thousands of symbols in combination — things synthetic fixtures
# never hit together).  Rebuilding them by hand each session is slow and
# error-prone, and an interrupted/partially-wiped autotools tree fails in
# confusing ways (`aclocal.m4` regeneration demanding autoconf, missing
# `lib/Makefile.in`, ...).  This script always produces a known-good state.
#
# Design notes:
#   * Sources are fetched to $PREFIX/<name> and built in-tree.
#   * `--clean` removes a damaged tree and starts over; this is the reliable
#     recovery path after the harness wipes file timestamps/permissions,
#     which makes autotools think it must re-run autoreconf.
#   * All builds use -ffunction-sections -fdata-sections so the linker's
#     section-GC and layout paths are actually exercised.
#   * Everything is skipped if the expected artefacts already exist, so the
#     script is cheap to re-run at the start of a session.
#
# Usage:
#   tests/linker/setup_workloads.sh [--clean] [--prefix DIR] [-j N]
# ============================================================================
set -euo pipefail

PREFIX=${LCCC_WORKLOADS:-/home/user/workloads}
JOBS=${JOBS:-2}
CLEAN=0
CFLAGS_COMMON="-O2 -ffunction-sections -fdata-sections"

while [[ $# -gt 0 ]]; do
  case $1 in
    --clean)  CLEAN=1; shift ;;
    --prefix) PREFIX=$2; shift 2 ;;
    -j)       JOBS=$2; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$PREFIX"
cd "$PREFIX"

log() { printf '\n=== %s\n' "$*"; }

fetch() { # fetch <url> <tarball> <dir>
  local url=$1 tar=$2 dir=$3
  if [[ $CLEAN -eq 1 ]]; then rm -rf "$dir"; fi
  if [[ ! -d "$dir" ]]; then
    [[ -f "$tar" ]] || curl -fsSL -o "$tar" "$url"
    tar xzf "$tar"
  fi
}

# ---------------------------------------------------------------------------
# zlib-ng — CMake, no autotools fragility
# ---------------------------------------------------------------------------
ZNG_DIR=zlib-ng-2.2.4
if [[ -f "$ZNG_DIR/build/libz.a" && $CLEAN -eq 0 ]]; then
  log "zlib-ng already built"
else
  log "building zlib-ng"
  fetch https://github.com/zlib-ng/zlib-ng/archive/refs/tags/2.2.4.tar.gz \
        zlib-ng.tar.gz "$ZNG_DIR"
  cmake -S "$ZNG_DIR" -B "$ZNG_DIR/build" \
        -DCMAKE_BUILD_TYPE=Release -DZLIB_COMPAT=ON -DWITH_GTEST=OFF \
        -DCMAKE_C_FLAGS="$CFLAGS_COMMON" >/dev/null
  cmake --build "$ZNG_DIR/build" -j "$JOBS" >/dev/null
  log "zlib-ng: $(ls -la "$ZNG_DIR"/build/libz.a | awk '{print $5" bytes"}')"
fi

# ---------------------------------------------------------------------------
# expat — autotools; a damaged tree is rebuilt from scratch rather than
# coaxed, because `make` in a tree with clobbered timestamps tries to run
# autoreconf and fails on a machine without autoconf/m4/perl.
# ---------------------------------------------------------------------------
EXPAT_DIR=expat-2.6.4
if [[ -f "$EXPAT_DIR/xmlwf/xmlwf-xmlwf.o" && -f "$EXPAT_DIR/lib/.libs/xmlparse.o" && $CLEAN -eq 0 ]]; then
  log "expat already built"
else
  log "building expat"
  rm -rf "$EXPAT_DIR"          # always from a pristine tree
  fetch https://github.com/libexpat/libexpat/releases/download/R_2_6_4/expat-2.6.4.tar.gz \
        expat.tar.gz "$EXPAT_DIR"
  ( cd "$EXPAT_DIR" && chmod +x configure && \
    ./configure --without-docbook --without-tests --without-examples \
                CFLAGS="$CFLAGS_COMMON" >/dev/null && \
    make -j "$JOBS" >/dev/null )
  log "expat: $(ls "$EXPAT_DIR"/xmlwf/*.o "$EXPAT_DIR"/lib/.libs/*.o 2>/dev/null | wc -l) objects"
fi

# ---------------------------------------------------------------------------
# gzip — autotools, same policy as expat
# ---------------------------------------------------------------------------
GZIP_DIR=gzip-1.13
if [[ -f "$GZIP_DIR/gzip.o" && -f "$GZIP_DIR/lib/libgzip.a" && $CLEAN -eq 0 ]]; then
  log "gzip already built"
else
  log "building gzip"
  rm -rf "$GZIP_DIR"
  fetch https://ftp.gnu.org/gnu/gzip/gzip-1.13.tar.gz gzip.tar.gz "$GZIP_DIR"
  ( cd "$GZIP_DIR" && chmod +x configure && \
    ./configure CFLAGS="$CFLAGS_COMMON" >/dev/null && \
    make -j "$JOBS" >/dev/null )
  log "gzip: $(ls "$GZIP_DIR"/*.o 2>/dev/null | wc -l) objects + lib/libgzip.a"
fi

cat <<EOF

Workloads ready in $PREFIX

    tests/linker/real_workloads.py --workloads $PREFIX

If a tree is damaged (the harness clobbers timestamps and permissions,
which makes autotools attempt an autoreconf), re-run with --clean.
EOF
