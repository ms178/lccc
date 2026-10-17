#!/usr/bin/env bash
# Provision the latest available ICX 2026.2.x build from Intel's oneAPI APT repo.
# The isolated APT client authenticates metadata; SHA-256 verifies each archive.
# A manifest freezes the selected versions/digests in a private, rootless prefix.
# Existing prefixes are validated snapshots. Use a NEW prefix to refresh one.
# Requires python3, apt-get, apt-cache, dpkg-deb and APT's HTTPS/signature support.
# Usage: scripts/ensure_icx_2026.sh [PREFIX]
# Default: $LCCC_ICX_PREFIX or $HOME/.cache/intel-icx-2026.2
# Stdout is only the absolute path of the verified icx binary.
set -euo pipefail
exec python3 "$(dirname "${BASH_SOURCE[0]}")/icx_provision.py" "$@"
