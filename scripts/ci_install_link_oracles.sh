#!/usr/bin/env bash
# Install the differential linker oracles the linker contract gates compare
# lccc-ld against: mold and lld.
#
# Why this is a script and not two apt-get lines in the workflow
# -------------------------------------------------------------
# `sudo apt-get update` against the hosted mirrors fails intermittently -- 5xx
# and connection resets under load -- and the failure is transient: the same
# job re-run minutes later succeeds. The step that installs these two oracles
# was the last bare `apt-get update` + `apt-get install` left in ci.yml, with
# no retry, so one warm-mirror blip red-failed the job at *step 8 of 21* and
# skipped every gate below it, including the build and the whole test suite.
# A network hiccup therefore read as a compiler defect, and the six copies of
# the step meant six independent chances per run to hit it.
#
# GCC 16 provisioning already solved exactly this class of failure in
# ci_install_gcc16.sh (see engineering/FOLLOWUP-2026-10-21-gcc16-provisioning-503.md).
# This script applies that same discipline here rather than leaving one apt
# step outside it, and collapses the six workflow copies onto one tested path.
#
# Contract
# --------
# Exit 0 means, and only means, that `mold` and one of `ld.lld`/`lld` are on
# PATH -- the exact predicates tests/linker/run_linker_tests.py uses to decide
# whether each oracle is available. Exit 1 means the index or the download
# failed after every attempt; exit 2 is a usage or platform error.
#
# The script is idempotent and cache-friendly: when both oracles are already
# installed it returns without touching the network, so a warm runner pays
# nothing. Resolution is keyed on resolvability, not on apt's exit status,
# because `apt-get update` exits 0 while printing
# "W: Some index files failed to download".
set -Eeuo pipefail

if [[ $(uname -s) != Linux ]]; then
    echo "ci_install_link_oracles: Linux is required" >&2
    exit 2
fi

if (( EUID == 0 )); then
    as_root() { "$@"; }
else
    command -v sudo >/dev/null 2>&1 || {
        echo "ci_install_link_oracles: sudo is required to install packages" >&2
        exit 1
    }
    as_root() { sudo -- "$@"; }
fi

packages=(mold lld)

# Same knob and default as ci_install_gcc16.sh: LCCC_CI_APT_RETRY_DELAY=0 keeps
# the mock-driven regression gate fast, while CI uses the real backoff because
# hosted mirrors are rate limited and an immediate retry hits the same warm
# failure.
retry_delay=${LCCC_CI_APT_RETRY_DELAY:-25}
if [[ ! $retry_delay =~ ^[0-9]+$ ]]; then
    echo "ci_install_link_oracles: LCCC_CI_APT_RETRY_DELAY must be a non-negative integer, got '$retry_delay'" >&2
    exit 2
fi

# The availability predicate run_linker_tests.py applies: mold, plus either
# spelling of the lld driver.
oracles_ready() {
    command -v mold >/dev/null 2>&1 || return 1
    command -v ld.lld >/dev/null 2>&1 || command -v lld >/dev/null 2>&1 || return 1
}

# Retry one privileged command with linear backoff. Used for operations whose
# failure is genuinely transient (index fetch, deb download); never to replay
# an operation whose inputs are known to be wrong.
apt_retry() {
    local attempts=$1 rc=0 attempt
    shift
    for (( attempt = 1; attempt <= attempts; attempt++ )); do
        rc=0
        as_root "$@" || rc=$?
        if (( rc == 0 )); then
            return 0
        fi
        echo "ci_install_link_oracles: '$*' failed (exit $rc, attempt $attempt/$attempts)" >&2
        if (( attempt < attempts )); then
            sleep $(( attempt * retry_delay ))
        fi
    done
    return "$rc"
}

