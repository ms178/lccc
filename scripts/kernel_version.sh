#!/usr/bin/env bash
# ============================================================================
# kernel_version.sh — single source of truth for "which Linux tree do the
# kernel harness scripts operate on".
#
# WHY THIS FILE EXISTS
# --------------------
# Every kernel harness script used to spell its default tree literally:
#
#     K=${KERNEL_DIR:-/home/user/target/kernel-work/linux-6.18.55}
#
# When the archpkgbuilds package moved from 6.18.55 to 6.18.58 that literal
# became wrong in twelve scripts at once, and it was wrong in *two* ways:
#
#   1. VERSION ROT.  prepare_kernel_tree.sh derives KVER from the PKGBUILD
#      (by design: "a PKGBUILD bump is picked up without touching this
#      script"), so it now materialises `linux-6.18.58`, while
#      build_kernel_boot.sh / build_kernel_full.sh / qemu_boot_test.sh still
#      looked for `linux-6.18.55`.  A freshly prepared workspace therefore
#      failed its first gate with "kernel tree missing" — a false negative
#      that looks exactly like a broken compiler.
#   2. LOCATION ROT.  prepare_kernel_tree.sh defaults to
#      `$XDG_CACHE_HOME/lccc/kernel-work` (deliberately outside the
#      repository so repository-wide scans never walk 80k kernel files), but
#      the build scripts defaulted to `$REPO/target/kernel-work`.  Even with
#      matching versions the two halves of the pipeline pointed at different
#      directories, so "prepare then build" could not work out of the box.
#
# The fix is not a new literal in twelve places; it is one resolver, sourced
# by every script, that answers the question the same way everywhere:
#
#   * an explicit KERNEL_DIR always wins (unchanged override semantics);
#   * an explicit KVER wins next;
#   * otherwise prefer the tree that actually EXISTS and is stamped complete,
#     because that is the tree the user just prepared;
#   * otherwise derive the version from the package PKGBUILD, which is the
#     only source of truth for "which version is current";
#   * otherwise fall back to the pinned version below, so the scripts still
#     name a directory (and prepare_kernel_tree.sh can create it) instead of
#     dying with an empty path.
#
# Nothing here downloads, clones, or mutates anything: it is a pure resolver,
# safe to source from set -euo pipefail scripts and cheap enough to call on
# every invocation (a handful of stat()s, no forks unless the PKGBUILD must
# be parsed).
#
# Usage (sourced):
#     # shellcheck source=kernel_version.sh
#     . "$SCRIPT_DIR/kernel_version.sh"
#     K=${KERNEL_DIR:-$(lccc_kernel_default_dir)}
#
# Usage (CLI, for scripts written in other languages):
#     kernel_version.sh --default-dir     # e.g. /home/user/.cache/lccc/kernel-work/linux-6.18.58
#     kernel_version.sh --kver            # e.g. 6.18.58
#     kernel_version.sh --workdir         # directory that holds the trees
#     kernel_version.sh --pkgdir          # archpkgbuilds package dir, if found
#
# Environment:
#   KERNEL_DIR   explicit tree           (wins over everything)
#   KVER         explicit version        (wins over discovery)
#   KERNEL_WORK  directory holding trees (default $XDG_CACHE_HOME/lccc/kernel-work)
#   PKGDIR       archpkgbuilds package dir
#   PKG_ROOT     archpkgbuilds checkout root (default .../lccc/archpkgbuilds)
# ============================================================================

# Keep the pinned fallback next to its explanation: it is only reached on a
# virgin machine with no prepared tree and no PKGBUILD checkout, where its sole
# job is to give prepare_kernel_tree.sh a directory name to create.
LCCC_KERNEL_FALLBACK_KVER=6.18.58
# The package directory name inside archpkgbuilds.  The major.minor in it is a
# *package series* name (the PKGBUILD deliberately keeps it stable across
# stable-point releases), so it must NOT be parsed as the kernel version.
LCCC_KERNEL_PKG_NAME=linux-cachymod-6.18

lccc_kernel_cache_dir() {
    printf '%s\n' "${XDG_CACHE_HOME:-$HOME/.cache}/lccc"
}

lccc_kernel_workdir() {
    printf '%s\n' "${KERNEL_WORK:-$(lccc_kernel_cache_dir)/kernel-work}"
}

lccc_kernel_pkgroot() {
    printf '%s\n' "${PKG_ROOT:-$(lccc_kernel_cache_dir)/archpkgbuilds}"
}

