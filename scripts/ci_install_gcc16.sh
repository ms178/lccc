#!/usr/bin/env bash
# Rolling-major GCC 16 oracle policy. LCCC_CI_GCC_VERSION optionally requires
# an exact upstream version; distribution package revisions remain visible in
# apt's log. Debian must use Debian packages, never an Ubuntu PPA.
#
# Provisioning contract
# ---------------------
# A normal invocation exits 0 only when every requested package is installed
# and the resulting GCC/G++ pair reports one consistent version. The read-only
# --print-packages mode exposes that exact request to the bundle exporter so
# preinstalled packages are included in the cache too. Two hosted-runner failure
# modes decide the shape of this script:
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
# a compiler package is missing, and a non-Ubuntu host never falls back to an
# Ubuntu PPA. CI oracle extras (qemu, mold, lld, PyYAML) join the same exact-name
# resolver and apt transaction, but a missing extra never triggers the GCC PPA.
set -euo pipefail

usage() {
    echo "usage: $0 [--base|--multilib] [--extras \"pkg ...\"] [--print-packages]" >&2
}

MODE=""
EXTRAS=""
extras_seen=0
PRINT_PACKAGES=0
while (( $# )); do
    case $1 in
        --base|--multilib)
            if [[ -n $MODE ]]; then
                usage
                exit 2
            fi
            MODE=$1
            shift
            ;;
        --extras)
            if (( extras_seen )) || (( $# < 2 )) || [[ $2 == --* ]]; then
                usage
                exit 2
            fi
            EXTRAS=$2
            extras_seen=1
            shift 2
            ;;
        --print-packages)
            if (( PRINT_PACKAGES )); then
                usage
                exit 2
            fi
            PRINT_PACKAGES=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage
            exit 2
            ;;
    esac
done
MODE=${MODE:---base}

if [[ $(uname -s) != Linux ]]; then
    echo "ci_install_gcc16: Linux is required" >&2
    exit 2
fi

# Keep the compiler subset separate: only its missing names can justify adding
# the Ubuntu toolchain PPA. The `packages` union below is what the resolver,
# offline-deb coverage, installed-package check and single apt transaction use.
compiler_packages=(gcc-16 g++-16)
if [[ $MODE == --multilib ]]; then
    # Older helpers still invoke unversioned gcc. Preserve BOTH multilib sets.
    compiler_packages+=(gcc-16-multilib g++-16-multilib gcc-multilib g++-multilib libc6-dev-i386 lib32gcc-16-dev)
fi
extra_packages=()
if [[ -n $EXTRAS ]]; then
    read -r -a extra_packages <<< "$EXTRAS"
fi
packages=("${compiler_packages[@]}" "${extra_packages[@]}")
if (( PRINT_PACKAGES )); then
    printf '%s\n' "${packages[@]}"
    exit 0
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

missing_installed_packages() {
    local package
    for package in "${packages[@]}"; do
        if [[ $(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null) != installed ]]; then
            printf '%s\n' "$package"
        fi
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

# Exact package presence in the index apt will actually install from (see
# contract note 2). The probe MUST carry the same index options as the install
# path: with a cached index, a probe that reads the default list directory
# reports "unresolvable" for a package apt can install offline.
package_known() {
    apt-cache "${index_opts[@]}" pkgnames "$1" 2>/dev/null | grep -Fxq -- "$1"
}

# Absent packages, one per line. DRY source of truth for both the boolean test
# and the failure diagnostic so they can never disagree.
missing_packages() {
    local package
    for package in "${packages[@]}"; do
        package_known "$package" || printf '%s\n' "$package"
    done
}

missing_compiler_packages() {
    local package
    for package in "${compiler_packages[@]}"; do
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
index_opts=()
if [[ -n ${LCCC_CI_APT_ARCHIVE_DIR:-} ]]; then
    mkdir -p "${LCCC_CI_APT_ARCHIVE_DIR}/partial" \
             "${LCCC_CI_APT_ARCHIVE_DIR}/lists/partial"
    archive_opts=(
        -o "Dir::Cache::archives=${LCCC_CI_APT_ARCHIVE_DIR}"
        -o APT::Keep-Downloaded-Packages=true
    )
    # Persisting the index next to the debs is what makes provisioning
    # survive a PPA CDN outage: a warm runner resolves gcc-16 from the cached
    # index and installs from the cached debs without fetching anything. It
    # also removes the index download from the common path entirely. apt keeps
    # the live index authoritative -- an update still refreshes this directory
    # whenever the network is healthy.
    index_opts=(-o "Dir::State::lists=${LCCC_CI_APT_ARCHIVE_DIR}/lists")
fi

# A single retryable index fetch.
#  * Acquire::Retries makes apt itself re-issue transient HTTP failures
#    instead of surrendering the whole index on one 503.
#  * Acquire::Languages=none drops the Translation-* indexes. They are ~2 MB
#    per run that nothing in this repository reads, and the PPA's translation
#    index is a recurring 503 source (observed in the failing lane).
refresh_indexes() {
    apt_retry 3 apt-get update -o Acquire::Retries=3 -o Acquire::Languages=none \
        "${index_opts[@]}"
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

# Rejected alternative, recorded so nobody re-invents it: retargeting the PPA
# to the legacy `ppa.launchpad.net` host. It looks equivalent (the hostname
# serves the same archive over plain HTTP) but apt cannot use it: the alias
# presents a certificate that does not cover the legacy name -- observed as
# "Certificate verification failed: The certificate is NOT trusted. The name in
# the certificate does not match the expected" -- and disabling peer
# verification to work around it is not an option. The outage is instead
# absorbed by retrying the real host over a longer, bounded window and by
# keeping apt's index (see LCCC_CI_APT_ARCHIVE_DIR) so a warm runner resolves
# the toolchain without touching the CDN at all.

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
            -o Acquire::Retries=3 "${archive_opts[@]}" "${index_opts[@]}" \
            "${packages[@]}" || rc=$?
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

# Offline fallback.  Every successful install keeps its downloaded debs in
# LCCC_CI_APT_ARCHIVE_DIR (APT::Keep-Downloaded-Packages=true), so a previous
# verified run already left the exact toolchain on disk, and dpkg needs no
# index at all.  This is what lets a Launchpad index outage break the *fetch*
# without failing the lane: the retry budget above still has to exhaust first,
# and the fallback stays fail-closed by construction -- it runs only when the
# cache holds a deb for every requested name, and the installed result must
# still pass the same version contract (toolchain_ready) as an apt install.
cached_debs_cover_request() {
    local package
    local -a matches
    [[ -n ${LCCC_CI_APT_ARCHIVE_DIR:-} ]] || return 1
    for package in "${packages[@]}"; do
        matches=("${LCCC_CI_APT_ARCHIVE_DIR}/${package}"_*.deb)
        [[ -e ${matches[0]} ]] || return 1
    done
}

install_from_cached_debs() {
    local -a debs
    cached_debs_cover_request || return 1
    debs=("${LCCC_CI_APT_ARCHIVE_DIR}"/*.deb)
    (( ${#debs[@]} )) || return 1
    echo "ci_install_gcc16: index unavailable; installing ${#debs[@]} cached deb(s)" >&2
    # A dependency that is neither cached nor installed makes dpkg fail; the
    # contract check below then fails closed exactly like a failed install.
    as_root dpkg --force-confdef --force-confold -i "${debs[@]}" >/dev/null 2>&1 || true
    toolchain_ready
}

# GitHub interprets `::error title=...::<message>` lines written to stdout as
# check-run annotations. Emitting one here -- instead of duplicating a capture
# wrapper in every workflow job -- makes a provisioning failure diagnosable
# from the Checks API in all six lanes, which matters because the Actions log
# download endpoint is not reachable from every API client. Annotations are
# disabled outside Actions so a developer's terminal stays clean.
annotate() {
    [[ ${GITHUB_ACTIONS:-} == true ]] || return 0
    local level=$1 title=$2 message=$3
    message=${message//%/%25}
    message=${message//$'\r'/%0D}
    message=${message//$'\n'/%0A}
    printf '::%s title=%s::%s\n' "$level" "$title" "$message"
}

# Bounded, high-signal failure report. The previous implementation dumped
# `apt-cache policy` for every regex match of every package name: 56 KB of
# unrelated `clang-16`/`libflang-16-dev` noise that buried the single line
# that mattered. Only the exact missing set, the failed index fetches and the
# resolver's own verdict are printed.
report_provisioning_failure() {
    local missing missing_installed fetches dry_run summary context
    missing=$(describe_missing)
    missing_installed=$(missing_installed_packages)
    echo "ci_install_gcc16: compiler/oracle provisioning failed (mode $MODE)" >&2
    if [[ -n $missing ]]; then
        summary="not resolvable from the configured repositories: ${missing// /, }"
        echo "ci_install_gcc16: $summary" >&2
    elif [[ -n $missing_installed ]]; then
        summary="all requested packages are resolvable; the install itself failed or left packages uninstalled: ${missing_installed//$'\n'/, }"
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
        "${index_opts[@]}" "${packages[@]}" 2>&1 | tail -n 20 || true)
    if [[ -n $dry_run ]]; then
        echo "ci_install_gcc16: resolver verdict follows" >&2
        printf '%s\n' "$dry_run" >&2
    fi
    # head -c bounds the annotation below GitHub's 64 KiB limit; the `|| true`
    # keeps SIGPIPE from a deliberately short read out of `set -e`, which would
    # otherwise abort before the annotation is printed.
    context=$(printf '%s\n%s\n%s\n%s' "$missing" "$missing_installed" "$fetches" "$dry_run" | head -c 6000 || true)
    annotate error "C oracle provisioning failed (mode $MODE)" "$summary"$'\n'"$context"
}

if ! toolchain_ready; then
    if [[ $MODE == --multilib ]]; then
        as_root dpkg --add-architecture i386
    fi
    # Distribution first: exactly one index refresh before considering the PPA.
    # Spending the PPA retry budget here would add a minute of sleeps to every
    # Ubuntu lane, where the distribution can never resolve gcc-16 anyway.
    resolvable=0
    if ensure_packages_resolvable 1; then
        resolvable=1
    elif [[ -z $(missing_compiler_packages) ]]; then
        # GCC is already resolvable; a missing oracle package is a mirror/index
        # problem, not a reason to add the Ubuntu toolchain PPA. Retry the
        # configured distribution repositories, then fail closed if the exact
        # extra names still do not appear.
        if ensure_packages_resolvable 3; then
            resolvable=1
        fi
    elif configure_ubuntu_ppa && ensure_packages_resolvable 6; then
        # The PPA edge flaps: probes a minute apart observed 503 and then a
        # healthy InRelease, and the 2026-10-21 incident outlived a four-round
        # window. Six rounds with linear backoff (25/50/75/100/125 s between
        # rounds, three apt attempts per round) ride out a multi-minute outage
        # while staying inside the job timeout.
        resolvable=1
    fi
    if (( resolvable )); then
        if ! install_packages; then
            report_provisioning_failure
            exit 1
        fi
        # apt can complete the requested transaction without leaving every
        # requested package installed (for example, a held/failed package).
        # Certify package state immediately instead of letting the later CI
        # command fail for a missing oracle after a seemingly green install.
        if ! toolchain_ready; then
            report_provisioning_failure
            exit 1
        fi
    elif install_from_cached_debs; then
        annotate warning "GCC 16 provisioned from the deb cache" \
            "the package index could not be refreshed; installed the debs cached by a previous verified run"
    else
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
