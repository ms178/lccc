#!/usr/bin/env bash
# i686 trivial-block threading must not drop a label whose address is in
# module data, and must not retarget a short-only branch across a relay.
#
# The static-label case is a later function, so its `.long .LBB` sits outside
# that function's peephole window. The short-only cases are toplevel asm with
# a 200-byte pad: the relay is in rel8 range and the ultimate target is not.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
src=$root/tests/regression/i686_label_ref_safety.c
tmp=${TMPDIR:-/tmp}/lccc-i686-label-ref.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
python3 "$root/scripts/check_asm_local_labels.py" --selftest

run_one() {
    local opt=$1
    "$CCC" -m32 -O"$opt" -Wall -Wextra -Werror "$src" -o "$tmp/t"
    "$CCC" -m32 -O"$opt" -S "$src" -o "$tmp/t.s"
    python3 "$root/scripts/check_asm_local_labels.py" "$tmp/t.s"
    python3 - "$tmp/t.s" <<'PY'
import re, sys
asm = open(sys.argv[1]).read()
for insn, relay in (
    ("jecxz", ".Ljecxz_relay_fwd"),
    ("jecxz", ".Ljecxz_relay_back"),
    ("jcxz", ".Ljcxz_relay_fwd"),
    ("loop", ".Lloop_relay_fwd"),
):
    if not re.search(rf"(?m)^\s*{insn}\s+{re.escape(relay)}\b", asm):
        sys.exit(f"{insn} no longer targets {relay}")
    if f"{relay}:" not in asm:
        sys.exit(f"missing relay {relay}")
PY
    local interp
    interp=$(readelf -l "$tmp/t" | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    if [[ -n $interp && ! -e $interp ]]; then
        echo "SKIP-RUN: -m32 -O$opt requires missing $interp"
        return
    fi
    set +e
    "$tmp/t"
    local rc=$?
    set -e
    if [[ $rc -ne 0 ]]; then
        echo "FAIL -m32 -O$opt exit=$rc" >&2
        exit "$rc"
    fi
    echo "PASS -m32 -O$opt"
}

run_one 0
run_one 1
run_one 2
run_one s
echo "PASS i686 label-ref safety"
