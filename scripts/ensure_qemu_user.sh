#!/usr/bin/env bash
# ensure_qemu_user.sh — (re)provision the pinned QEMU user-mode emulators
# (qemu-aarch64, qemu-arm, qemu-i386, qemu-riscv64) after a harness wipe.
#
# Why this exists, and why the pin is 11.1.2
# ------------------------------------------
# The cross/i686 execution gates run foreign binaries through
# `qemu-<arch>` resolved BY NAME.  The repository's offline transport
# for them was the `harness-cache-3` tag: four binaries extracted from
# Ubuntu's qemu-user-static 1:8.2.2+ds-0ubuntu1.18 — a 2024 build, two
# QEMU major releases behind the latest public QEMU release (11.1.2,
# download.qemu.org).  A gateway emulator that old misjudges modern
# guest code and silently turns "the emulator is stale" into compiler
# verdicts, which is exactly the failure the oracle pins elsewhere in
# this tree exist to prevent.
#
# Provenance of the new pin
# -------------------------
# QEMU upstream ships source, not static user-mode binaries.  Debian's
# `qemu-user` package is "QEMU user mode emulation (static binaries)"
# (it replaced `qemu-user-static` in Debian's 1:9.1.0 packaging) and
# Debian sid carries the exact latest upstream version, 11.1.2+ds-3.
# This script pins that `.deb` BY CONTENT:
#
#   * SHA-256 as published in Debian sid's signed Packages index
#     (dists/sid/main/binary-amd64/Packages.xz, `qemu-user` stanza);
#   * SHA-1 of the same bytes, used ONLY as the snapshot.debian.org
#     content-addressed retrieval key when the live Debian pool has
#     pruned this (superseded) pool file.  The SHA-256 check applies
#     identically to bytes from either transport, so the fallback can
#     never smuggle a different package past the pin.
#
# Only the four emulators the gates name are installed out of the
# ~73 MB package.  A pre-existing install is reused ONLY when every
# binary's `--version` reports 11.1.2 — an 8.2.2-era install in the
# same prefix is replaced, never vouched for by its directory name,
# so "update the old ones correctly" is the fast path's job too.
#
# Usage: scripts/ensure_qemu_user.sh [install-prefix] [--print-bin]
#   install-prefix defaults to ${LCCC_QEMU_PREFIX:-$HOME/.cache/lccc/qemu-user-11.1.2};
#   binaries land at <prefix>/bin/qemu-<arch>.  The download cache is
#   ${QEMU_DL_DIR:-$HOME/.cache/dl}, the same channel ensure_gas_247.sh
#   and ensure_arm_a64_xml.sh use: a pre-placed .deb there must hash to
#   the same pin before it is adopted.
#
# scripts/ensure_qemu_user.sh --self-test
#   exercises the version probe against fake emulators (11.1.2 accepted,
#   8.2.2 / lookalikes / missing arch rejected) with no network.
set -euo pipefail

QEMU_VERSION=11.1.2
DEB_VERSION=11.1.2+ds-3
DEB_NAME="qemu-user_${DEB_VERSION}_amd64.deb"
DEB_SHA256=4bacc2bf647f1756c5f08e7f146161d3eeef9d9f53e5d6412d1b3daca0c93655
DEB_SHA1=6067b061f3dd361d94e6c48a1044f79860c56c51
DEB_URLS=(
    "https://deb.debian.org/debian/pool/main/q/qemu/$DEB_NAME"
    "https://snapshot.debian.org/file/$DEB_SHA1"
)
ARCHES=(aarch64 arm i386 riscv64)

print_bin=0
prefix=
self_test=0
for arg in "$@"; do
    case $arg in
        --print-bin) print_bin=1 ;;
        --self-test) self_test=1 ;;
        -h|--help) sed -n '2,48p' "$0"; exit 0 ;;
        --*) echo "ensure_qemu_user: unknown argument: $arg" >&2; exit 2 ;;
        *) [ -z "$prefix" ] || { echo "ensure_qemu_user: only one prefix may be given" >&2; exit 2; }
           prefix=$arg ;;
    esac
done
prefix=${prefix:-${LCCC_QEMU_PREFIX:-$HOME/.cache/lccc/qemu-user-$QEMU_VERSION}}
dl_dir=${QEMU_DL_DIR:-$HOME/.cache/dl}
deb_cache=$dl_dir/$DEB_NAME
bindir=$prefix/bin

_sha256_is() {  # _sha256_is <file> <expected-hex>: whole-digest equality
    local got
    got=$(sha256sum "$1" 2>/dev/null | awk '{print $1}') || return 1
    [[ $got == "$2" ]]
}

# _version_ok <bin-dir>: every required emulator exists and its first
# `--version` line carries the pinned version as a whole token.
# "qemu-i386 version 11.1.2 (Debian 1:11.1.2+ds-3)" is accepted;
# 8.2.2, 11.1.20 and a missing arch are not.
_version_ok() {
    local dir=$1 arch line
    for arch in "${ARCHES[@]}"; do
        [ -x "$dir/qemu-$arch" ] || return 1
        line=$("$dir/qemu-$arch" --version 2>/dev/null | head -n 1) || return 1
        case $line in
            *" version $QEMU_VERSION "*|*" version $QEMU_VERSION") ;;
            *) return 1 ;;
        esac
    done
}

