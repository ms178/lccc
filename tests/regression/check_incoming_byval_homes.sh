#!/usr/bin/env bash
# Incoming x86-64 MEMORY objects: by-value identity, mutation/escape, sret,
# packed fields, GP exhaustion, varargs, alloca and outgoing realignment.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
if [[ -n ${ORACLE_CC:-} ]]; then
    oracle=$ORACLE_CC
else
    # Default lane is pinned GCC 16. Explicit vendor/Clang lanes need no
    # unrelated GCC install and never silently fall back to another oracle.
    . "$root/tests/regression/lib/gcc_oracle.sh"
    require_gcc_oracle check_incoming_byval_homes
    oracle=$GCC_ORACLE_BIN
fi
command -v "$oracle" >/dev/null
fixture=$root/tests/regression/fixtures/incoming-byval
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for opt in -O0 -O2; do
    for frame in -fomit-frame-pointer -fno-omit-frame-pointer; do
        for disabled in 0 1; do
            if ((disabled)); then export CCC_NO_INCOMING_BYVAL_HOME=1
            else unset CCC_NO_INCOMING_BYVAL_HOME; fi
            for unit in callee driver; do
                "$ccc" "$opt" "$frame" -c "$fixture/$unit.c" -o "$tmp/lccc-$unit.o"
                "$oracle" "$opt" "$frame" -c "$fixture/$unit.c" -o "$tmp/oracle-$unit.o"
            done
            for callee in lccc oracle; do
                for driver in lccc oracle; do
                    "$oracle" "$tmp/$callee-callee.o" "$tmp/$driver-driver.o" -o "$tmp/run"
                    "$tmp/run"
                done
            done
            echo "ok incoming-byval: $opt $frame disabled=$disabled ($oracle; four pairings)"
        done
    done
done
unset CCC_NO_INCOMING_BYVAL_HOME
"$ccc" -O2 -S "$fixture/callee.c" -o "$tmp/on.s"
CCC_NO_INCOMING_BYVAL_HOME=1 "$ccc" -O2 -S "$fixture/callee.c" -o "$tmp/off.s"
python3 - "$tmp/on.s" "$tmp/off.s" <<'PY'
import re
import sys
from pathlib import Path

def count(path):
    text = Path(path).read_text()
    match = re.search(r'^sum_big:\n(.*?)(?=^\s*\.size\s+sum_big\b|^\s*\.globl|\Z)',
                      text, re.M | re.S)
    if not match:
        raise SystemExit('FAIL: missing sum_big assembly')
    return sum(bool(re.match(r'^\s+[a-z][a-z0-9]*(?:\s|$)', line))
               for line in match[1].splitlines())
on, off = map(count, sys.argv[1:])
if not on < off:
    raise SystemExit(f'FAIL: incoming capture not eliminated: on={on}, off={off}')
print(f'ok incoming-byval: sum_big instructions {off} -> {on}')
PY
