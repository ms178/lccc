#!/usr/bin/env bash
# Install the Clang oracle the ext_vector_type gates compare against:
# Clang 23.1.3, EXACTLY (tests/regression/lib/clang_oracle.sh states why the
# version is part of the contract). Hosted CI and developers without that
# release on PATH both use this script; it is idempotent and cache-friendly.
#
# Source: the official LLVM release asset for Linux x86-64, pinned by the
# SHA-256 digest GitHub publishes for it. Only the C front end is extracted
# (the driver, its resource headers, and the shared libraries the official
# build links against, when the archive ships them): ~1 % of the 5 GiB
# distribution, which keeps a cache entry small.
#
# Usage:
#   scripts/ci_install_clang_oracle.sh [--prefix DIR] [--print-bin]
#
# Exit status 0 means `$PREFIX/bin/clang -dumpversion` prints 23.1.3. Under
# GitHub Actions the script also exports CLANG=<that binary> through
# $GITHUB_ENV so every later step's gate resolves the oracle via $CLANG.
set -euo pipefail

VERSION=23.1.3
ASSET="LLVM-$VERSION-Linux-X64.tar.zst"
URL="https://github.com/llvm/llvm-project/releases/download/llvmorg-$VERSION/$ASSET"
# `gh api repos/llvm/llvm-project/releases/tags/llvmorg-23.1.3 --jq
#  '.assets[] | select(.name=="LLVM-23.1.3-Linux-X64.tar.zst") | .digest'`
SHA256=14d2f701eb68fb799001bdea6231048f3990690fa2f406d563555ff8f744daba

prefix=${LCCC_CLANG_ORACLE_PREFIX:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc/clang-oracle/$VERSION}
print_bin=0
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
        *) echo "ci_install_clang_oracle: unknown argument: $1" >&2; exit 2 ;;
    esac
done

log() { printf '[clang-oracle] %s\n' "$*" >&2; }

installed_version() {
    [ -x "$prefix/bin/clang" ] || return 1
    "$prefix/bin/clang" -dumpversion 2>/dev/null
}

finish() {
    local ver
    ver=$(installed_version || true)
    if [ "$ver" != "$VERSION" ]; then
        log "FAIL: $prefix/bin/clang reports '${ver:-nothing}', expected $VERSION"
        return 1
    fi
    # The driver must find its resource directory (stddef.h, immintrin.h,
    # ...) or every oracle compile fails; check once here, not in each gate.
    if ! printf '#include <stddef.h>\nsize_t s;\n' |
        "$prefix/bin/clang" -fsyntax-only -x c - >/dev/null 2>&1; then
        log "FAIL: $prefix/bin/clang cannot compile against its own resource headers"
        return 1
    fi
    log "Clang $VERSION oracle at $prefix/bin/clang"
    if [ -n "${GITHUB_ENV:-}" ]; then
        echo "CLANG=$prefix/bin/clang" >> "$GITHUB_ENV"
    fi
    if [ "$print_bin" = 1 ]; then
        echo "$prefix/bin/clang"
    fi
}

if [ "$(installed_version || true)" = "$VERSION" ]; then
    finish
    exit
fi

for tool in curl tar zstd sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || {
        log "FAIL: $tool is required"
        exit 1
    }
done

work=$(mktemp -d "${TMPDIR:-/tmp}/clang-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
archive=$work/$ASSET

log "downloading $URL"
curl --fail --location --silent --show-error --retry 5 --retry-delay 3 \
    --retry-all-errors --output "$archive" "$URL"
log "verifying SHA-256"
echo "$SHA256  $archive" | sha256sum --check --quiet -
log "extracting the C front end into $prefix"
rm -rf "$prefix"
mkdir -p "$prefix"
top="LLVM-$VERSION-Linux-X64"
# One listing pass selects the members; tar then extracts exactly that list.
listing=$work/members
tar --zstd -tf "$archive" > "$listing"
members=$(grep -E "^$top/(bin/clang(-23)?$|lib/clang/23/include(/|$))" "$listing" || true)
if ! grep -qx "$top/bin/clang-23" "$listing"; then
    log "FAIL: $ASSET does not contain $top/bin/clang-23"
    exit 1
fi
printf '%s\n' "$members" |
    tar --zstd -xf "$archive" -C "$prefix" --strip-components=1 --files-from=-
# The official release driver is linked statically against LLVM; if a build
# ever ships it against lib/libLLVM.so or libclang-cpp.so, fetch exactly the
# libraries the dynamic loader reports missing rather than the whole lib/.
missing=$(ldd "$prefix/bin/clang-23" 2>/dev/null | awk '/not found/ { print $1 }' || true)
if [ -n "$missing" ]; then
    wanted=$work/wanted
    : > "$wanted"
    while IFS= read -r so; do
        grep -E "^$top/lib/$so(\.[0-9.]+)*$" "$listing" >> "$wanted" || true
    done <<< "$missing"
    if [ -s "$wanted" ]; then
        log "extracting the shared libraries the driver links against: ${missing//$'\n'/ }"
        tar --zstd -xf "$archive" -C "$prefix" --strip-components=1 --files-from="$wanted"
    fi
    # Resolve via the archive's lib/ (RUNPATH $ORIGIN/../lib is set by the
    # official build; the symlink chain libLLVM.so -> libLLVM.so.23 may need
    # the versioned file, which the pattern above includes).
fi
finish
