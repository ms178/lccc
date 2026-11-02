#!/usr/bin/env bash
# Install the mold linker oracle: mold 3.0.0, EXACTLY. Hosted CI and
# developers without that release on PATH both use this script; it is
# idempotent and cache-friendly.
#
# Why 3.0.0, and why an upstream tarball instead of the tag cache
# ---------------------------------------------------------------
# mold 3.0.0 (2026-10-05) superseded the harness-cache-1 payload
# `mold-2.42.1-x86_64`.  3.0 is the Rust rewrite of mold: the upstream
# release notes state it is a drop-in replacement for 2.42.1 (same
# options, same targets, same output apart from listed bug fixes).
# The prebuilt 2.42.1 tag payload is therefore stale, and silently
# reusing it would compare lccc-ld against a linker that is no
# longer the latest public release.
#
# Source: the official mold release asset for Linux x86-64, pinned by
# the SHA-256 digest GitHub publishes for it
# (`gh api repos/rui314/mold/releases/tags/v3.0.0 --jq
#  '.assets[] | select(.name=="mold-3.0.0-x86_64-linux.tar.gz") | .digest'`).
# Building the oracle from source instead remains available in
# tests/linker/setup_oracles.sh (mold v3.0.0, x86_64+i386 targets
# only, via Cargo features -- mold 3.x removed the CMake build and
# its -DMOLD_TARGETS= option).
#
# Usage:
#   scripts/ci_install_mold_oracle.sh [--prefix DIR] [--print-bin]
#
# Exit status 0 means `$PREFIX/bin/mold --version` reports 3.0.0 AND
# the binary linked a trivial program through a C driver (when a
# driver is available).  Under GitHub Actions the script also exports
# MOLD=<that binary> through $GITHUB_ENV.
set -Eeuo pipefail

VERSION=3.0.0
ASSET="mold-$VERSION-x86_64-linux.tar.gz"
URL="https://github.com/rui314/mold/releases/download/v$VERSION/$ASSET"
SHA256=6c90d4a474c7c0409dfb575be03a5345878ac14fdba18de8b40fa58c60121189

prefix=${LCCC_MOLD_ORACLE_PREFIX:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc/mold-oracle/$VERSION}
print_bin=0
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        -h|--help) sed -n '2,34p' "$0"; exit 0 ;;
        *) echo "ci_install_mold_oracle: unknown argument: $1" >&2; exit 2 ;;
    esac
done
[[ $prefix = /* && $prefix != / ]] || { echo "ci_install_mold_oracle: prefix must be an absolute private directory" >&2; exit 2; }

log() { printf '[mold-oracle] %s\n' "$*" >&2; }

errlog=
annotate() {
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        local msg=$*
        msg=${msg//'%'/%25}; msg=${msg//$'\r'/%0D}; msg=${msg//$'\n'/%0A}
        printf '::error title=mold %s oracle::%s\n' "$VERSION" "$msg"
    fi
}
fail() {
    log "FAIL: $*"
    annotate "$*"
    exit 1
}
on_err() {
    local status=$1 line=$2 cmd=$3 detail=
    [ -n "$errlog" ] && [ -s "$errlog" ] && detail=$'\n'"$(tail -n 20 "$errlog")"
    log "FAIL: line $line: \`$cmd' exited $status$detail"
    annotate "line $line: $cmd exited $status$detail"
    exit "$status"
}
trap 'on_err $? $LINENO "$BASH_COMMAND"' ERR

installed_version() {
    [ -x "$prefix/bin/mold" ] || return 1
    "$prefix/bin/mold" --version 2>/dev/null | head -n 1
}

finish() {
    local line
    line=$(installed_version || true)
    case $line in
        "mold $VERSION "*) ;;
        *) fail "$prefix/bin/mold reports '${line:-nothing}', expected mold $VERSION" ;;
    esac
    [ -e "$prefix/bin/ld.mold" ] || fail "$prefix/bin/ld.mold missing (-fuse-ld=mold cannot resolve)"
    # A version string is not the contract gates need: prove the driver
    # can LINK through this mold (catches a prefix missing its libexec
    # sibling).  Skipped only when no C driver exists at all, and said
    # out loud -- never silently.
    local driver= probe rc=0
    for cand in "${CC:-}" gcc clang cc; do
        [ -n "$cand" ] || continue
        if command -v "$cand" >/dev/null 2>&1; then driver=$cand; break; fi
    done
    if [ -n "$driver" ]; then
        probe=$(mktemp -d "${TMPDIR:-/tmp}/mold-oracle.XXXXXX")
        printf 'int main(void){return 42;}\n' > "$probe/t.c"
        if ! "$driver" -B"$prefix/bin" -fuse-ld=mold "$probe/t.c" -o "$probe/t" 2>/dev/null; then
            rm -rf "$probe"
            fail "$driver cannot link a trivial program through mold $VERSION"
        fi
        "$probe/t" && rc=0 || rc=$?
        rm -rf "$probe"
        [ "$rc" = 42 ] || fail "mold-linked probe exited $rc, expected 42"
    else
        log "note: no C driver available; link probe skipped (version probe only)"
    fi
    log "mold $VERSION oracle at $prefix/bin/mold"
    if [ -n "${GITHUB_ENV:-}" ]; then
        echo "MOLD=$prefix/bin/mold" >> "$GITHUB_ENV"
    fi
    if [ "$print_bin" = 1 ]; then
        echo "$prefix/bin/mold"
    fi
}

case $(installed_version || true) in
    "mold $VERSION "*) finish; exit ;;
esac
# A pre-existing prefix that is not this exact mold is never overlaid:
# it may be a foreign install.  An empty or absent prefix is ours.
if [ -d "$prefix" ] && [ -n "$(find "$prefix" -mindepth 1 -print -quit 2>/dev/null)" ]; then
    fail "unverified nonempty prefix $prefix is not mold $VERSION; choose a fresh prefix"
fi

for tool in curl tar sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
done

work=$(mktemp -d "${TMPDIR:-/tmp}/mold-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
archive=$work/$ASSET
errlog=$work/tools.err
: > "$errlog"

log "downloading $URL"
curl --fail --location --silent --show-error --retry 5 --retry-delay 3 \
    --retry-all-errors --output "$archive" "$URL" 2>>"$errlog"
log "verifying SHA-256 of $(stat -c %s "$archive") bytes"
echo "$SHA256  $archive" | sha256sum --check --quiet - 2>>"$errlog"
log "extracting into $prefix"
mkdir -p "$prefix"
tar -xzf "$archive" -C "$prefix" --strip-components=1 2>>"$errlog"
finish
