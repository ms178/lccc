#!/usr/bin/env bash
# Install the mold 3.0.0 linker oracle from the reviewed GitHub-only agent
# cache, with a FUNCTIONAL proof of the x86 target set, instead of rebuilding
# mold from source on every fresh runner (~40 min at -j2).
#
# Provenance: the agent cache's `mold-3.0.0-x86_64-linux.tar.gz` carries the
# same SHA-256 (6c90d4a474c7c0409dfb575be03a5345878ac14fdba18de8b40fa58c60121189)
# that scripts/ci_install_mold_oracle.sh pins for the official upstream release
# asset, so these are the upstream release bytes — not a local rebuild whose
# feature set nobody recorded.  restore_agent_toolchain_cache.py verifies the
# archive against the tag's MANIFEST.sha256 and the reviewed in-tree lock
# before extracting it.
#
# Target-set proof (the user's X86/i686 preset): a directory name and a version
# banner do not prove which target crates were linked in.  This script links a
# trivial object for BOTH `elf_i386` and `elf_x86_64` and checks the resulting
# ELF class/machine with readelf.  mold 3.x removed the CMake build and its
# -DMOLD_TARGETS= option; the source-build equivalent is Cargo's
# `--no-default-features --features x86_64,i386` (see tools/linker/setup_oracles.sh).
# The upstream release binary supports a superset of those two targets, which
# the probe confirms rather than assumes.
#
# Usage:
#   scripts/ensure_mold_300_binary.sh [--prefix DIR] [--tag TAG] [--print-bin]
#                                     [--verify-only] [--self-test]
#
# Exit status 0 means `$PREFIX/bin/mold --version` reports exactly 3.0.0 AND
# both x86 targets linked and produced the expected ELF class/machine.
set -Eeuo pipefail

REPO=$(CDPATH= cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOCK="$REPO/engineering/toolchains/agent-cache-lock.json"
RESTORE="$REPO/scripts/restore_agent_toolchain_cache.py"
VERSION=3.0.0

tag=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tag"])' "$LOCK")
prefix=${LCCC_MOLD_PREFIX:-$HOME/.cache/lccc/mold-$VERSION}
remote=origin
print_bin=0
verify_only=0
self_test=0
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        --tag) tag=$2; shift 2 ;;
        --remote) remote=$2; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        --verify-only) verify_only=1; shift ;;
        --self-test) self_test=1; shift ;;
        -h|--help) sed -n '2,27p' "$0"; exit 0 ;;
        *) echo "ensure_mold_300_binary: unknown argument: $1" >&2; exit 2 ;;
    esac
