#!/usr/bin/env bash
# Install the GCC oracle the LLVM-oracle lanes compare against: GCC 16.2,
# EXACTLY (tests/regression/check_llvm_*.py refuse any other version, because
# every one of those oracles was measured against that release).
#
# WHY THIS EXISTS
# ---------------
# scripts/ci_install_gcc16.sh follows a ROLLING-MAJOR policy: whatever the
# distribution publishes as gcc-16 is the oracle, package revisions included.
# That is the right policy for the corpus-wide comparisons, and it is what most
# lanes want.  It cannot satisfy an exact-version gate: on the hosted runner the
# distribution's gcc-16 is 16.0.1, while the LLVM oracle family pins 16.2.
#
# Run 39822157492 is the measurement.  The first version of
# scripts/ci_verify_oracles.sh defaulted its expectation to 16.2 even for lanes
# that had never asked for it, so `Build and package compiler` and
# `Assertion-enabled units and corpus` went red on a toolchain that was exactly
# what their gates expect (16.0.1).  The repair is two-sided, and this script is
# the side that makes the pin TRUE for the lanes that do ask: a lane that pins
# 16.2 now installs 16.2 instead of asserting it against whatever apt delivered.
#
# SOURCE
# ------
# GCC publishes no official Linux binaries, so the exact release comes from the
# repository's own toolchain cache (the `harness-cache-1` ref, the same tree the
# research host provisions from): a relocatable prefix built on Debian bookworm,
# pinned here by the SHA-256 in that ref's MANIFEST.sha256.  The archive is
# self-contained -- it ships its own 32-bit libgcc (lib/gcc/*/16.2.0/32), so
# `-m32` keeps working on top of the distribution's libc6-dev-i386 -- and it is
# unpacked into a private prefix rather than installed with dpkg, so it cannot
# fight with the distribution's gcc-16 and can be cached as one directory.
#
# Every lane that pins keeps the rolling-major install too: the multilib
# packages, the extras and the `gcc` shim come from scripts/ci_provision_toolchain.sh,
# and this prefix only shadows `gcc-16`/`g++-16` (the names the gates use for
# the oracle).  Nothing here changes what `gcc` means by itself.
#
# Usage:
#   scripts/ci_install_gcc_oracle.sh [--version 16.2] [--prefix DIR]
#                                    [--from FILE] [--url URL] [--print-bin]
#   scripts/ci_install_gcc_oracle.sh --self-test
#
# Exit status 0 means `$PREFIX/bin/gcc-16 -dumpfullversion` is 16.2.x, that it
# compiles C23, and that it links AND runs both a 64-bit and a 32-bit probe.
# Under GitHub Actions the script exports $PREFIX/bin through $GITHUB_PATH.
set -Eeuo pipefail

VERSION=16.2
# The tag-cache ref, the asset name it stores, and the digest from its
# MANIFEST.sha256 (`gh api 'repos/ms178/lccc/contents/MANIFEST.sha256?ref=harness-cache-1'
#  -H 'Accept: application/vnd.github.raw'`).  The ref stores the archive with a
# `.part00` suffix; the digest covers the file without it.
REF=harness-cache-1
ASSET=gcc-16-bookworm-x86_64.tar.xz
SHA256=4fa6af4f6b29991e31631af18d83964a69caa9daf3f78546d4dc30f39880cd8a
# Derived with gcc_oracle_tree.py --archive from the archive above *after*
# verifying SHA256. Covers regular-file bytes/modes, directories and symlinks;
# does not pin host headers, binutils, CRT or libraries outside the prefix.
TREE_SHA256=0242cc2c6dc2e356e2421da01e565b1dd06983a2716dc76d31ff20a4a8eacbb9
TREE_TOOL=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/gcc_oracle_tree.py
URL="https://raw.githubusercontent.com/ms178/lccc/$REF/$ASSET.part00"
SELF_SCRIPT=$0

prefix=${LCCC_GCC_ORACLE_PREFIX:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc/gcc-oracle/$VERSION}
print_bin=0
source_archive=""
source_url=$URL
# Explicit overrides support synthetic/self-test archives. Hosted production
# invocations must not set them; environment variables cannot override pins.
expect_sha256=""
expect_tree_sha256=""
self_test=0
while [ $# -gt 0 ]; do
    case $1 in
        --version) wanted=${2:?--version needs a value}; shift 2
                   if [ "$wanted" != "$VERSION" ]; then
                       echo "ci_install_gcc_oracle: this script pins $VERSION; $wanted is not in the tag cache" >&2
                       exit 2
                   fi ;;
        --prefix) prefix=${2:?--prefix needs a value}; shift 2 ;;
        --from) source_archive=${2:?--from needs a value}; shift 2 ;;
        --url) source_url=${2:?--url needs a value}; shift 2 ;;
        --expect-sha256) expect_sha256=${2:?--expect-sha256 needs a value}; shift 2 ;;
        --expect-tree-sha256) expect_tree_sha256=${2:?--expect-tree-sha256 needs a value}; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        --self-test) self_test=1; shift ;;
        -h|--help) sed -n '2,52p' "$0"; exit 0 ;;
        *) echo "ci_install_gcc_oracle: unknown argument: $1" >&2; exit 2 ;;
    esac