run_self_test() {
    local td arch fake
    td=$(mktemp -d)
    # Expand now: at EXIT time the function's locals are out of scope
    # and `set -u` would abort the cleanup with an unbound variable.
    trap "rm -rf '$td'" EXIT
    mkdir -p "$td/good" "$td/old" "$td/lookalike" "$td/missing"
    _mk() {  # _mk <dir> <banner-version>
        local d=$1 v=$2 a
        for a in "${ARCHES[@]}"; do
            printf '#!/bin/sh\nprintf "qemu-%s version %s (fake)\\n" "%s" "%s"\n' "$a" "$v" "$a" "$v" > "$d/qemu-$a"
            chmod +x "$d/qemu-$a"
        done
    }
    _mk "$td/good" "$QEMU_VERSION"
    _mk "$td/old" "8.2.2"
    _mk "$td/lookalike" "11.1.20"
    _mk "$td/missing" "$QEMU_VERSION"; rm "$td/missing/qemu-arm"
    _version_ok "$td/good"      || { echo "self-test FAIL: 11.1.2 set rejected"; exit 1; }
    ! _version_ok "$td/old"      || { echo "self-test FAIL: 8.2.2 set accepted"; exit 1; }
    ! _version_ok "$td/lookalike" || { echo "self-test FAIL: 11.1.20 lookalike accepted"; exit 1; }
    ! _version_ok "$td/missing"  || { echo "self-test FAIL: incomplete set accepted"; exit 1; }
    echo "ensure_qemu_user self-test: PASS (accept $QEMU_VERSION; reject 8.2.2, 11.1.20, incomplete)"
}

if [ "$self_test" = 1 ]; then
    run_self_test
    exit 0
fi

# Fast path: a warm install whose every binary still reports the pin.
if _version_ok "$bindir"; then
    echo "qemu-user: $bindir present (QEMU $QEMU_VERSION, fast path)"
    [ "$print_bin" = 1 ] && echo "$bindir"
    exit 0
fi

for tool in curl sha256sum tar; do
    command -v "$tool" >/dev/null 2>&1 || { echo "ensure_qemu_user: $tool is required" >&2; exit 2; }
done
command -v dpkg-deb >/dev/null 2>&1 || command -v ar >/dev/null 2>&1 || {
    echo "ensure_qemu_user: dpkg-deb or ar is required to unpack the .deb" >&2; exit 2; }

# Download (or adopt a pre-placed .deb) and verify the pin before any
# extraction.  Walk the URL chain: the live pool carries the file
# today; snapshot.debian.org keeps it by content hash forever.
mkdir -p "$dl_dir"
if ! _sha256_is "$deb_cache" "$DEB_SHA256"; then
    fetched=0
    for url in "${DEB_URLS[@]}"; do
        echo "qemu-user: fetching $url" >&2
        if curl -fsSL --retry 3 --retry-delay 2 -o "$deb_cache.part" "$url" \
           && _sha256_is "$deb_cache.part" "$DEB_SHA256"; then
            mv "$deb_cache.part" "$deb_cache"
            fetched=1
            break
        fi
        rm -f "$deb_cache.part"
    done
    [ "$fetched" = 1 ] || {
        echo "qemu-user: FATAL: no transport delivered bytes matching the pinned SHA-256 ($DEB_SHA256)" >&2
        exit 1
    }
fi

# Extract into staging, validate, THEN publish: an interrupted run
# must not leave a half-set that the fast path could accept, and a
# stale previous install is replaced only by a verified new one.
staging=$prefix/.staging.$$
rm -rf "$staging"
mkdir -p "$staging/root" "$staging/bin"
trap 'rm -rf "$staging"' EXIT
if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb -x "$deb_cache" "$staging/root"
else
    ( cd "$staging/root" && ar x "$deb_cache" && tar -xf data.tar.* && rm -f data.tar.* control.tar.* debian-binary )
fi
for arch in "${ARCHES[@]}"; do
    [ -x "$staging/root/usr/bin/qemu-$arch" ] || {
        echo "qemu-user: FATAL: $DEB_NAME contains no usr/bin/qemu-$arch" >&2; exit 1; }
    install -m755 "$staging/root/usr/bin/qemu-$arch" "$staging/bin/qemu-$arch"
done
_version_ok "$staging/bin" || {
    echo "qemu-user: FATAL: extracted emulators do not all report $QEMU_VERSION" >&2; exit 1; }

mkdir -p "$prefix"
rm -rf "$bindir"
mv "$staging/bin" "$bindir"
printf '%s  %s\n' "$DEB_SHA256" "$DEB_NAME (Debian qemu-user $DEB_VERSION; QEMU $QEMU_VERSION)" \
    > "$prefix/.lccc-qemu-pin"
rm -rf "$staging"
trap - EXIT

_version_ok "$bindir" || { echo "qemu-user: FATAL: post-install version check failed" >&2; exit 1; }
echo "qemu-user: installed $bindir (QEMU $QEMU_VERSION, Debian qemu-user $DEB_VERSION)"
for arch in "${ARCHES[@]}"; do
    "$bindir/qemu-$arch" --version | head -n 1
done
if [ "$print_bin" = 1 ]; then echo "$bindir"; fi
exit 0