done
[[ $prefix = /* && $prefix != / ]] || {
    echo "ensure_mold_300_binary: prefix must be an absolute private directory" >&2; exit 2; }

# The repository's one version matcher: a whole release token on the identity
# line, so 3.0.0 never matches 3.0.0-rc1 or 3.0.10.
# shellcheck source=../tools/linker/oracle_versions.sh
. "$REPO/tools/linker/oracle_versions.sh"

verify() {
    local mold=$1
    [ -x "$mold" ] || { echo "missing $mold" >&2; return 1; }
    oracle_require_version "$mold" "$VERSION" || return 1
    command -v "${AS:-as}" >/dev/null 2>&1 || {
        echo "ensure_mold_300_binary: need an assembler (as) for the target probe" >&2; return 1; }
    command -v "${READELF:-readelf}" >/dev/null 2>&1 || {
        echo "ensure_mold_300_binary: need readelf for the target probe" >&2; return 1; }
    local tmp
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-mold-probe.XXXXXX")
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp'" RETURN
    # A position-independent, libc-free _start: the probe tests the linker's
    # target support, not the C runtime or a multilib installation.
    printf '.globl _start\n_start:\n mov $1,%%eax\n mov $0,%%ebx\n int $0x80\n' > "$tmp/t.s"
    local spec bits want_class want_machine
    for spec in "32 elf_i386 ELF32 80386" "64 elf_x86_64 ELF64 X86-64"; do
        read -r bits emulation want_class want_machine <<<"$spec"
        "${AS:-as}" "--$bits" "$tmp/t.s" -o "$tmp/t$bits.o" || {
            echo "ensure_mold_300_binary: assembler cannot produce --$bits objects" >&2; return 1; }
        "$mold" -m "$emulation" -o "$tmp/t$bits" "$tmp/t$bits.o" -e _start || {
            echo "ensure_mold_300_binary: mold $VERSION cannot link $emulation" >&2; return 1; }
        local header
        header=$("${READELF:-readelf}" -h "$tmp/t$bits") || return 1
        grep -Eq "Class:[[:space:]]+$want_class" <<<"$header" || {
            echo "ensure_mold_300_binary: $emulation produced the wrong ELF class" >&2; return 1; }
        grep -Eq "Machine:.*$want_machine" <<<"$header" || {
            echo "ensure_mold_300_binary: $emulation produced the wrong machine" >&2; return 1; }
    done
}

if [ "$self_test" = 1 ]; then
    python3 "$RESTORE" --self-test
    # Offline negative probes: the verifier must reject a wrong version and a
    # binary that cannot link the x86 targets, not just a missing file.
    st=$(mktemp -d "${TMPDIR:-/tmp}/lccc-mold-selftest.XXXXXX")
    trap 'rm -rf "$st"' EXIT
    rc=0
    if verify "$st/absent" 2>/dev/null; then
        echo "self-test FAIL: a missing binary verified" >&2; rc=1
    fi
    printf '#!/bin/sh\necho "mold 2.42.1"\n' > "$st/wrong-version"
    chmod +x "$st/wrong-version"
    if verify "$st/wrong-version" 2>/dev/null; then
        echo "self-test FAIL: mold 2.42.1 accepted as 3.0.0" >&2; rc=1
    fi
    printf '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "mold 3.0.0"; exit 0; fi\nexit 1\n' \
        > "$st/right-version-no-link"
    chmod +x "$st/right-version-no-link"
    if verify "$st/right-version-no-link" 2>/dev/null; then
        echo "self-test FAIL: a non-linking binary passed the target probe" >&2; rc=1
    fi
    [ "$rc" = 0 ] && echo "ensure_mold_300_binary self-test: PASS"
    exit "$rc"
fi

if [ "$verify_only" = 1 ]; then
    verify "$prefix/bin/mold" || exit 1
    echo "mold $VERSION binary oracle OK (elf_i386 + elf_x86_64 probed): $prefix/bin"
    exit 0
fi

# Restore into the SHARED agent-cache prefix so an already-restored cache is
# reused (the restore script answers a warm prefix from its installed
# manifest in milliseconds) instead of downloading a second copy.
cache=${LCCC_AGENT_CACHE_PREFIX:-$HOME/.cache/lccc/agent-toolchains}
python3 "$RESTORE" --tag "$tag" --prefix "$cache" --repo "$REPO" --remote "$remote" --only mold
root="$cache/mold-$VERSION-x86_64-linux"
[ -x "$root/bin/mold" ] || {
    echo "ensure_mold_300_binary: cache extracted no mold executable at $root/bin/mold" >&2
    exit 1; }
# The upstream release binary needs only system libraries (verified: NEEDED
# lists libgcc_s/librt/libpthread/libdl/libc/ld-linux and it runs with
# LD_LIBRARY_PATH unset), so publishing the executable alone is safe.  The
# libexec/lib siblings are copied when present for mold-wrapper/ld.mold users.
mkdir -p "$prefix/bin"
cp -f "$root/bin/mold" "$prefix/bin/mold.tmp"
chmod 0755 "$prefix/bin/mold.tmp"
mv -f "$prefix/bin/mold.tmp" "$prefix/bin/mold"
[ ! -e "$root/bin/ld.mold" ] || ln -sf mold "$prefix/bin/ld.mold"
for sib in libexec lib; do
    if [ -d "$root/$sib" ] && [ ! -e "$prefix/$sib" ]; then
        cp -a "$root/$sib" "$prefix/$sib"
    fi
done
verify "$prefix/bin/mold"
sha256sum "$prefix/bin/mold"
if [ "$print_bin" = 1 ]; then
    echo "$prefix/bin"
fi
