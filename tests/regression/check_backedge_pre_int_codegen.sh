#!/usr/bin/env bash
# Structural/performance regression for integer Backedge PRE: the optimized
# recurrence should carry the previous latch square and emit one fewer imul in
# the hot loop than CCC_DISABLE_PASSES=bepre.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
src="$dir/backedge_pre_int_recurrence.c"
on=${TMPDIR:-/tmp}/backedge_pre_int.on.s
off=${TMPDIR:-/tmp}/backedge_pre_int.off.s
"$CCC" -O2 "$src" -S -o "$on"
CCC_DISABLE_PASSES=bepre "$CCC" -O2 "$src" -S -o "$off"
on_count=$(grep -Ec '\bimul' "$on" || true)
off_count=$(grep -Ec '\bimul' "$off" || true)
if [ "$on_count" -ge "$off_count" ]; then
  echo "expected Backedge PRE to reduce imul count: on=$on_count off=$off_count" >&2
  exit 1
fi
