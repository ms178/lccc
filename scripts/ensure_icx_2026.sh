#!/usr/bin/env bash
# ensure_icx_2026.sh — provision the local Intel ICX 2026.1.1 oracle.
#
# Compiler Explorer's `cicxlatest` channel is ICX 2024.0.0 (probe:
# __INTEL_LLVM_COMPILER == 20240000), so the local 2026.1.1 compiler is the
# only way to measure the current Intel release. This script fetches the five
# Intel apt packages pinned below, verifies each by SHA-256, and extracts them
# under a private prefix (no toolkit-wide install, no root needed). The
# package set and digests come from the official apt index selection recorded
# in engineering/evidence/redteam99/icx-packages-selected.json (S99).
#
# Idempotent: an existing prefix is accepted only if the probe still reports
# ICX 2026.1.1 (__INTEL_LLVM_COMPILER == 20260101). The version label in the
# directory name is never trusted on its own.
#
# Usage: scripts/ensure_icx_2026.sh [PREFIX]
#   PREFIX defaults to $LCCC_ICX_PREFIX or $HOME/.cache/intel-icx-2026.1.1.
#   Prints the absolute path of the verified icx binary on stdout.
set -euo pipefail

prefix=${1:-${LCCC_ICX_PREFIX:-${HOME}/.cache/intel-icx-2026.1.1}}
mirror=${LCCC_ICX_MIRROR:-https://apt.repos.intel.com/oneapi}
dl=${LCCC_ICX_DL:-${prefix}.downloads}

# filename | sha256
pins=(
    "intel-oneapi-compiler-shared-runtime-2026.1-2026.1.1-325_amd64.deb|c0c77700243af0413f7fbefd26649943c4959171ead91b6ace36ef0c8dfb2630"
    "intel-oneapi-compiler-shared-2026.1-2026.1.1-325_amd64.deb|87039478855d1b7ce08c5a30456e360158cdf45a6fb6a791083651ec340c5e99"
    "intel-oneapi-dpcpp-cpp-2026.1-2026.1.1-325_amd64.deb|fa97aac3f6ef43b0966f736dc47ebd4c97bddc709c80cba0fec0e1a1f36f9662"
    "intel-oneapi-compiler-dpcpp-cpp-common-2026.1-2026.1.1-325_all.deb|d26bef9048fd2f9ff9cc8a0443f41766ef3ff0682d6847f3d60f496f3fdd4d26"
    "intel-oneapi-compiler-shared-common-2026.1-2026.1.1-325_all.deb|283b675d295c9da38902edbc936980cde6b85d835480269f87559e5c59236591"
)

probe_icx() {
    # $1 = compiler binary. Prints the __INTEL_LLVM_COMPILER value, or nothing.
    local bin=$1 out
    out=$(printf '__INTEL_LLVM_COMPILER\n' | "$bin" -E -x c - -dM 2>/dev/null |
        awk '$2=="__INTEL_LLVM_COMPILER"{print $3}') || true
    printf '%s' "$out"
}

find_icx() {
    find "$1" -type f -name icx -path '*/bin/*' 2>/dev/null | head -n1
}

if [[ -d $prefix ]]; then
    bin=$(find_icx "$prefix")
    if [[ -n $bin && -x $bin && $(probe_icx "$bin") == 20260101 ]]; then
        echo "$bin"
        exit 0
    fi
    echo "error: $prefix exists but does not probe as ICX 2026.1.1; remove it and rerun" >&2
    exit 1
fi

mkdir -p "$dl"
for pin in "${pins[@]}"; do
    name=${pin%%|*}
    want=${pin##*|}
    file="$dl/$name"
    if [[ ! -s $file || $(sha256sum "$file" | cut -d' ' -f1) != "$want" ]]; then
        curl --proto '=https' --tlsv1.2 -fL --retry 3 -o "$file.part" "$mirror/pool/main/$name"
        got=$(sha256sum "$file.part" | cut -d' ' -f1)
        if [[ $got != "$want" ]]; then
            echo "error: SHA-256 mismatch for $name: got $got, want $want" >&2
            rm -f "$file.part"
            exit 1
        fi
        mv "$file.part" "$file"
    fi
done

staging="${prefix}.partial.$$"
rm -rf "$staging"
mkdir -p "$staging"
for pin in "${pins[@]}"; do
    dpkg-deb -x "$dl/${pin%%|*}" "$staging"
done
mv "$staging" "$prefix"

bin=$(find_icx "$prefix")
if [[ -z $bin || $(probe_icx "$bin") != 20260101 ]]; then
    echo "error: extracted compiler does not probe as ICX 2026.1.1" >&2
    exit 1
fi
echo "$bin"
