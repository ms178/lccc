#!/usr/bin/env bash
# `MAX_EVAL_CALL_DEPTH`: the compile-time constant evaluator in `ipcp` must
# refuse a fold it cannot finish instead of exhausting the compiler's stack.
#
# The step budget alone did not bound *stack*: one recursion level costs one
# step, so a body that recurses per step reached hundreds of thousands of Rust
# frames under a 2,000,000-step budget and aborted the compiler with "thread
# '<unknown>' has overflowed its stack" (the pipeline runs on a 64 MiB stack).
# The program below nests far deeper than `ackermann(3, 11)` — the deepest fold
# the pass exists for, ~16.4k levels — while staying a legitimate runtime
# recursion, so the only acceptable outcomes are a clean compile and the
# arithmetically correct answer.
#
# Usage: CCC=target/fastbuild/lccc bash tests/regression/check_ipcp_eval_depth.sh
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
repo=$(cd "$(dirname "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-ipcpdepth.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail=0

bad() {
  printf 'FAIL: %s\n' "$*"
  fail=1
}

src="$repo/tests/regression/ipcp_eval_depth.c"

# 1. The compiler must finish. A nonzero status here is the historical
#    stack-overflow abort (SIGABRT/SIGSEGV), which `set -e` would otherwise
#    turn into a silent early exit.
if ! "$CCC" -O2 -o "$tmp/depth" "$src" 2>"$tmp/err"; then
  bad "compiling a 100k-deep constant recursion failed: $(head -2 "$tmp/err")"
else
  # 2. The answer must still be right: a wrong fold is worse than no fold.
  out=$("$tmp/depth")
  if [ "$out" != "100000" ]; then
    bad "deep(100000) produced '$out', expected 100000"
  fi
  # 3. The call must survive as a call — a fold that *did* run would have
  #    returned the same number, so the check above cannot see the difference.
  #    `MAX_EVAL_CALL_DEPTH` is the only reason it does not.
  if ! grep -qE '^\s+call deep$' <<<"$("$CCC" -O2 -S "$src" -o -)"; then
    bad "deep() was folded away: the depth bound did not refuse an unfinishable fold"
  fi
fi

if [ "$fail" = "0" ]; then
  printf '%s\n' "ipcp eval depth: 100k-deep constant recursion compiles and runs"
fi
exit "$fail"
