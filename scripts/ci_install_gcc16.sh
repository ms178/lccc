#!/usr/bin/env bash
# Install the GCC 16 toolchain used by CI's native oracle gates and benchmarks.
# Prefer the runner distribution; use the Ubuntu Toolchain PPA only when the
# distribution does not publish gcc-16 (the ubuntu-latest migration is rolling).
set -euo pipefail

MODE=${1:---base}
case "$MODE" in
    --base|--multilib) ;;
    *) echo "usage: $0 [--base|--multilib]" >&2; exit 2 ;;
esac

if [[ $(uname -s) != Linux ]]; then
    echo "ci_install_gcc16: Linux is required" >&2
    exit 2
fi

if (( EUID == 0 )); then
    as_root() { "$@"; }
else
    command -v sudo >/dev/null 2>&1 || {
        echo "ci_install_gcc16: sudo is required to install packages" >&2
        exit 1
    }
    as_root() { sudo -- "$@"; }
fi

if [[ $MODE == --multilib ]]; then
    as_root dpkg --add-architecture i386
fi
as_root apt-get update

candidate=$(apt-cache policy gcc-16 2>/dev/null | awk '/Candidate:/ { print $2; exit }' || true)
if [[ -z $candidate || $candidate == '(none)' ]]; then
    if ! command -v add-apt-repository >/dev/null 2>&1; then
        as_root apt-get install -y --no-install-recommends software-properties-common
    fi
    as_root add-apt-repository -y ppa:ubuntu-toolchain-r/test
    as_root apt-get update
fi

packages=(gcc-16 g++-16)
if [[ $MODE == --multilib ]]; then
    # Keep the runner's system-compiler multilib as well: older regression
    # helpers still invoke unversioned `gcc`, while the corpus oracle itself is
    # explicitly GCC 16.
    packages+=(gcc-16-multilib g++-16-multilib gcc-multilib g++-multilib libc6-dev-i386)
fi
as_root apt-get install -y --no-install-recommends "${packages[@]}"

version=$(gcc-16 -dumpfullversion)
major=${version%%.*}
if [[ $major != 16 ]]; then
    echo "ci_install_gcc16: expected GCC 16, got $version" >&2
    exit 1
fi
command -v g++-16 >/dev/null 2>&1 || {
    echo "ci_install_gcc16: g++-16 was not installed" >&2
    exit 1
}
printf 'Installed GCC toolchain: %s\n' "$(gcc-16 --version | head -n 1)"
printf 'Installed G++ toolchain: %s\n' "$(g++-16 --version | head -n 1)"
