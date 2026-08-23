#!/usr/bin/env bash
# Structural regression for scalar destructive-FMA destination coalescing.
# Runtime semantics are covered by fma_dest_coalesce.c; this check prevents a
# correct-but-slower return to the xmm0/xmm1 relay sequence.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out=$(mktemp "${TMPDIR:-/tmp}/lccc-fma-codegen.XXXXXX.s")
trap 'rm -f "$out"' EXIT
"$CCC" -O2 -march=x86-64-v3 -S "$dir/fma_dest_coalesce.c" -o "$out"
python3 - "$out" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()

def body(name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)

distance = body("distance3")
fmas = re.findall(r"^\s*vfmadd\w*\s+%xmm(\d+),\s*%xmm\1,\s*%xmm(\d+)\s*$", distance, re.M)
if len(fmas) != 2 or len({dest for _, dest in fmas}) != 1:
    raise SystemExit("distance3 must accumulate two squares directly into one XMM destination")
if re.search(r"^\s*vfmadd\w*.*,\s*%xmm0\s*$", distance, re.M):
    raise SystemExit("distance3 regressed to the xmm0 destructive-FMA scratch path")
if re.search(r"^\s*(?:pushq|popq|subq\s+\$\d+,\s*%rsp|addq\s+\$\d+,\s*%rsp)", distance, re.M):
    raise SystemExit("distance3 regressed to a callee-save or empty stack frame")
if re.search(r"^\s*movq\s+%(?:rdi|rsi),", distance, re.M):
    raise SystemExit("distance3 failed to retain pointer parameters in ABI registers")
mem_subs = re.findall(r"^\s*vsubsd\s+\d*\(%rsi\),\s*%xmm\d+,\s*%xmm\d+", distance, re.M)
if len(mem_subs) != 3:
    raise SystemExit("distance3 must fold all three single-use RHS loads into vsubsd")
# Only the ABI return copy should remain; the old path had seven XMM relays.
if len(re.findall(r"^\s*movsd\s+%xmm\d+,\s*%xmm\d+\s*$", distance, re.M)) != 1:
    raise SystemExit("distance3 contains unexpected register-to-register movsd relays")

# When an accumulator shares a home with a multiplicand, the destructive FMA
# must not clobber a multiplicand before both are read. Two correct shapes
# exist: (a) the guarded xmm0 fallback, or (b) the copy-then-accumulate form
# (copy both ABI sources to scratch registers, then vfmadd into a third).
# The check accepts either but rejects a bare FMA whose destination is an
# un-copied ABI source register.
for name in ("accumulator_alias_lhs", "accumulator_alias_rhs"):
    fn_body = body(name)  # NOTE: do NOT rebind `text` — body() reads it
    if not re.search(r"^\s*vfmadd\w*", fn_body, re.M):
        raise SystemExit(f"{name} no longer emits an FMA")
    fma = re.search(r"^\s*vfmadd\w*\s+%xmm(\d+),\s*%xmm(\d+),\s*%xmm(\d+)\s*$", fn_body, re.M)
    if fma:
        dest = int(fma.group(3))
        for src in (int(fma.group(1)), int(fma.group(2))):
            if src == dest and src in (0, 1):
                # FMA writes an ABI source register that was NOT copied out
                # first — the aliasing bug this fixture guards against.
                copies = re.findall(
                    r"^\s*movsd\s+%xmm[01],\s*%xmm\d+\s*$", fn_body, re.M
                )
                if not copies:
                    raise SystemExit(f"{name} destructive FMA clobbers an un-copied multiplicand")

# The ten-argument case puts two source parameters on the incoming stack while
# still requiring direct accumulation into the allocated (non-xmm0) result.
stack = body("stack_multiplicands")
if len(re.findall(r"^\s*movsd\s+\d+\(%rsp\),\s*%xmm\d+", stack, re.M)) < 2:
    raise SystemExit("stack_multiplicands no longer exercises stack FP parameters")
if not re.search(r"^\s*vfmadd\w*.*,\s*%xmm([2-9]|1[0-5])\s*$", stack, re.M):
    raise SystemExit("stack_multiplicands did not FMA directly into its allocated result")
PY
