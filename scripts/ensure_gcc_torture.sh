#!/usr/bin/env bash
# ensure_gcc_torture.sh — (re)provision the GCC C torture corpus after a
# harness wipe.
#
# The Arena workspace snapshot caps at ~128 MB / ~10k files, and the extracted
# GCC testsuite (gcc.c-torture + gcc.dg) is ~22k files, while the upstream
# tarball alone is ~107 MB.  Neither may live in the persisted zone: either
# one pushes the snapshot over its cap and the harness then drops the WHOLE
# snapshot (repository included).  Both therefore live under the
# snapshot-excluded `.cache` directory and are re-derived on demand — the
# download is ~20 s, the selective extraction ~60 s on the 2-core host.
#
#   $LCCC_DL/gcc-<ver>.tar.xz                          (download cache)
#   $dest/gcc.c-torture/{execute,compile}              (torture corpus)
#   $dest/gcc.dg                                       (dg/torture headers)
#   $dest/.lccc-manifest                               (sha256 of every file)
#   $dest/.lccc-provisioned                            (stamp, see below)
#
# Trust model.  A wipe can delete part of `.cache`, and a kill can interrupt
# a replacement half-way, so the stamp alone never certifies the tree:
#
#   * The tarball is checked against a sha512 pin before it is extracted
#     (PINS below; GCC_TORTURE_SHA512 supplies one for an unpinned version).
#     A cached tarball that fails the check is deleted and downloaded again;
#     a fresh download that fails it is an error.
#   * Re-provisioning removes the stamp FIRST (before the download), then
#     swaps the directories, then validates the new tree, and only then
#     writes the stamp (via a temporary file and rename).  A failed or
#     interrupted run leaves no stamp.
#   * The stamp records the version, the tarball sha512 and the manifest
#     sha256.  It is honoured only when the manifest matches that hash, the
#     required directories exist, the file set under gcc.c-torture and
#     gcc.dg equals the manifest's exactly (nothing missing, nothing extra),
#     and every file's content matches its manifest sha256.
#   * Any failed check re-provisions automatically (from the cached tarball
#     when it is intact); the reason is printed on stderr.
#
# The stamp's first line is the bare version (e.g. gcc-16.2.0); the torture
# runner reads it as the corpus identity for its JSON reports.
#
# Usage:  scripts/ensure_gcc_torture.sh [--force] [tarball] [dest-root]
# Env:    GCC_TORTURE_VERSION (default 16.2.0), LCCC_DL (download cache dir),
#         GCC_TESTSUITE_ROOT (default dest-root), GCC_TORTURE_SHA512 (pin for
#         a version not listed in PINS; overrides nothing that is listed),
#         GCC_TORTURE_URL (download base, default https://ftp.gnu.org/gnu/gcc)
#
# The x86 torture runner (scripts/x86_gcc_torture.py) resolves the same
# default location, so `ensure_gcc_torture.sh && x86_gcc_torture.py` works
# from a freshly wiped sandbox without any environment.
# tests/regression/check_ensure_gcc_torture.sh exercises every path above.
set -euo pipefail
export LC_ALL=C

# sha512 of the release tarballs.  Each pin was cross-checked against the
# GNU release signature (gcc-<ver>.tar.xz.sig, verified with gnu-keyring.gpg
# from ftp.gnu.org) when it was added.
declare -A PINS=(
    [gcc-16.2.0]=c51c30ca7422d0cbecf504b2e0f33c3aca31e0f90a76b65217f465163fa6fa17b3f5de39e145c47e5bab90ac0ce7fff3b03c8d553ae36e01faaea5a50f8648d1
)
# What consumers need: the torture runner's execute/compile legs and the
# gcc.dg tree that torture sources include headers from.
REQUIRED_DIRS=(gcc.c-torture/execute gcc.c-torture/compile gcc.dg)
TREES=(gcc.c-torture gcc.dg)

force=0
if [[ ${1:-} == --force ]]; then
    force=1
    shift
fi

