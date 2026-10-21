#!/usr/bin/env bash
# Rolling-major GCC 16 oracle policy. LCCC_CI_GCC_VERSION optionally requires
# an exact upstream version; distribution package revisions remain visible in
# apt's log. Debian must use Debian packages, never an Ubuntu PPA.
#
# Provisioning contract
# ---------------------
# Exit 0 means, and only means, that every requested package is installed and
# the resulting GCC/G++ pair reports one consistent version. Two hosted-runner
# failure modes decide the shape of this script:
#
#  1. `apt-get update` exits 0 even when an individual repository index fails
#     to download: it prints `W: Some index files failed to download` and
#     keeps going. A successful update therefore proves nothing about whether
#     `gcc-16` is resolvable. The Launchpad PPA answers 503 under load, and
#     that exact 503 turned into three pointless `apt-get install` retries and
#     a red pipeline lane (see engineering/FOLLOWUP-2026-10-21-gcc16-provisioning-503.md).
#     Resolution is therefore keyed on the real requirement: every requested
#     package name must be present in the current index.
#  2. `apt-cache policy <name>` treats an unknown name as a *regex*. With
#     `gcc-16` absent it happily reports `clang-16`'s candidate, which would
#     "prove" the toolchain is available and skip the PPA entirely. Presence
#     is therefore tested against the exact package name via `apt-cache
#     pkgnames`, which lists real names only.
#
# The distribution is tried first; the Ubuntu toolchain PPA is added only when
# the distribution itself cannot resolve the packages, and a non-Ubuntu host
# never falls back to an Ubuntu PPA.
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

# LCCC_CI_APT_RETRY_DELAY=0 keeps the mock-driven regression gate fast; CI
# uses the real backoff (hosted-runner mirrors are rate limited and answer 5xx
# under load, so an immediate retry usually hits the same warm failure).
retry_delay=${LCCC_CI_APT_RETRY_DELAY:-25}
if [[ ! $retry_delay =~ ^[0-9]+$ ]]; then
    echo "ci_install_gcc16: LCCC_CI_APT_RETRY_DELAY must be a non-negative integer, got '$retry_delay'" >&2
    exit 2
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

# Retry one privileged command with linear backoff. Used for operations whose
# failure is genuinely transient (index fetch, deb download); never used to
# replay an operation whose inputs are known to be wrong.
apt_retry() {
    local attempts=$1 rc=0 attempt
    shift
    for (( attempt = 1; attempt <= attempts; attempt++ )); do
        rc=0
        as_root "$@" || rc=$?
        if (( rc == 0 )); then
            return 0
        fi
        echo "ci_install_gcc16: '$*' failed (exit $rc, attempt $attempt/$attempts)" >&2
        if (( attempt < attempts )); then
            sleep $(( attempt * retry_delay ))
        fi
    done
    return "$rc"
}

# Exact package presence in the current index (see contract note 2).
package_known() {
    apt-cache pkgnames "$1" 2>/dev/null | grep -Fxq -- "$1"
}

# Absent packages, one per line. DRY source of truth for both the boolean test
# and the failure diagnostic so they can never disagree.
missing_packages() {
    local package
    for package in "${packages[@]}"; do
        package_known "$package" || printf '%s\n' "$package"
    done
}

all_packages_known() {
    [[ -z $(missing_packages) ]]
}