done
# Custom digest overrides are for controlled test/offline archives only, and
# must be supplied together; an untrusted cached manifest is never an anchor.
if [[ -n $expect_sha256 || -n $expect_tree_sha256 ]]; then
    if [[ ! $expect_sha256 =~ ^[0-9a-f]{64}$ || ! $expect_tree_sha256 =~ ^[0-9a-f]{64}$ ]]; then
        echo 'ci_install_gcc_oracle: custom archive and tree SHA-256 values are both required' >&2
        exit 2
    fi
    TREE_SHA256=$expect_tree_sha256
fi

log() { printf '[gcc-oracle] %s\n' "$*" >&2; }

# Same annotation contract as scripts/ci_install_clang_oracle.sh: the raw job
# log is not reachable from the check-run API, so a failure has to say what it
# was in the Checks UI itself.
errlog=
annotate() {
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        local msg=$*
        msg=${msg//'%'/%25}; msg=${msg//$'\r'/%0D}; msg=${msg//$'\n'/%0A}
        printf '::error title=GCC %s oracle::%s\n' "$VERSION" "$msg"
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
    [ -x "$prefix/bin/gcc-16" ] || return 1
    "$prefix/bin/gcc-16" -dumpfullversion 2>/dev/null
}

# 32-bit execution is refused outright on kernels built without ia32
# emulation; qemu-i386 is the same fallback the regression rows use.
run_probe() {
    local dir=$1 bin=$2 rc=0
    "$bin" >"$dir/out" 2>"$dir/err" || rc=$?
    if (( rc == 126 || rc == 127 )); then
        command -v qemu-i386 >/dev/null 2>&1 || return "$rc"
        qemu-i386 "$bin" >"$dir/out" 2>"$dir/err" || return $?
    elif (( rc != 0 )); then
        return "$rc"
    fi
    [[ $(sed -n '1p' "$dir/out") == ok ]]
}

work=
errlog_or_null=/dev/null
finish() {
    local ver probe
    ver=$(installed_version || true)
    case $ver in
        "$VERSION"|"$VERSION".*) ;;
        *) fail "$prefix/bin/gcc-16 reports '${ver:-nothing}', expected $VERSION.x" ;;
    esac
    # A compiler that cannot parse the standard its own gates use, or that
    # cannot link its own 32-bit runtime, is not the oracle they pin.
    # Here-strings throughout: a producer that loses the race to a consumer
    # which exits early takes SIGPIPE, and under `set -o pipefail` that reads
    # as a broken compiler.
    if ! "$prefix/bin/gcc-16" -std=c23 -x c -fsyntax-only - >/dev/null 2>"$errlog_or_null" \
        <<<$'#include <stddef.h>\nsize_t s;\nint main(void) { return 0; }'; then
        fail "$prefix/bin/gcc-16 cannot compile -std=c23"
    fi
    probe=$(mktemp -d "${TMPDIR:-/tmp}/gcc-oracle-probe.XXXXXX")
    if ! "$prefix/bin/gcc-16" -O1 -x c - -o "$probe/p64" >/dev/null 2>"$errlog_or_null" \
        <<<$'#include <stdio.h>\nint main(void) { puts("ok"); return 0; }'; then
        fail "$prefix/bin/gcc-16 cannot link its 64-bit probe"
    fi
    if ! run_probe "$probe" "$probe/p64"; then
        fail "$prefix/bin/gcc-16's 64-bit probe did not print ok: $(sed -n '1,3p' "$probe/err" | tr '\n' ' ')"
    fi
    if ! "$prefix/bin/gcc-16" -m32 -x c - -o "$probe/p32" >/dev/null 2>"$errlog_or_null" \
        <<<$'#include <stdio.h>\nint main(void) { puts("ok"); return 0; }'; then
        fail "$prefix/bin/gcc-16 cannot link -m32 (the archive ships 32/libgcc, so this is a broken prefix)"
    fi
    if ! run_probe "$probe" "$probe/p32"; then
        fail "$prefix/bin/gcc-16's -m32 probe did not print ok (is libc6-dev-i386 installed?): $(sed -n '1,3p' "$probe/err" | tr '\n' ' ')"
    fi
    rm -rf "$probe"
    log "GCC $ver oracle at $prefix/bin/gcc-16"
    if [ -n "${GITHUB_PATH:-}" ]; then
        echo "$prefix/bin" >> "$GITHUB_PATH"
    fi
    if [ "$print_bin" = 1 ]; then
        echo "$prefix/bin/gcc-16"
    fi
}

