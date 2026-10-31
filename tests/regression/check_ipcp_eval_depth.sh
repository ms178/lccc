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
# recursion. A small constant call positively proves that IPCP reached and
# evaluated this function; the larger constant call must be refused.
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
if ! CCC_DEBUG_IPCP_EVAL=1 "$CCC" -O2 -o "$tmp/depth" "$src" 2>"$tmp/err"; then
  bad "compiling a 100k-deep constant recursion failed: $(tail -3 "$tmp/err")"
else
  # Positive control: a real constant call reached the evaluator and folded.
  if ! grep -qE "\[IPCP-EVAL\] deep -> .*I32\(100\)" "$tmp/err"; then
    bad "small constant recursion did not reach and fold in IPCP"
  fi
  # 2. The answer must still be right: a wrong fold is worse than no fold.
  out=$("$tmp/depth")
  if [ "$out" != "100 100000" ]; then
    bad "constant recursion produced '$out', expected 100 100000"
  fi
  # 3. The call must survive as a call — a fold that *did* run would have
  #    returned the same number, so the check above cannot see the difference.
  #    `MAX_EVAL_CALL_DEPTH` is the only reason it does not.
  if ! "$CCC" -O2 -S "$src" -o "$tmp/depth.s"; then
    bad "could not compile the assembly control"
  else
    awk '/^main:/{in_main=1;next} /^\.size main,/{in_main=0} in_main' "$tmp/depth.s" > "$tmp/main.s"
    if ! grep -qE '^[[:space:]]+call deep$' "$tmp/main.s"; then
      bad "main no longer calls deep: the large constant call was folded away"
    fi
  fi
fi

if [ "$fail" = "0" ]; then
  printf '%s\n' "ipcp eval depth: small constant call folds; 100k-deep constant call survives and runs"
fi
exit "$fail"
