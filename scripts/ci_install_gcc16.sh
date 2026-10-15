#!/usr/bin/env bash
# Rolling-major GCC 16 oracle policy. LCCC_CI_GCC_VERSION optionally requires
# an exact upstream version; distribution package revisions remain visible in
# apt's log. Debian must use Debian packages, never an Ubuntu PPA.
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

packages=(gcc-16 g++-16)
if [[ $MODE == --multilib ]]; then
    # Older helpers still invoke unversioned gcc. Preserve BOTH multilib sets.
    packages+=(gcc-16-multilib g++-16-multilib gcc-multilib g++-multilib libc6-dev-i386 lib32gcc-16-dev)
fi

toolchain_ready() {
    command -v gcc-16 >/dev/null 2>&1 && command -v g++-16 >/dev/null 2>&1 || return 1
    local package
    for package in "${packages[@]}"; do
        [[ $(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null) == installed ]] || return 1
    done
}

if ! toolchain_ready; then
    if [[ $MODE == --multilib ]]; then
        as_root dpkg --add-architecture i386
    fi
    as_root apt-get update
    candidate=$(apt-cache policy gcc-16 | awk '/Candidate:/ { print $2; exit }')
    if [[ -z $candidate || $candidate == '(none)' ]]; then
        # Override is useful for mock-only tests; normal callers use the host.
        # shellcheck disable=SC1090
        source "${LCCC_CI_OS_RELEASE:-/etc/os-release}"
        if [[ ${ID:-} != ubuntu ]]; then
            echo "ci_install_gcc16: gcc-16 unavailable from ${ID:-unknown} repositories; refusing an Ubuntu PPA" >&2
            exit 1
        fi
        if ! command -v add-apt-repository >/dev/null 2>&1; then
            as_root apt-get install -y --no-install-recommends software-properties-common
        fi
        as_root add-apt-repository -y ppa:ubuntu-toolchain-r/test
        as_root apt-get update
    fi
    as_root apt-get install -y --no-install-recommends "${packages[@]}"
fi

version=$(gcc-16 -dumpfullversion)
if [[ ${version%%.*} != 16 || -n ${LCCC_CI_GCC_VERSION:-} && $version != "$LCCC_CI_GCC_VERSION" ]]; then
    echo "ci_install_gcc16: expected GCC ${LCCC_CI_GCC_VERSION:-16.x}, got $version" >&2
    exit 1
fi
command -v g++-16 >/dev/null 2>&1 || {
    echo "ci_install_gcc16: g++-16 was not installed" >&2
    exit 1
}
# Capture failures before printing; consume all output (no head/SIGPIPE).
gcc_banner=$(gcc-16 --version)
gxx_banner=$(g++-16 --version)
printf 'Installed GCC toolchain: %s\n' "${gcc_banner%%$'\n'*}"
printf 'Installed G++ toolchain: %s\n' "${gxx_banner%%$'\n'*}"
