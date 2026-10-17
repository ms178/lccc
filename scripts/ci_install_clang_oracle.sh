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
set -Eeuo pipefail

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

# Every failure is reported as a GitHub annotation when running under
# Actions: the raw job log is not always reachable from a reviewer's seat,
# the check-run API is. `fail` is for conditions this script detects; the
# ERR trap covers a tool exiting non-zero under `set -e`, naming the command,
# its status and the last lines the tools wrote to $errlog.
errlog=
annotate() {
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        local msg=$*
        msg=${msg//'%'/%25}; msg=${msg//$'\r'/%0D}; msg=${msg//$'\n'/%0A}
        printf '::error title=Clang %s oracle::%s\n' "$VERSION" "$msg"
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

# The official archives are zstd frames; some are written with a window
# larger than the decoder default, which `zstd -d` refuses without
# --long=31 (tar then reports the child failure as exit 2). Raising the
# ceiling costs nothing for frames that do not need it, so always do.
unzstd_to_stdout() { zstd -dc --long=31 "$1"; }

installed_version() {
    [ -x "$prefix/bin/clang" ] || return 1
    "$prefix/bin/clang" -dumpversion 2>/dev/null
}

errlog_or_null=/dev/null
finish() {
    local ver
    ver=$(installed_version || true)
    if [ "$ver" != "$VERSION" ]; then
        fail "$prefix/bin/clang reports '${ver:-nothing}', expected $VERSION"
    fi
    # The driver must find its resource directory (stddef.h, immintrin.h,
    # ...) or every oracle compile fails; check once here, not in each gate.
    if ! printf '#include <stddef.h>\nsize_t s;\n' |
        "$prefix/bin/clang" -fsyntax-only -x c - >/dev/null 2>"$errlog_or_null"; then
        fail "$prefix/bin/clang cannot compile against its own resource headers"
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
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
done

work=$(mktemp -d "${TMPDIR:-/tmp}/clang-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
archive=$work/$ASSET
errlog=$work/tools.err
errlog_or_null=$errlog
: > "$errlog"
# Tool identities, for the log only: never a failure path.
versions=$( { zstd --version 2>&1 | head -n 1; tar --version 2>&1 | head -n 1; } | paste -sd ';' || true)
log "tools: $versions"

log "downloading $URL"
curl --fail --location --silent --show-error --retry 5 --retry-delay 3 \
    --retry-all-errors --output "$archive" "$URL" 2>>"$errlog"
log "verifying SHA-256 of $(stat -c %s "$archive") bytes"
echo "$SHA256  $archive" | sha256sum --check --quiet - 2>>"$errlog"
log "extracting the C front end into $prefix"
rm -rf "$prefix"
mkdir -p "$prefix"
top="LLVM-$VERSION-Linux-X64"
# One listing pass selects the members; tar then extracts exactly that list.
# Files and symlinks only: naming a directory AND its files makes tar match
# the files through the directory and then report every explicitly named
# file as "Not found in archive" (exit 2). Parents are created on extraction.
listing=$work/members
unzstd_to_stdout "$archive" 2>>"$errlog" | tar -tf - > "$listing" 2>>"$errlog"
members=$(grep -E "^$top/(bin/clang(-23)?|lib/clang/23/include/.*[^/])$" "$listing" || true)
if ! grep -qx "$top/bin/clang-23" "$listing"; then
    fail "$ASSET does not contain $top/bin/clang-23 ($(wc -l < "$listing") members; first: $(head -n 1 "$listing"))"
fi
printf '%s\n' "$members" > "$work/selected"
unzstd_to_stdout "$archive" 2>>"$errlog" |
    tar -xf - -C "$prefix" --strip-components=1 --files-from="$work/selected" 2>>"$errlog"
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
        unzstd_to_stdout "$archive" 2>>"$errlog" |
            tar -xf - -C "$prefix" --strip-components=1 --files-from="$wanted" 2>>"$errlog"
    fi
    # Resolve via the archive's lib/ (RUNPATH $ORIGIN/../lib is set by the
    # official build; the symlink chain libLLVM.so -> libLLVM.so.23 may need
    # the versioned file, which the pattern above includes).
fi
finish
