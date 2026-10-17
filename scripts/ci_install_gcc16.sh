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
    local package version
    version=$(gcc-16 -dumpfullversion) || return 1
    [[ ${version%%.*} == 16 ]] || return 1
    [[ -z ${LCCC_CI_GCC_VERSION:-} || $version == "$LCCC_CI_GCC_VERSION" ]] || return 1
    for package in "${packages[@]}"; do
        [[ $(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null) == installed ]] || return 1
    done
}

# Hosted-runner apt is flaky (mirror races, PPA publication skew between the
# gcc-16 core and the multilib/lib32 binaries). Retry with backoff so a
# transient window does not fail the whole suite, and self-diagnose on the
# final failure so the step log names the unresolvable package instead of a
# bare apt exit code 100.
apt_retry() {
    local attempt rc
    for attempt in 1 2 3; do
        rc=0
        as_root "$@" || rc=$?
        if (( rc == 0 )); then
            return 0
        fi
        echo "ci_install_gcc16: '$*' failed (exit $rc, attempt $attempt/3)" >&2
        if (( attempt < 3 )); then
            # LCCC_CI_APT_RETRY_DELAY=0 keeps the mock-driven regression
            # gate fast; CI uses the real backoff.
            sleep $(( attempt * ${LCCC_CI_APT_RETRY_DELAY:-25} ))
        fi
    done
    return "$rc"
}

# Hosted runners re-download ~300 MB of PPA debs on every job (launchpad
# serves at a few hundred KB/s; measured 15 minutes per job). When
# LCCC_CI_APT_ARCHIVE_DIR is set, apt keeps its downloaded .debs in that
# directory so the workflow can persist them in actions/cache; a warm cache
# turns the install into pure local dpkg work. apt still consults the live
# index first, so a stale cache only costs the delta download, never
# correctness or the rolling-major version policy.
archive_opts=()
if [[ -n ${LCCC_CI_APT_ARCHIVE_DIR:-} ]]; then
    mkdir -p "${LCCC_CI_APT_ARCHIVE_DIR}/partial"
    archive_opts=(
        -o "Dir::Cache::archives=${LCCC_CI_APT_ARCHIVE_DIR}"
        -o APT::Keep-Downloaded-Packages=true
    )
fi

if ! toolchain_ready; then
    if [[ $MODE == --multilib ]]; then
        as_root dpkg --add-architecture i386
    fi
    apt_retry apt-get update
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
        apt_retry apt-get update
    fi
    if ! apt_retry apt-get install -y --no-install-recommends \
            -o Acquire::Retries=3 "${archive_opts[@]}" "${packages[@]}"; then
        echo "ci_install_gcc16: install failed; per-package resolver state follows" >&2
        for package in "${packages[@]}"; do
            apt-cache policy "$package" >&2 || true
        done
        # Name the exact uninstallable dependency chain in the log.
        as_root apt-get install -y --no-install-recommends --dry-run \
            "${packages[@]}" >&2 || true
        exit 1
    fi
    if [[ -n ${LCCC_CI_APT_ARCHIVE_DIR:-} ]] && (( EUID != 0 )); then
        # apt wrote the debs as root; hand them back so the post-job cache
        # upload (which runs unprivileged) can read every file.
        as_root chown -R "$(id -un):$(id -gn)" "$LCCC_CI_APT_ARCHIVE_DIR" || true
    fi
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
# A binary named g++-16 on PATH is not proof of its version. Check both
# drivers, including the already-installed fast path, before certifying the
# pair. Keep the rolling-major/explicit-version policy above unchanged.
cxx_version=$(g++-16 -dumpfullversion)
if [[ $cxx_version != "$version" ]]; then
    echo "ci_install_gcc16: g++-16 version $cxx_version does not match gcc-16 version $version" >&2
    exit 1
fi

# Capture failures before printing; consume all output (no head/SIGPIPE).
gcc_banner=$(gcc-16 --version)
gxx_banner=$(g++-16 --version)
printf 'Installed GCC toolchain: %s\n' "${gcc_banner%%$'\n'*}"
printf 'Installed G++ toolchain: %s\n' "${gxx_banner%%$'\n'*}"