# lccc_kernel_pkgdir -> first package directory that actually carries a
# PKGBUILD, or empty.  $PKGDIR is honoured verbatim (the caller may point at
# any checkout); the fallbacks cover the three situations that occur in
# practice: the script-cloned cache checkout, a developer's sibling checkout of
# archpkgbuilds next to the compiler repository, and nothing at all.
lccc_kernel_pkgdir() {
    local candidate
    if [[ -n ${PKGDIR:-} ]]; then
        printf '%s\n' "$PKGDIR"
        return 0
    fi
    for candidate in \
        "$(lccc_kernel_pkgroot)/packages/$LCCC_KERNEL_PKG_NAME" \
        "$(dirname -- "$(lccc_kernel_script_dir)")/../archpkgbuilds/packages/$LCCC_KERNEL_PKG_NAME"
    do
        if [[ -r $candidate/PKGBUILD ]]; then
            printf '%s\n' "$(realpath -m -- "$candidate")"
            return 0
        fi
    done
    return 0
}

# Directory of the script that is asking, so the sibling-checkout probe above
# resolves relative to the repository rather than to the caller's cwd.  Every
# harness script computes SCRIPT_DIR itself before sourcing this file; fall
# back to BASH_SOURCE for the (rare) caller that does not.
lccc_kernel_script_dir() {
    printf '%s\n' "${SCRIPT_DIR:-$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)}"
}

# _lccc_kernel_pkgbuild_scalar <name> -> value of a `name=value` line in the
# PKGBUILD (first match).  Mirrors prepare_kernel_tree.sh's reader; kept local
# so this file stays fork-free through the common path.
_lccc_kernel_pkgbuild_scalar() {
    local pkgbuild=$1 name=$2
    sed -n "s/^$name=\\([^ #]*\\).*/\\1/p" "$pkgbuild" | head -n 1
}

# lccc_kernel_prepared_kver -> version of a tree in $KERNEL_WORK that is stamped
# complete, preferring the highest version when several are present (a session
# that bumps the package leaves the old tree behind until it is reclaimed).
lccc_kernel_prepared_kver() {
    local work tree version best=""
    work=$(lccc_kernel_workdir)
    [[ -d $work ]] || return 0
    for tree in "$work"/linux-*/; do
        [[ -d $tree ]] || continue
        [[ -f $tree/.lccc-prepared ]] || continue
        version=${tree#"$work"/linux-}
        version=${version%/}
        [[ $version =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || continue
        if [[ -z $best ]] || [[ $(printf '%s\n%s\n' "$best" "$version" | sort -V | tail -n 1) == "$version" ]]; then
            best=$version
        fi
    done
    [[ -n $best ]] && printf '%s\n' "$best"
    return 0
}

# lccc_kernel_pkgbuild_kver -> version from the package PKGBUILD, or empty.
lccc_kernel_pkgbuild_kver() {
    local pkgdir major minor
    pkgdir=$(lccc_kernel_pkgdir)
    [[ -n $pkgdir && -r $pkgdir/PKGBUILD ]] || return 0
    major=$(_lccc_kernel_pkgbuild_scalar "$pkgdir/PKGBUILD" _major)
    minor=$(_lccc_kernel_pkgbuild_scalar "$pkgdir/PKGBUILD" _minor)
    [[ -n $major && -n $minor ]] || return 0
    printf '%s.%s\n' "$major" "$minor"
}

# lccc_kernel_kver -> the version every script should agree on.
lccc_kernel_kver() {
    if [[ -n ${KVER:-} ]]; then
        printf '%s\n' "$KVER"
        return 0
    fi
    local kver
    kver=$(lccc_kernel_prepared_kver)
    if [[ -z $kver ]]; then
        kver=$(lccc_kernel_pkgbuild_kver)
    fi
    printf '%s\n' "${kver:-$LCCC_KERNEL_FALLBACK_KVER}"
}

# lccc_kernel_dir_for <kver> -> tree path inside $KERNEL_WORK.
lccc_kernel_dir_for() {
    printf '%s\n' "$(lccc_kernel_workdir)/linux-$1"
}

# lccc_kernel_default_dir -> the tree a script should use when the caller did
# not name one.  When an explicit KVER was requested but its tree does not
# exist yet, the explicit version still wins: the caller asked for it, and
# prepare_kernel_tree.sh will create exactly that directory.
lccc_kernel_default_dir() {
    if [[ -n ${KERNEL_DIR:-} ]]; then
        printf '%s\n' "$KERNEL_DIR"
        return 0
    fi
    lccc_kernel_dir_for "$(lccc_kernel_kver)"
}

# ---- CLI (only when executed, never when sourced) ---------------------------
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    case ${1:---default-dir} in
        --default-dir) lccc_kernel_default_dir ;;
        --kver) lccc_kernel_kver ;;
        --workdir) lccc_kernel_workdir ;;
        --pkgdir) lccc_kernel_pkgdir ;;
        -h | --help)
            sed -n '2,60p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            ;;
        *)
            echo "usage: kernel_version.sh [--default-dir|--kver|--workdir|--pkgdir]" >&2
            exit 2
            ;;
    esac
fi
