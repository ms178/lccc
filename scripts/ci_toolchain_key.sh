#!/usr/bin/env bash
# The identity of a provisioned-toolchain bundle: the package baseline it is a
# delta against, and the installer that produced it.
#
# WHY THIS EXISTS
# ---------------
# `ci_toolchain_bundle.sh` stores the files of one apt transaction and
# `restore` unpacks them over `/`.  Such a delta is valid on exactly the
# package baseline it was computed against.  The cache key that selects it
# said `runner.os` -- the constant `Linux` -- where its own comment promised
# "one bundle per (runner image, installer) pair", so the first bundle ever
# built was handed to every later image as an exact hit.
#
# Run 44345167051 (2026-11-18) is what that costs.  Runner image
# 20261116.133.1 ships glibc 2.39-0ubuntu8.12; the cached bundle, built on
# 2026-11-06, carries the `libc6-dbg` of the image it was built on.  valgrind
# needs the debug symbols of the loader that is actually running, so the
# Memcheck probe failed, the lane fell back to apt, and the AVX-512VL lane --
# one of two without a package archive -- fetched 319 MB from the Launchpad PPA
# at 292 kB/s:
#
#     restored 3064 file(s) into /
#     valgrind cannot run Memcheck (missing libc6-dbg for the loader, ...)
#     cached bundle failed verification; installing from packages
#     Fetched 319 MB in 18min 10s (292 kB/s)
#     toolchain provisioned in 1733.4s          <- of a 30-minute job
#
# Five of thirty provisioning jobs took that path across two runs, and none of
# them could repair it: `actions/cache` never rewrites an entry whose key
# already exists, so the rebuilt bundle was thrown away each time.
#
# WHAT THE KEY CONTAINS
# ---------------------
#   * The dpkg architecture and every installed package with its version.  A
#     delta's validity is a property of the whole baseline, not of the one
#     package that happened to break it this time; keying on glibc alone would
#     restore the same failure for the next package that moves.
#   * The installer scripts, this one included: a different installer produces
#     a different delta from the same baseline.
#
# The digest is preceded by the image's own name where the runner publishes
# one (`ImageOS`/`ImageVersion`), so a cache listing can be read by a person.
# It must be computed before anything in the job installs a package, and
# `ci_provision_toolchain.sh` stamps every bundle it exports with it: the
# stamp, not the cache action's hit/miss flag, is what decides whether an
# archive may be unpacked.
#
# Usage:
#   ci_toolchain_key.sh                   print the key
#   ci_toolchain_key.sh --github-output   print `key=<key>` for $GITHUB_OUTPUT
set -euo pipefail

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# A different installer produces a different delta.  Listed, not globbed: the
# identity must not change because an unrelated script was added next to these.
INSTALLERS=(
    ci_install_gcc16.sh
    ci_install_link_oracles.sh
    ci_provision_toolchain.sh
    ci_toolchain_bundle.sh
    ci_toolchain_key.sh
)

FORMAT=plain
case ${1:-} in
    "") ;;
    --github-output) FORMAT=github ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "ci_toolchain_key: unknown argument: $1" >&2; exit 2 ;;
esac
(( $# <= 1 )) || { echo "ci_toolchain_key: unexpected argument: $2" >&2; exit 2; }

# The package baseline.  A host without dpkg has no package delta to key, so
# its baseline is the release it reports; the bundle is still stamped and the
# stamp still has to match.
baseline() {
    if command -v dpkg-query >/dev/null 2>&1; then
        dpkg --print-architecture 2>/dev/null || uname -m
        dpkg-query -W -f='${binary:Package} ${Version}\n' | LC_ALL=C sort
    else
        uname -m
        cat /etc/os-release 2>/dev/null || uname -sr
    fi
}

installers() {
    local name
    for name in "${INSTALLERS[@]}"; do
        printf '== %s\n' "$name"
        # A missing installer is part of the identity too, not an error here:
        # the provisioner that needs it fails by name when it runs.
        cat "$HERE/$name" 2>/dev/null || printf 'absent\n'
    done
}

digest=$({ baseline; installers; } | sha256sum)
digest=${digest:0:20}

# Human-readable prefix; never trusted, only displayed.
label=${ImageOS:-}
if [[ -z $label && -r /etc/os-release ]]; then
    label=$(. /etc/os-release && printf '%s%s' "${ID:-linux}" "${VERSION_ID:-}")
fi
label=${label:-linux}${ImageVersion:+-$ImageVersion}
label=${label//[^A-Za-z0-9._-]/_}

key=$label-$digest
if [[ $FORMAT == github ]]; then
    printf 'key=%s\n' "$key"
else
    printf '%s\n' "$key"
fi
