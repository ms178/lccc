#!/usr/bin/env bash
# Structural regression: -Os must not run unrestricted post-structural inlining.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-os-postinline.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

"$CCC" -m32 -Os -S "$dir/os_postinline_size_policy.c" -o "$tmp/test.s"

# The medium helper should remain outlined and all three direct calls should
# survive.  Before the fix, the late unrestricted inliner removed the symbol
# and cloned its body into call_medium_three_times three times.
grep -Eq '^medium_helper:' "$tmp/test.s"
calls=$(grep -Ec '^[[:space:]]*call[lq]?[[:space:]]+medium_helper([[:space:]]|$)' "$tmp/test.s" || true)
if [ "$calls" -ne 3 ]; then
    echo "expected 3 outlined medium_helper calls at -Os, found $calls" >&2
    exit 1
fi
