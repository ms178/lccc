#!/usr/bin/env bash
# Install the Clang oracle the ext_vector_type gates compare against:
# Clang 23.1.5, EXACTLY (tests/regression/lib/clang_oracle.sh states why the
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
# Exit status 0 means `$PREFIX/bin/clang -dumpversion` prints 23.1.5. Under
# GitHub Actions the script also exports CLANG=<that binary> through
# $GITHUB_ENV so every later step's gate resolves the oracle via $CLANG.
#
# This installer is the single live Clang pin of the repository. The
# `harness-cache-4` release asset and the `harness-cache-7` tag payload still
# carry a repacked 23.1.4 (a main-branch patch cannot re-issue release assets);
# both are recorded as superseded transport payloads in
# scripts/check_toolchain_cache_freshness.py and are re-issued as 23.1.5 by
# scripts/publish_toolchain_cache.py / publish_agent_toolchain_cache.sh on
# their next hosted run.
set -Eeuo pipefail

VERSION=23.1.5
ASSET="LLVM-$VERSION-Linux-X64.tar.zst"
URL="https://github.com/llvm/llvm-project/releases/download/llvmorg-$VERSION/$ASSET"
# `gh api repos/llvm/llvm-project/releases/tags/llvmorg-23.1.5 --jq
#  '.assets[] | select(.name=="LLVM-23.1.5-Linux-X64.tar.zst") | .digest'`
# (published 2026-11-03; re-verified live against that API on 2026-11-05)
SHA256=c3107384835f788768f614db610e87fba7077f125ca950eb02a78bab0a308065

prefix=${LCCC_CLANG_ORACLE_PREFIX:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc/clang-oracle/$VERSION}
print_bin=0
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        # Callers may *state* the version they expect; it is verified against
        # the pin rather than honoured, because two installable Clangs would
        # make every "matches the Clang oracle" claim ambiguous. The flag
        # exists so a workflow that names a version fails here, at
        # provisioning, instead of silently installing a different one.
        --version)
            [ "${2:-}" = "$VERSION" ] || {
                echo "ci_install_clang_oracle: --version ${2:-} is not the pinned oracle $VERSION" >&2
                exit 2
            }
            shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
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

# Preflight the free space of the directory the archive lands in.
#
# The asset is >1 GiB, and on hosts whose TMPDIR is a small tmpfs -- agent
# sandboxes mount /tmp as one, this harness has 992 MiB -- curl dies part-way
# with exit 23 "write error". That reads exactly like a network failure, so the
# real cause (a full filesystem) is invisible and the retry loop just repeats
# it. Asking the filesystem first turns it into a named, actionable condition.
# The size comes from the server (a HEAD costs one round trip); the fallback
# constant is the 23.1.5 asset plus extraction headroom.
asset_bytes=$(curl --head --location --silent --show-error --max-time 30 \
    -o /dev/null -w '%{size_download}' "$URL" 2>>"$errlog" || true)
[ "${asset_bytes:-0}" -gt 1048576 ] 2>/dev/null || asset_bytes=1198896522
need_kb=$(( (asset_bytes * 11 / 10 + 209715200) / 1024 ))
avail_kb=$(df -Pk "$work" 2>/dev/null | awk 'NR==2 {print $4}')
if [ -n "${avail_kb:-}" ] && [ "$avail_kb" -lt "$need_kb" ]; then
    fail "$work has $((avail_kb / 1024)) MiB free; downloading $ASSET needs ~$((need_kb / 1024)) MiB. Point TMPDIR at a disk-backed directory, e.g. TMPDIR=/var/tmp."
fi
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