# Honour the same archive/index cache ci_install_gcc16.sh populates. Both
# oracles come from the distribution rather than a PPA, but sharing the
# directory means a warm runner resolves and installs them from cache without
# fetching anything, and a stale cache only ever costs the delta download.
archive_opts=()
index_opts=()
if [[ -n ${LCCC_CI_APT_ARCHIVE_DIR:-} ]]; then
    mkdir -p "${LCCC_CI_APT_ARCHIVE_DIR}/partial" \
             "${LCCC_CI_APT_ARCHIVE_DIR}/lists/partial"
    archive_opts=(
        -o "Dir::Cache::archives=${LCCC_CI_APT_ARCHIVE_DIR}"
        -o APT::Keep-Downloaded-Packages=true
    )
    index_opts=(-o "Dir::State::lists=${LCCC_CI_APT_ARCHIVE_DIR}/lists")
fi

# Exact package presence in the index apt will actually install from. The
# probe carries the same index options as the install path: with a cached
# index, a probe reading the default list directory reports "unresolvable"
# for a package apt can install offline.
package_known() {
    # Do not use grep -q here: with `set -o pipefail`, grep exits as soon as
    # it sees the exact package name and can SIGPIPE apt-cache while it is
    # still writing similarly-prefixed names (e.g. `mold`, then `molds`).
    # That turns a resolvable package into a false miss and exhausts retries.
    apt-cache "${index_opts[@]}" pkgnames "$1" 2>/dev/null | grep -Fx -- "$1" >/dev/null
}

missing_packages() {
    local package
    for package in "${packages[@]}"; do
        package_known "$package" || printf '%s\n' "$package"
    done
}

describe_missing() {
    local missing
    missing=$(missing_packages)
    printf '%s' "${missing//$'\n'/ }"
}

refresh_indexes() {
    apt_retry 3 apt-get update -o Acquire::Retries=3 -o Acquire::Languages=none \
        "${index_opts[@]}"
}

# Refresh until both names are resolvable. Keyed on resolvability rather than
# apt's exit status, so an update that "succeeds" while an index 503'd still
# retries instead of spending three install attempts on a name absent from the
# index.
ensure_packages_resolvable() {
    local rounds=$1 round
    if [[ -z $(missing_packages) ]]; then
        return 0
    fi
    for (( round = 1; round <= rounds; round++ )); do
        refresh_indexes || true
        if [[ -z $(missing_packages) ]]; then
            return 0
        fi
        echo "ci_install_link_oracles: unresolvable after index refresh $round/$rounds (missing: $(describe_missing))" >&2
        if (( round < rounds )); then
            sleep $(( round * retry_delay ))
        fi
    done
    return 1
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
        echo "ci_install_link_oracles: install failed (exit $rc, attempt $attempt/3)" >&2
        if (( attempt == 3 )); then
            break
        fi
        # Both apt failure families that survive Acquire::Retries are index
        # problems -- a repository index that 503'd mid-install, or a stale
        # list whose deb URLs rotated to 404. Both are fixed by a refresh and
        # neither by a plain replay, so refresh before spending another
        # attempt. This only costs time on the failure path.
        refresh_indexes || true
        sleep $(( attempt * retry_delay ))
        attempt=$(( attempt + 1 ))
    done
    return "$rc"
}

if oracles_ready; then
    echo "ci_install_link_oracles: mold and lld already present; skipping the network"
    exit 0
fi

if ! ensure_packages_resolvable 3; then
    echo "ci_install_link_oracles: cannot resolve '${packages[*]}' from any index" >&2
    exit 1
fi

if ! install_packages; then
    echo "ci_install_link_oracles: installation failed after every attempt" >&2
    exit 1
fi

# Fail closed on the real contract. An apt install that reports success while
# leaving a binary uninstalled must not become a silently degraded gate run:
# run_linker_tests.py skips an absent oracle rather than failing, so a half
# install would quietly narrow the differential coverage.
if ! oracles_ready; then
    echo "ci_install_link_oracles: apt reported success but the oracles are not on PATH" >&2
    exit 1
fi

mold --version
ld.lld --version 2>/dev/null || lld --version
echo "ci_install_link_oracles: differential linker oracles ready"
