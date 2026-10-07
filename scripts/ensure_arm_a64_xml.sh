#!/usr/bin/env bash
# ensure_arm_a64_xml.sh — (re)provision the pinned Arm A64 ISA XML package
# (release 2026-09, the machine-readable companion of ARM DDI 0487 M.d)
# for scripts/aarch64_doccheck_2026_09.py.
#
# The doccheck gate NAKs every release that is not 2026-09 — that is the
# "validate against the latest September-2026 documentation" law made
# executable — so the provisioner must supply exactly that package and
# nothing else may satisfy its fast path:
#
#   * the tarball bytes are pinned by SHA-256 below (Arm exploration
#     packages are immutable per release; the digest was taken from the
#     developer.arm.com permalink the first time this tree fetched it),
#   * a pre-placed tarball (ARM_XML_DL_DIR, default $HOME/dl — the same
#     channel ensure_gas_247.sh uses) must hash to the same digest, so a
#     doctored or truncated archive is a failed provision, not a wrong
#     oracle, and
#   * only the `ISA_A64_xml_A_profile-2026-09_md/` member is extracted:
#     the archive also carries the superseded 2026-06 package and two
#     PDFs, and a tree containing the wrong release must never satisfy
#     the fast path (the manifest is re-read and re-checked after every
#     extraction).
#
# Usage: scripts/ensure_arm_a64_xml.sh [install-prefix]
#   install-prefix defaults to $HOME/.cache/arm-isa-a64-2026-09; the
#   extract lands at <prefix>/ISA_A64_xml_A_profile-2026-09_md with its
#   build-manifest.json. The result is printed on success.
set -euo pipefail

ARM_XML_RELEASE="2026-09"
ARM_XML_DIRNAME="ISA_A64_xml_A_profile-2026-09_md"
ARM_XML_TARBALL_SHA256="62917cba0da668deb6c28005231f4b94d1044cd52dbe4a4729fcc26d1642d827"
ARM_XML_URL="https://developer.arm.com/-/cdn-downloads/permalink/Exploration-Tools-A64-ISA/ISA_A64/ISA_A64_xml_A_profile-2026-09_md.tar.gz"

prefix="${1:-${HOME}/.cache/arm-isa-a64-2026-09}"
dl_dir="${ARM_XML_DL_DIR:-${HOME}/dl}"
tarball="$dl_dir/$ARM_XML_DIRNAME.tar.gz"
install_dir="$prefix/$ARM_XML_DIRNAME"
manifest="$install_dir/build-manifest.json"

_sha256_is() {  # _sha256_is <file> <expected-hex>: whole-digest equality
    local got
    got=$(sha256sum "$1" 2>/dev/null | awk '{print $1}') || return 1
    [[ "$got" == "$2" ]]
}

_release_ok() {  # _release_ok <manifest>: declares the pinned release
    [[ -f "$1" ]] || return 1
    python3 - "$1" "$ARM_XML_RELEASE" <<'PY'
import json
import sys

try:
    data = json.load(open(sys.argv[1]))
except (OSError, json.JSONDecodeError):
    sys.exit(1)
rel = data.get("artifact", {}).get("release_version", "")
sys.exit(0 if sys.argv[2] in rel else 1)
PY
}

# Fast path: a warm extract whose manifest still declares the pin.
if _release_ok "$manifest"; then
    echo "arm-a64-xml: $install_dir present (release $ARM_XML_RELEASE, fast path)"
    exit 0
fi

# Download (or adopt a pre-placed tarball) and verify the pin.  curl's -f
# turns an HTML error page into a failure so a CDN outage cannot land a
# non-archive in the cache.
if [[ ! -f "$tarball" ]]; then
    mkdir -p "$dl_dir"
    echo "arm-a64-xml: fetching $ARM_XML_URL" >&2
    curl -fsSL --retry 3 --retry-delay 2 -o "$tarball.part" "$ARM_XML_URL"
    mv "$tarball.part" "$tarball"
fi
if ! _sha256_is "$tarball" "$ARM_XML_TARBALL_SHA256"; then
    echo "arm-a64-xml: FATAL: $tarball does not match the pinned SHA-256" \
         "($ARM_XML_TARBALL_SHA256); remove it to refetch" >&2
    exit 1
fi

# Extract only the pinned member into a staging dir, validate, then move
# into place — an interrupted extraction must not leave a half-tree that
# the next run's fast path would accept.
staging="$prefix/.staging.$$"
rm -rf "$staging"
mkdir -p "$staging"
trap 'rm -rf "$staging"' EXIT
tar -xzf "$tarball" -C "$staging" "$ARM_XML_DIRNAME"
if ! _release_ok "$staging/$ARM_XML_DIRNAME/build-manifest.json"; then
    echo "arm-a64-xml: FATAL: extracted manifest does not declare" \
         "$ARM_XML_RELEASE" >&2
    exit 1
fi
mkdir -p "$prefix"
rm -rf "$install_dir"
mv "$staging/$ARM_XML_DIRNAME" "$install_dir"
rm -rf "$staging"
trap - EXIT

_release_ok "$manifest" || {
    echo "arm-a64-xml: FATAL: post-install manifest check failed" >&2
    exit 1
}
echo "arm-a64-xml: installed $install_dir (release $ARM_XML_RELEASE)"