self_test() {
    # The payload cannot be exercised here -- every lane's real archive is 65 MB
    # of compiler -- so the CONTRACT is: a private prefix, a digest check that
    # refuses a doctored archive, and a version/probe check that refuses a
    # compiler that is not 16.2.  A mock compiler script keeps probes real
    # enough to fail: it answers -dumpfullversion, writes an executable for
    # `-o`, and the executable prints ok.
    local root status digest tree_digest
    # Synthetic prefixes are deleted at the end and must not pollute the
    # caller's actual GitHub Actions PATH for subsequent steps.
    local GITHUB_PATH=
    root=$(mktemp -d "${TMPDIR:-/tmp}/gcc-oracle-test.XXXXXX")
    mkdir -p "$root/tree/gcc-16/bin"
    cat >"$root/tree/gcc-16/bin/gcc-16" <<'MOCK'
#!/bin/sh
for a in "$@"; do [ "$a" = -dumpfullversion ] && { printf '%s\n' "${MOCK_VERSION:-16.2.0}"; exit 0; }; done
prev=""; out=""
cat >/dev/null 2>&1 || true
for a in "$@"; do [ "$prev" = -o ] && out=$a; prev=$a; done
[ -n "$out" ] && { printf '#!/bin/sh\necho ok\n' >"$out"; chmod +x "$out"; }
exit 0
MOCK
    cp "$root/tree/gcc-16/bin/gcc-16" "$root/tree/gcc-16/bin/g++-16"
    chmod +x "$root/tree/gcc-16/bin/gcc-16" "$root/tree/gcc-16/bin/g++-16"
    tar -C "$root/tree" -cJf "$root/good.tar.xz" gcc-16
    cp "$root/good.tar.xz" "$root/doctored.tar.xz"
    printf 'garbage' >>"$root/doctored.tar.xz"
    digest=$(sha256sum "$root/good.tar.xz" | cut -d' ' -f1)
    tree_digest=$(python3 "$TREE_TOOL" --archive "$root/good.tar.xz")

    # 1. A verified archive lands in the private prefix, and $GITHUB_PATH gets it.
    status=0
    : >"$root/path"
    GITHUB_PATH=$root/path \
        bash "$SELF_SCRIPT" --from "$root/good.tar.xz" --prefix "$root/installed" \
        --expect-sha256 "$digest" --expect-tree-sha256 "$tree_digest" >"$root/out" 2>"$root/err" || status=$?
    if (( status == 0 )) && [ -x "$root/installed/bin/gcc-16" ] \
            && grep -qx "$root/installed/bin" "$root/path"; then
        echo "  ok   verified archive installed and exported through GITHUB_PATH"
    else
        echo "  FAIL install: exit $status, err=$(sed -n '1,5p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi

    # A warm, intact cache works even if the archive is unavailable.
    status=0
    bash "$SELF_SCRIPT" --from "$root/missing.tar.xz" --prefix "$root/installed" \
        --expect-sha256 "$digest" --expect-tree-sha256 "$tree_digest" >"$root/out" 2>"$root/err" || status=$?
    if (( status != 0 )); then
        echo "  FAIL valid warm cache: $(sed -n '1,5p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi
    echo "  ok   intact warm cache verified before use"

    # Change an auxiliary compiler that the version and C probes never execute.
    # It must force reinstallation instead of passing as a valid cache.
    printf '\n# altered auxiliary compiler\n' >>"$root/installed/bin/g++-16"
    status=0
    bash "$SELF_SCRIPT" --from "$root/good.tar.xz" --prefix "$root/installed" \
        --expect-sha256 "$digest" --expect-tree-sha256 "$tree_digest" >"$root/out" 2>"$root/err" || status=$?
    if (( status != 0 )) || grep -q 'altered auxiliary compiler' "$root/installed/bin/g++-16"; then
        echo "  FAIL mutated cached auxiliary file: $(sed -n '1,8p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi
    echo "  ok   mutated auxiliary compiler rejected and reinstalled"

    # A malicious cached driver must not even be executed for -dumpfullversion.
    { printf '#!/bin/sh\nprintf executed > "$POISON_MARKER"\n'; tail -n +2 "$root/installed/bin/gcc-16"; } >"$root/poison"
    cp "$root/poison" "$root/installed/bin/gcc-16"
    chmod +x "$root/installed/bin/gcc-16"
    status=0
    POISON_MARKER="$root/executed" bash "$SELF_SCRIPT" --from "$root/good.tar.xz" --prefix "$root/installed" \
        --expect-sha256 "$digest" --expect-tree-sha256 "$tree_digest" >"$root/out" 2>"$root/err" || status=$?
    if (( status != 0 )) || [ -e "$root/executed" ]; then
        echo "  FAIL unverified driver execution: $(sed -n '1,8p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi
    echo "  ok   poisoned cached driver rejected before execution"

    # 2. The same archive is refused when its digest does not match, and the
    #    message names the asset rather than "exit 1".
    status=0
    bash "$SELF_SCRIPT" --from "$root/doctored.tar.xz" --prefix "$root/refused" \
        --expect-sha256 "$digest" --expect-tree-sha256 "$tree_digest" >"$root/out" 2>"$root/err" || status=$?
    if (( status != 0 )) && grep -q 'does not match' "$root/err"; then
        echo "  ok   a doctored archive is refused by digest"
    else
        echo "  FAIL digest check: exit $status, err=$(sed -n '1,5p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi

    # 3. A compiler that is not the pinned release is refused before any lane
    #    spends time on a gate that would have judged it.
    status=0
    MOCK_VERSION=16.0.1 bash "$SELF_SCRIPT" --from "$root/good.tar.xz" --prefix "$root/wrongver" \
        --expect-sha256 "$digest" --expect-tree-sha256 "$tree_digest" >"$root/out" 2>"$root/err" || status=$?
    if (( status != 0 )) && grep -q 'expected 16.2' "$root/err"; then
        echo "  ok   a 16.0.1 compiler is refused, naming the pinned version"
    else
        echo "  FAIL version check: exit $status, err=$(sed -n '1,5p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi

    # 4. A version this cache does not hold is refused instead of downloaded.
    status=0
    bash "$SELF_SCRIPT" --version 17.1 >"$root/out" 2>"$root/err" || status=$?
    if (( status != 0 )) && grep -q 'not in the tag cache' "$root/err"; then
        echo "  ok   an unpinned release is refused, not silently fetched"
    else
        echo "  FAIL unknown version: exit $status, err=$(sed -n '1,5p' "$root/err")" >&2
        rm -rf "$root"; return 1
    fi

    echo "  self-test: 7/7 cases pass"
    rm -rf "$root"
    return 0
}