ver_num=${GCC_TORTURE_VERSION:-16.2.0}
dl=${LCCC_DL:-/home/user/.cache/lccc-dl}
tarball=${1:-$dl/gcc-${ver_num}.tar.xz}
dest=${2:-${GCC_TESTSUITE_ROOT:-/home/user/.cache/lccc-gcc-testsuite}}
url_base=${GCC_TORTURE_URL:-https://ftp.gnu.org/gnu/gcc}
ver=$(basename "$tarball" .tar.xz)
stamp="$dest/.lccc-provisioned"
manifest="$dest/.lccc-manifest"

die() {
    echo "ensure_gcc_torture: error: $*" >&2
    exit 1
}
note() { echo "ensure_gcc_torture: $*" >&2; }

pin=${PINS[$ver]:-}
if [[ -z $pin ]]; then
    pin=${GCC_TORTURE_SHA512:-}
    [[ -n $pin ]] || die "no sha512 pin for $ver; add it to PINS (after checking the GNU signature) or set GCC_TORTURE_SHA512"
fi
[[ $pin =~ ^[0-9a-f]{128}$ ]] || die "malformed sha512 pin for $ver"

report() {
    local n_exec n_comp
    n_exec=$(find "$dest/gcc.c-torture/execute" -maxdepth 1 -name '*.c' | wc -l)
    n_comp=$(find "$dest/gcc.c-torture/compile" -maxdepth 1 -name '*.c' | wc -l)
    echo "gcc.c-torture ($ver): execute=${n_exec} compile=${n_comp} sources under $dest/gcc.c-torture (manifest verified)"
}

# stamp_field KEY -> value of "KEY=value" in the stamp (empty if absent).
stamp_field() {
    sed -n "s/^$1=//p" "$stamp" | head -n1
}

# validate_tree ROOT MANIFEST EXPECTED_SHA256 -> 0, or 1 with $why set.
validate_tree() {
    local root=$1 man=$2 want=$3 d have
    if [[ ! -s $man ]]; then
        why="manifest missing"
        return 1
    fi
    have=$(sha256sum <"$man" | cut -d' ' -f1)
    if [[ $have != "$want" ]]; then
        why="manifest hash mismatch"
        return 1
    fi
    for d in "${REQUIRED_DIRS[@]}"; do
        if [[ ! -d $root/$d ]] || [[ -z $(find "$root/$d" -type f -print -quit) ]]; then
            why="required directory $d missing or empty"
            return 1
        fi
    done
    # Exact file-set equality: sha256sum -c notices missing and altered
    # files, not extra ones (a stray source would silently join the corpus).
    if ! cmp -s <(cd "$root" && find "${TREES[@]}" -type f | sort) \
        <(sed 's/^[0-9a-f]\{64\}  //' "$man"); then
        why="file set differs from manifest"
        return 1
    fi
    if ! (cd "$root" && sha256sum --quiet --strict -c "$man" >/dev/null 2>&1); then
        why="file content differs from manifest"
        return 1
    fi
    return 0
}

stamp_valid() {
    if [[ ! -f $stamp ]]; then
        why="no stamp"
        return 1
    fi
    if [[ $(head -n1 "$stamp") != "$ver" ]]; then
        why="stamp names $(head -n1 "$stamp"), want $ver"
        return 1
    fi
    if [[ $(stamp_field tarball-sha512) != "$pin" ]]; then
        why="stamp was made from a different tarball"
        return 1
    fi
    validate_tree "$dest" "$manifest" "$(stamp_field manifest-sha256)"
}

tarball_ok() {
    [[ -s $tarball ]] && [[ $(sha512sum <"$tarball" | cut -d' ' -f1) == "$pin" ]]
}

why=""
if [[ $force -eq 0 ]]; then
    if stamp_valid; then
        report
        exit 0
    fi
    note "re-provisioning $dest: $why"
fi
# The stamp is known not to certify what will be on disk from here on:
# drop it before anything else can fail, so no exit path — a failed
# download, a bad pin, an interruption during the swap — leaves one behind.
rm -f "$stamp"

if [[ -e $tarball ]] && ! tarball_ok; then
    note "cached $tarball fails its sha512 pin; discarding it"
    rm -f "$tarball"
fi
if [[ ! -e $tarball ]]; then
    mkdir -p "$(dirname "$tarball")"
    # Download off-path and rename: a wipe/kill mid-transfer must never leave
    # a truncated tarball that a later run would trust.
    curl -fsSLo "$tarball.part" "$url_base/${ver}/${ver}.tar.xz"
    mv -f "$tarball.part" "$tarball"
    tarball_ok || die "downloaded $tarball does not match its sha512 pin"
fi

mkdir -p "$dest"

tmp=$(mktemp -d "$(dirname "$dest")/.torture-extract.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
tar -xJf "$tarball" -C "$tmp" "${ver}/gcc/testsuite/gcc.c-torture" "${ver}/gcc/testsuite/gcc.dg"
src="$tmp/${ver}/gcc/testsuite"
for d in "${REQUIRED_DIRS[@]}"; do
    [[ -d $src/$d ]] || die "$tarball lacks gcc/testsuite/$d"
done
# sha256sum escapes names containing a backslash or newline, which would
# break the manifest's path column; the GCC testsuite has none, so refuse
# rather than mis-describe such a tree.
if (cd "$src" && find "${TREES[@]}" -name '*[\\
]*' -print -quit) | grep -q .; then
    die "$tarball contains a file name with a backslash or newline"
fi
(cd "$src" && find "${TREES[@]}" -type f | sort | xargs -d '\n' sha256sum) >"$tmp/manifest"
man_sha=$(sha256sum <"$tmp/manifest" | cut -d' ' -f1)

rm -rf "$dest/gcc.c-torture" "$dest/gcc.dg" "$manifest"
mv "$src/gcc.c-torture" "$src/gcc.dg" "$dest/"
mv "$tmp/manifest" "$manifest"

validate_tree "$dest" "$manifest" "$man_sha" || die "freshly provisioned tree fails validation: $why"
printf '%s\ntarball-sha512=%s\nmanifest-sha256=%s\n' "$ver" "$pin" "$man_sha" >"$stamp.tmp"
mv -f "$stamp.tmp" "$stamp"

report
