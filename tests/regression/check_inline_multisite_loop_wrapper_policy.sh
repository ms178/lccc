#!/usr/bin/env bash
# Structural OP-26 regression: the wrapper is intentionally retained, while
# its single-call-site loop kernel may still inline into it.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-inline-wrapper.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
"$CCC" -O2 -S "$dir/inline_multisite_loop_wrapper.c" -o "$tmp/test.s"
grep -Eq '^wrapper:' "$tmp/test.s"
calls=$(grep -Ec '^[[:space:]]*call[q]?[[:space:]]+wrapper([[:space:]]|$)' "$tmp/test.s" || true)
if [[ "$calls" -ne 2 ]]; then
    echo "expected two outlined wrapper calls at -O2, found $calls" >&2
    exit 1
fi
