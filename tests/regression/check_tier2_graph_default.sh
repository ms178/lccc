#!/usr/bin/env bash
# RA-23/Tier-2: production coloring is safe and reduces huft's frame.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}; root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd); t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
src="$root/tests/regression/huft_build_crash.c"
"$CCC" -O2 "$src" -o "$t/on"; "$t/on"
"$CCC" -O2 -S "$src" -o "$t/on.s"
CCC_NO_TIER2_GRAPH=1 "$CCC" -O2 -S "$src" -o "$t/off.s"
on=$(sed -n 's/.*subq $\([0-9][0-9]*\), %rsp.*/\1/p' "$t/on.s" | head -1)
off=$(sed -n 's/.*subq $\([0-9][0-9]*\), %rsp.*/\1/p' "$t/off.s" | head -1)
[ -n "$on" ] && [ -n "$off" ] && [ "$on" -lt "$off" ] || { echo "Tier-2 did not reduce huft frame: on=$on off=$off" >&2; exit 1; }
