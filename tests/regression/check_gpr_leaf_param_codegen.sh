#!/usr/bin/env bash
# Structural regression for caller-saved homes in one-block leaf functions.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out=$(mktemp "${TMPDIR:-/tmp}/lccc-gpr-param.XXXXXX.s")
trap 'rm -f "$out"' EXIT
"$CCC" -O2 -S "$dir/gpr_param_caller_saved.c" -o "$out"
python3 - "$out" <<'PY'
import re
import sys
text = open(sys.argv[1], encoding="utf-8").read()

def body(name):
    m = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not m:
        raise SystemExit(f"missing {name} assembly")
    return m.group(1)

for name in ("mix6", "pointer_mix"):
    asm = body(name)
    if re.search(r"%\b(?:rbx|r12|r13|r14|r15)\b", asm):
        raise SystemExit(f"{name} assigned a call-free parameter to a callee-saved register")
    if re.search(r"^\s*(?:pushq|popq)\s", asm, re.M):
        raise SystemExit(f"{name} retained callee-save push/pop overhead")

mix = body("mix6")
if re.search(r"\d+\(%rsp\)", mix) or re.search(r"^\s*(?:subq|addq).*%rsp", mix, re.M):
    raise SystemExit("mix6 regressed to stack-homing call-free parameters")
# RCX cannot itself be allocated (backend scratch).  Its destination R8 holds
# argument 5, whose destination R9 holds argument 6.  When the parallel-copy
# solver emits the shuffle, it must preserve values in this exact dependency
# order.
#
# The peephole copy-coalescer (copy_coalesce.rs) can retire the whole shuffle by
# renaming each home back onto the ABI register the parameter arrived in, which
# is strictly better code.  Accept that outcome, but if ANY of the moves is
# still emitted, all of them must be present and correctly ordered — a partial
# shuffle would mean a value was clobbered before its copy.
ordered = [r"movq\s+%r9,\s*%r11", r"movq\s+%r8,\s*%r9", r"movq\s+%rcx,\s*%r8"]
present = [bool(re.search(move, mix)) for move in ordered]
if any(present):
    if not all(present):
        raise SystemExit(f"mix6 emitted a partial parallel parameter shuffle: {present}")
    pos = -1
    for move in ordered:
        m = re.search(move, mix)
        if m.start() <= pos:
            raise SystemExit(f"mix6 misordered parallel parameter move: {move}")
        pos = m.start()
else:
    # Fully coalesced: the parameters must still be consumed from their ABI
    # registers, and the result must come back in %rax.
    for reg in ("%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"):
        if reg not in mix:
            raise SystemExit(f"mix6 coalesced away a parameter register: {reg}")
    if not re.search(r"movq\s+%\w+,\s*%rax", mix):
        raise SystemExit("mix6 does not return its result in %rax")
PY
