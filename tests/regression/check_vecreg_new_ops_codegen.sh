#!/usr/bin/env bash
# RA-21: post-whitelist SSE2 operations must retain multi-use values in XMM
# homes, while the kill switch provides an exact stack-backed control.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/simd_vecreg_new_ops.c"
out=$(mktemp "${TMPDIR:-/tmp}/lccc-vecreg-new.XXXXXX.s")
control=$(mktemp "${TMPDIR:-/tmp}/lccc-vecreg-new-control.XXXXXX.s")
trap 'rm -f "$out" "$control"' EXIT

"$CCC" -O2 -msse4.1 -S "$src" -o "$out"
CCC_NO_VECREG=1 "$CCC" -O2 -msse4.1 -S "$src" -o "$control"

python3 - "$out" "$control" <<'PY'
import re
import sys


def body(path, name):
    text = open(path, encoding="utf-8").read()
    match = re.search(
        rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},",
        text,
    )
    if not match:
        raise SystemExit(f"{path}: missing assembly body for {name}")
    return match.group(1)


def instructions(text):
    return [
        line for line in text.splitlines()
        if line.startswith("    ") and not line.lstrip().startswith(".")
    ]


def vector_stack_accesses(text):
    return sum(
        bool(re.search(r"\b(?:v?movdqu(?:64)?|movdqa)\b.*\(%r(?:sp|bp)\)", line))
        for line in instructions(text)
    )


current = body(sys.argv[1], "sat_chain_store")
disabled = body(sys.argv[2], "sat_chain_store")
if not re.search(r"movdqa %xmm0, %xmm[3-7]", current):
    raise SystemExit("sat_chain_store: multi-use value did not receive an XMM home")
current_mem = vector_stack_accesses(current)
disabled_mem = vector_stack_accesses(disabled)
if current_mem + 4 > disabled_mem:
    raise SystemExit(
        "sat_chain_store: expected at least four fewer vector stack accesses "
        f"({current_mem} vs kill-switch {disabled_mem})"
    )
if len(instructions(current)) >= len(instructions(disabled)):
    raise SystemExit(
        "sat_chain_store: register-resident path did not reduce instruction count "
        f"({len(instructions(current))} vs {len(instructions(disabled))})"
    )
PY