if (( self_test )); then
    self_test
    exit $?
fi

# Never execute even the version probe from an unverified cache. A valid
# version banner and a few successful probes do not establish cc1's contents.
# The expectation comes from the verified archive, not a mutable cache file.
command -v python3 >/dev/null 2>&1 || fail 'python3 is required to verify the oracle tree'
if [ -e "$prefix" ] || [ -L "$prefix" ]; then
    if python3 "$TREE_TOOL" --directory "$prefix" --expect "$TREE_SHA256" >/dev/null; then
        finish
        exit
    fi
    log "cached oracle tree failed integrity check; reinstalling without executing it"
fi

for tool in tar xz sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
done

work=$(mktemp -d "${TMPDIR:-/tmp}/gcc-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
errlog=$work/tools.err
errlog_or_null=$errlog
: > "$errlog"

archive=$source_archive
if [ -z "$archive" ]; then
    command -v curl >/dev/null 2>&1 || fail "curl is required to fetch $source_url"
    archive=$work/$ASSET
    log "downloading $source_url"
    curl --fail --location --silent --show-error --retry 5 --retry-delay 3 \
        --retry-all-errors --output "$archive" "$source_url" 2>>"$errlog" ||
        fail "cannot fetch $source_url (the tag cache ref is '$REF'); use --from FILE for an offline mirror"
fi

expect=${expect_sha256:-$SHA256}
log "verifying SHA-256 of $(stat -c %s "$archive") bytes"
if ! printf '%s  %s\n' "$expect" "$archive" | sha256sum --check --quiet - 2>>"$errlog"; then
    fail "$ASSET does not match the SHA-256 in $REF/MANIFEST.sha256 ($expect); refusing to install a doctored toolchain"
fi

log "extracting into $prefix"
rm -rf "$prefix"
mkdir -p "$prefix"
tar -xJpf "$archive" -C "$prefix" --strip-components=1 2>>"$errlog"
if ! python3 "$TREE_TOOL" --directory "$prefix" --expect "$TREE_SHA256" >>"$errlog" 2>&1; then
    fail "extracted oracle tree does not match its pinned content identity"
fi
finish
