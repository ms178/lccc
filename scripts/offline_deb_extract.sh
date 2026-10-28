#!/usr/bin/env bash
# Extract trusted, offline Debian packages without touching the host system.
# Usage: scripts/offline_deb_extract.sh SYSROOT PACKAGE.deb [PACKAGE.deb ...]
# SYSROOT must be an absent or empty directory strictly below /home/user or
# /tmp. All packages for one sysroot must be provided together. Absolute Debian
# symlinks are converted to sysroot-relative links. No maintainer scripts run.
set -euo pipefail
umask 077

if (( $# < 2 )); then
    echo "usage: $0 SYSROOT PACKAGE.deb [PACKAGE.deb ...]" >&2
    exit 2
fi
if [[ $(id -u) == 0 ]]; then
    echo 'offline-deb: refuse to extract as root; run unprivileged' >&2
    exit 1
fi
sysroot=$(realpath -m -- "$1")
shift
case "$sysroot" in
    /home/user/*|/tmp/*) ;;
    *) echo "offline-deb: destination must be strictly below /home/user or /tmp: $sysroot" >&2; exit 1 ;;
esac
if [[ -e $sysroot || -L $sysroot ]]; then
    if [[ ! -d $sysroot || ! -O $sysroot || -n $(find "$sysroot" -mindepth 1 -maxdepth 1 -print -quit) ]]; then
        echo "offline-deb: destination must be an empty, user-owned directory: $sysroot" >&2
        exit 1
    fi
fi
packages=()
for package in "$@"; do
    if [[ ! -f $package ]]; then
        echo "offline-deb: no such package: $package" >&2
        exit 1
    fi
    packages+=("$(realpath -e -- "$package")")
done
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python3 "$script_dir/offline_deb_validate.py" check "${packages[@]}"

parent=$(dirname -- "$sysroot")
mkdir -p -- "$parent"
# Re-resolve after creating the parent; an existing symlink must not redirect
# the extraction outside either permitted root.
sysroot=$(realpath -m -- "$sysroot")
case "$sysroot" in
    /home/user/*|/tmp/*) ;;
    *) echo "offline-deb: destination escaped the allowed roots: $sysroot" >&2; exit 1 ;;
esac
stage=$(mktemp -d "${sysroot}.tmp.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
for package in "${packages[@]}"; do
    dpkg -x "$package" "$stage"
done
python3 "$script_dir/offline_deb_validate.py" confine "$stage"
# Never replace nonempty directories, even if the destination changed while
# packages were being checked. Rename only a completely validated staging tree.
if [[ -e $sysroot ]]; then
    rmdir -- "$sysroot"
fi
mv -T -- "$stage" "$sysroot"
trap - EXIT
printf 'Extracted %d trusted package(s) unprivileged into %s\n' "${#packages[@]}" "$sysroot"
printf 'export LCCC_SYSROOT=%q\n' "$sysroot"
cat <<'RECIPE'
export C_INCLUDE_PATH="$LCCC_SYSROOT/usr/include:$LCCC_SYSROOT/usr/include/x86_64-linux-gnu${C_INCLUDE_PATH:+:$C_INCLUDE_PATH}"
export LIBRARY_PATH="$LCCC_SYSROOT/usr/lib32:$LCCC_SYSROOT/lib32${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$LCCC_SYSROOT/lib32:$LCCC_SYSROOT/usr/lib32${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# Example for a 32-bit program (also pass suitable host/glibc include paths):
# gcc-16 -m32 --sysroot="$LCCC_SYSROOT" -B"$LCCC_SYSROOT/usr/lib32/" \
#   -Wl,--dynamic-linker="$LCCC_SYSROOT/lib32/ld-linux.so.2" source.c -o program
# LCCC recognizes LCCC_SYSROOT; use the same explicit --dynamic-linker when
# the generated program must run without a system-wide 32-bit loader.
RECIPE