describe_missing() {
    local missing
    missing=$(missing_packages)
    printf '%s' "${missing//$'\n'/ }"
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

# A single retryable index fetch.
#  * Acquire::Retries makes apt itself re-issue transient HTTP failures
#    instead of surrendering the whole index on one 503.
#  * Acquire::Languages=none drops the Translation-* indexes. They are ~2 MB
#    per run that nothing in this repository reads, and the PPA's translation
#    index is a recurring 503 source (observed in the failing lane).
refresh_indexes() {
    apt_retry 3 apt-get update -o Acquire::Retries=3 -o Acquire::Languages=none
}

# Refresh until every requested package is resolvable. The loop is keyed on
# the real contract (resolvability), not on apt's exit status, so an update
# that "succeeds" while the PPA index 503'd still retries instead of burning
# three install attempts on a package that is not in the index at all.
ensure_packages_resolvable() {
    local rounds=$1 round
    if all_packages_known; then
        return 0
    fi
    for (( round = 1; round <= rounds; round++ )); do
        refresh_indexes || true
        if all_packages_known; then
            return 0
        fi
        echo "ci_install_gcc16: unresolvable after index refresh $round/$rounds (missing: $(describe_missing))" >&2
        if (( round < rounds )); then
            sleep $(( round * retry_delay ))
        fi
    done
    return 1
}

# The Launchpad PPA is served by two hosts. `ppa.launchpadcontent.net` is the
# current CDN edge; on 2026-10-21 it answered 503 for every index path
# (InRelease and Packages.gz alike) for hours, which no amount of retrying can
# fix, while the legacy `ppa.launchpad.net` host kept serving the same signed
# archive. Retargeting the configured source is therefore the only way to keep
# the lanes alive during such an outage. It is a *fallback*: it runs only after
# the primary host has exhausted its retry budget and it announces itself, so
# nobody mistakes a legacy-host build for a healthy one.
retarget_ppa_to_legacy_host() {
    local directory=${LCCC_CI_APT_SOURCES_DIR:-/etc/apt/sources.list.d}
    local file retargeted=0
    for file in "$directory"/*toolchain-r*test*.sources \
                "$directory"/*toolchain-r*test*.list; do
        [[ -f $file ]] || continue
        grep -q 'ppa\.launchpadcontent\.net' "$file" 2>/dev/null || continue
        as_root sed -i 's#ppa\.launchpadcontent\.net#ppa.launchpad.net#g' "$file"
        retargeted=1
        echo "ci_install_gcc16: retargeted $file to ppa.launchpad.net (primary CDN unusable)" >&2
    done
    if (( retargeted == 0 )); then
        echo "ci_install_gcc16: no configured PPA source to retarget" >&2
        return 1
    fi
    # Announce only what actually happened: a "degraded" annotation on a run
    # that never retargeted would be a lie in the check run.
    annotate "GCC 16 provisioning degraded" \
        "the primary PPA host was unusable; retargeted the toolchain PPA to ppa.launchpad.net"
    return 0
}

configure_ubuntu_ppa() {
    # shellcheck disable=SC1090
    source "${LCCC_CI_OS_RELEASE:-/etc/os-release}"
    if [[ ${ID:-} != ubuntu ]]; then
        echo "ci_install_gcc16: gcc-16 unavailable from ${ID:-unknown} repositories; refusing an Ubuntu PPA" >&2
        return 1
    fi
    if ! command -v add-apt-repository >/dev/null 2>&1; then
        apt_retry 3 apt-get install -y --no-install-recommends software-properties-common
    fi
    as_root add-apt-repository -y ppa:ubuntu-toolchain-r/test
}

install_packages() {
    local attempt=1 rc=0
    while (( attempt <= 3 )); do
        rc=0
        as_root apt-get install -y --no-install-recommends \
            -o Acquire::Retries=3 "${archive_opts[@]}" "${packages[@]}" || rc=$?
        if (( rc == 0 )); then
            return 0
        fi
        echo "ci_install_gcc16: install failed (exit $rc, attempt $attempt/3)" >&2
        if (( attempt == 3 )); then
            break
        fi
        # Both apt failure families that survive Acquire::Retries are index
        # problems: a repository index that 503'd mid-install ("Unable to
        # locate package") or a stale list whose deb URLs rotated (404). Both
        # are fixed by a refresh, and a plain replay is not, so refresh before
        # spending another attempt. This only costs time on the failure path.
        refresh_indexes || true
        sleep $(( attempt * retry_delay ))
        attempt=$(( attempt + 1 ))
    done
    return "$rc"
}

# GitHub interprets `::error title=...::<message>` lines written to stdout as
# check-run annotations. Emitting one here -- instead of duplicating a capture
# wrapper in every workflow job -- makes a provisioning failure diagnosable
# from the Checks API in all six lanes, which matters because the Actions log
# download endpoint is not reachable from every API client. Annotations are
# disabled outside Actions so a developer's terminal stays clean.
annotate() {
    [[ ${GITHUB_ACTIONS:-} == true ]] || return 0
    local title=$1 message=$2
    message=${message//%/%25}
    message=${message//$'\r'/%0D}
    message=${message//$'\n'/%0A}
    printf '::error title=%s::%s\n' "$title" "$message"
}

# Bounded, high-signal failure report. The previous implementation dumped
# `apt-cache policy` for every regex match of every package name: 56 KB of
# unrelated `clang-16`/`libflang-16-dev` noise that buried the single line
# that mattered. Only the exact missing set, the failed index fetches and the
# resolver's own verdict are printed.
report_provisioning_failure() {
    local missing fetches dry_run summary context
    missing=$(describe_missing)
    echo "ci_install_gcc16: provisioning failed (mode $MODE)" >&2
    if [[ -n $missing ]]; then
        summary="not resolvable from the configured repositories: ${missing// /, }"
        echo "ci_install_gcc16: $summary" >&2
    else
        summary="all requested packages are resolvable; the install itself failed"
        echo "ci_install_gcc16: $summary" >&2
    fi
    fetches=$(as_root apt-get update -o Acquire::Retries=1 2>&1 \
        | grep -E '^(Err:|E:|W:)' | head -n 20 || true)
    if [[ -n $fetches ]]; then
        echo "ci_install_gcc16: failing index fetches follow" >&2
        printf '%s\n' "$fetches" >&2
    fi
    dry_run=$(as_root apt-get install -y --no-install-recommends --dry-run \
        "${packages[@]}" 2>&1 | tail -n 20 || true)
    if [[ -n $dry_run ]]; then
        echo "ci_install_gcc16: resolver verdict follows" >&2
        printf '%s\n' "$dry_run" >&2
    fi
    # head -c bounds the annotation below GitHub's 64 KiB limit; the `|| true`
    # keeps SIGPIPE from a deliberately short read out of `set -e`, which would
    # otherwise abort before the annotation is printed.
    context=$(printf '%s\n%s\n%s' "$missing" "$fetches" "$dry_run" | head -c 6000 || true)
    annotate "GCC 16 provisioning failed (mode $MODE)" "$summary"$'\n'"$context"
}

if ! toolchain_ready; then
    if [[ $MODE == --multilib ]]; then
        as_root dpkg --add-architecture i386
    fi
    # Distribution first: exactly one index refresh before considering the PPA.
    # Spending the PPA retry budget here would add a minute of sleeps to every
    # Ubuntu lane, where the distribution can never resolve gcc-16 anyway.
    if ! ensure_packages_resolvable 1; then
        if ! configure_ubuntu_ppa; then
            report_provisioning_failure
            exit 1
        fi
        # Two rounds on the primary host, then the legacy host gets the same
        # budget. Spending the whole budget on a CDN that is down for hours
        # only makes the failure slower, and switching earlier makes it
        # impossible to tell a transient hiccup from an outage.
        if ! ensure_packages_resolvable 2; then
            if ! retarget_ppa_to_legacy_host; then
                report_provisioning_failure
                exit 1
            fi
            if ! ensure_packages_resolvable 2; then
                report_provisioning_failure
                exit 1
            fi
        fi
    fi
    if ! install_packages; then
        report_provisioning_failure
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

# Fail closed at the point of provisioning, not forty minutes later: every
# --multilib caller runs the i686 differential suites through `gcc -m32`, and
# a silently broken multilib (missing lib32 runtime, PPA lib32 package lag)
# would otherwise surface as dozens of unrelated test failures.
if [[ $MODE == --multilib ]]; then
    m32_binary=${TMPDIR:-/tmp}/lccc-ci-m32-probe
    if ! printf 'int main(void) { return 0; }\n' \
            | gcc-16 -m32 -x c - -o "$m32_binary" 2>/dev/null; then
        rm -f "$m32_binary"
        echo "ci_install_gcc16: gcc-16 -m32 cannot link; the i686 multilib is incomplete" >&2
        exit 1
    fi
    rm -f "$m32_binary"
fi

# Capture failures before printing; consume all output (no head/SIGPIPE).
gcc_banner=$(gcc-16 --version)
gxx_banner=$(g++-16 --version)
printf 'Installed GCC toolchain: %s\n' "${gcc_banner%%$'\n'*}"
printf 'Installed G++ toolchain: %s\n' "${gxx_banner%%$'\n'*}"
