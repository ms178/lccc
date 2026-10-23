#!/usr/bin/env bash
# Structural regression for scalar destructive-FMA destination coalescing.
# Runtime semantics are covered by fma_dest_coalesce.c; this check prevents a
# correct-but-slower return to the xmm0/xmm1 relay sequence.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out=$(mktemp "${TMPDIR:-/tmp}/lccc-fma-codegen.XXXXXX.s")
trap 'rm -f "$out"' EXIT
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=fast -S "$dir/fma_dest_coalesce.c" -o "$out"
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
# Count every register-to-register XMM move spelling: the full-register
# `(v)movapd` (dependency-free, eliminated at rename) replaced the merging
# `movsd %src, %dst`, and a relay must not hide behind either name.
relays = re.findall(
    r"^\s*v?mov(?:s[sd]|ap[sd])\s+%xmm\d+,\s*%xmm(\d+)\s*$", distance, re.M
)
if len(relays) != 1 or relays[0] != "0":
    raise SystemExit(
        f"distance3 contains unexpected register-to-register XMM relays: {relays}"
    )
if re.search(r"^\s*movs[sd]\s+%xmm\d+,\s*%xmm\d+\s*$", distance, re.M):
    raise SystemExit("distance3 return copy is a merging movsd (false dependency)")

# When an accumulator shares a home with a multiplicand, the destructive FMA
# must not clobber a multiplicand before every read of it. A single VEX FMA
# reads all three operands before it writes, so `vfmadd231sd %xmm1, %xmm0,
# %xmm0` (xmm0 = xmm0*xmm1 + xmm0 = x*y + x) is the optimal one-instruction
# body, not a hazard; the hazard is an EARLIER write of a register a later
# instruction still reads as the original parameter. So the body is executed
# symbolically (register moves and the three VEX FMA operand orders; any other
# instruction touching an XMM register fails closed) and the returned %xmm0
# must be exactly the source expression. One FMA and no relay is required.
MOVES = ("movsd", "movapd", "vmovsd", "vmovapd", "movaps", "vmovaps")
def returned_value(name, fn_body, regs, memory=None):
    """Symbolically execute a straight-line scalar-double body.

    `regs` seeds the incoming XMM parameters, `memory` the incoming stack
    slots (`N(%rsp)` -> symbol). Register moves, stack loads, and the three
    VEX FMA operand orders (register or stack third operand) are modelled;
    any other instruction mentioning an XMM register or an unmodelled memory
    operand fails closed rather than being guessed at.
    """
    regs = dict(regs)
    memory = memory or {}
    fmas = 0
    def read(operand):
        if operand.startswith("%xmm"):
            return regs.get(operand[1:])
        if operand in memory:
            return memory[operand]
        raise SystemExit(f"{name}: unmodelled operand {operand!r}")
    for raw in fn_body.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line or line.startswith(".") or line == "ret":
            continue
        op, _, rest = line.partition(" ")
        args = [a.strip() for a in rest.split(",")] if rest else []
        if op in MOVES and len(args) == 2 and args[1].startswith("%xmm"):
            regs[args[1][1:]] = read(args[0])
        elif op.startswith("vfmadd") and op.endswith("sd") and len(args) == 3:
            a, b, c = (read(r) for r in args)  # AT&T: op3, op2, op1(dest)
            form = op[len("vfmadd"):-2]
            prod, add = {"132": ((c, a), b), "213": ((b, c), a), "231": ((b, a), c)}[form]
            regs[args[2][1:]] = ("fma", frozenset(prod) if prod[0] != prod[1] else prod, add)
            fmas += 1
        elif "xmm" in line or "(%rsp)" in line:
            raise SystemExit(f"{name}: unmodelled instruction {line!r}; extend the model")
    if fmas != 1:
        raise SystemExit(f"{name}: expected exactly one FMA, found {fmas}")
    return regs.get("xmm0")
RELAY = r"^\s*v?mov(?:sd|apd|aps)\s+%xmm\d+,\s*%xmm\d+\s*$"
for name, addend in (("accumulator_alias_lhs", "x"), ("accumulator_alias_rhs", "y")):
    got = returned_value(name, body(name), {"xmm0": "x", "xmm1": "y"})
    want = ("fma", frozenset(("x", "y")), addend)
    if got != want:
        raise SystemExit(f"{name}: returned {got!r}, want x*y + {addend} (multiplicand clobbered)")
    relays = re.findall(RELAY, body(name), re.M)
    if relays:
        raise SystemExit(f"{name}: {len(relays)} XMM relay(s) around a single destructive FMA")

# The ten-argument case passes the two multiplicands on the incoming stack
# (8(%rsp), 16(%rsp)) and the accumulator in %xmm0, the return register.
# GCC 16.2 and Clang 23 emit `vmovsd 8(%rsp), %xmm1; vfmadd231sd 16(%rsp),
# %xmm1, %xmm0; ret`. Required: the result is exactly acc + x*y, both stack
# reads stay in the SSE domain (no GPR bounce such as `movq 16(%rsp), %rax;
# movq %rax, %xmm4`, the former last-stack-FP-parameter path), and no relay.
stack = body("stack_multiplicands")
if re.search(r"^\s*mov[lq]\s+\d+\(%rsp\),\s*%[re]?[a-z0-9]+\s*$", stack, re.M):
    raise SystemExit("stack_multiplicands bounces a stack FP parameter through a GPR")
got = returned_value("stack_multiplicands", stack, {"xmm0": "acc"},
                     {"8(%rsp)": "x", "16(%rsp)": "y"})
if got != ("fma", frozenset(("x", "y")), "acc"):
    raise SystemExit(f"stack_multiplicands returned {got!r}, want acc + x*y")
if re.findall(RELAY, stack, re.M):
    raise SystemExit("stack_multiplicands contains an XMM relay")
PY

echo "fma dest coalesce codegen: PASS (alias bodies and stack multiplicands proven symbolically; no GPR bounce, no relay)"
