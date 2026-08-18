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
# argument 5, whose destination R9 holds argument 6.  The parallel-copy solver
# must therefore preserve values in this exact dependency order.
ordered = [r"movq\s+%r9,\s*%r11", r"movq\s+%r8,\s*%r9", r"movq\s+%rcx,\s*%r8"]
pos = -1
for move in ordered:
    m = re.search(move, mix)
    if not m or m.start() <= pos:
        raise SystemExit(f"mix6 missing/misordered parallel parameter move: {move}")
    pos = m.start()
PY
