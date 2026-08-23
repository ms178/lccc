#!/usr/bin/env bash
# FP Backedge PRE should fire by default for multi-use FP top expressions, but
# not for the Mandelbrot-style singly-used square that is known to regress on
# x86-64 due to loop-carried FP pressure.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fp_src="$dir/backedge_pre_fp_multiuse.c"
fp_on=${TMPDIR:-/tmp}/backedge_pre_fp.on.s
fp_off=${TMPDIR:-/tmp}/backedge_pre_fp.off.s
"$CCC" -O2 "$fp_src" -S -o "$fp_on"
CCC_DISABLE_PASSES=bepre "$CCC" -O2 "$fp_src" -S -o "$fp_off"
on_count=$(grep -Ec '\bvmulsd\b|\bmulsd\b' "$fp_on" || true)
off_count=$(grep -Ec '\bvmulsd\b|\bmulsd\b' "$fp_off" || true)
if [ "$on_count" -ge "$off_count" ]; then
  echo "expected FP multi-use Backedge PRE to reduce mulsd count: on=$on_count off=$off_count" >&2
  exit 1
fi
mandel="$(CDPATH= cd -- "$dir/../benchmark/programs" && pwd)/mandelbrot.c"
if [ -f "$mandel" ]; then
  m_on=${TMPDIR:-/tmp}/backedge_pre_mandel.on.s
  m_off=${TMPDIR:-/tmp}/backedge_pre_mandel.off.s
  "$CCC" -O2 "$mandel" -S -o "$m_on"
  CCC_DISABLE_PASSES=bepre "$CCC" -O2 "$mandel" -S -o "$m_off"
  if ! cmp -s "$m_on" "$m_off"; then
    echo "default FP Backedge PRE must not perturb Mandelbrot guardrail" >&2
    exit 1
  fi
fi
