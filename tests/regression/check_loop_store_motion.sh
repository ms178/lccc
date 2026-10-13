#!/usr/bin/env bash
# Loop store motion gate (loop_memory_promote).
#
# loop_memory_promote moves a loop-invariant load/store pair out of a loop: the
# loop keeps a register and the final value is stored once after the loop. It
# must do so only where the program cannot observe the difference.
#
# Checks on tests/regression/loop_store_motion_last_value.c:
#
#   1. Behaviour: lccc -O0 and -O2 print exactly what gcc -O0 prints. GCC is
#      the oracle; the file covers zero-trip paths, read-only objects, aliasing
#      loads, calls and volatile stores.
#   2. Placement, from the assembly. Positive control: the do-while accumulator
#      is promoted, so its store is after the loop. Negative controls: the
#      conditional, call, aliasing-load, volatile and store-only loops keep their
#      store inside the loop. A store is "inside" when its basic block lies on a
#      cycle of the control-flow graph, so layout does not matter.
#
# Known limit, not asserted here: a for-loop that exits from its header is not
# promoted through a pointer parameter, because the speculation proof needs the
# access to dominate the exit. The store-only global loop is also not sunk.
#
# Skips (exit 0, with a message) on non-x86-64 hosts: the placement check reads
# x86-64 assembly.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
src=tests/regression/loop_store_motion_last_value.c

if [ "$(uname -m)" != "x86_64" ] || [ "$(uname -s)" != "Linux" ]; then
    echo "SKIP: loop store motion placement check needs x86-64 Linux assembly"
    exit 0
fi
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_loop_store_motion" || exit 1

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-loop-store-motion.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail=0

"$GCC_ORACLE_BIN" -O0 -w -o "$tmp/oracle" "$src"
"$tmp/oracle" > "$tmp/oracle.out"

for lvl in -O0 -O2; do
    "$CCC" "$lvl" -w -o "$tmp/run$lvl" "$src"
    "$tmp/run$lvl" > "$tmp/run$lvl.out"
    if ! diff -u "$tmp/oracle.out" "$tmp/run$lvl.out" > "$tmp/run$lvl.diff"; then
        echo "FAIL: lccc $lvl output differs from gcc -O0"
        cat "$tmp/run$lvl.diff"
        fail=1
    fi
done

"$CCC" -O2 -S -o "$tmp/run.s" "$src"

cat > "$tmp/placement.py" <<'PY'
import re, sys

LABEL = re.compile(r'^(\.?[A-Za-z_][\w.$]*):\s*$')
JUMP = re.compile(r'^\s*(j\w*)\s+(\.?[A-Za-z_][\w.$]*)\s*$')
UNCOND = re.compile(r'^\s*jmp\s+(\.?[A-Za-z_][\w.$]*)\s*$')
RET = re.compile(r'^\s*ret\b')

def function_body(lines, name):
    start = None
    for i, line in enumerate(lines):
        if re.match(rf'^{re.escape(name)}:\s*$', line):
            start = i
            break
    if start is None:
        return None
    for j in range(start + 1, len(lines)):
        if re.match(r'^\s*\.size\s+' + re.escape(name) + r',', lines[j]):
            return lines[start:j]
    return lines[start:]

def blocks_of(body):
    blocks = [('__entry', [])]
    for line in body:
        s = line.strip()
        if not s or s.startswith('#'):
            continue
        m = LABEL.match(s)
        if m:
            blocks.append((m.group(1), []))
            continue
        if s.startswith('.'):
            continue
        blocks[-1][1].append(s)
    return blocks

def successors(blocks):
    index = {lab: i for i, (lab, _) in enumerate(blocks)}
    succ = {i: set() for i in range(len(blocks))}
    for i, (_, ins) in enumerate(blocks):
        last = ins[-1] if ins else ''
        if RET.match(last):
            continue
        m = UNCOND.match(last)
        if m:
            if m.group(1) in index:
                succ[i].add(index[m.group(1)])
            continue
        m = JUMP.match(last)
        if m and m.group(2) in index:
            succ[i].add(index[m.group(2)])
        if i + 1 < len(blocks):
            succ[i].add(i + 1)
    return succ

def on_cycle(succ, b):
    stack = list(succ[b])
    seen = set()
    while stack:
        n = stack.pop()
        if n == b:
            return True
        if n in seen:
            continue
        seen.add(n)
        stack.extend(succ[n])
    return False

path, name, expect, target = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
lines = open(path).read().splitlines()
body = function_body(lines, name)
if body is None:
    print(f"MISSING {name}")
    sys.exit(2)
blocks = blocks_of(body)
succ = successors(blocks)
store = re.compile(r'^mov[lq]\s+%\w+,\s*' + target + r'$')
hits = [i for i, (_, ins) in enumerate(blocks) if any(store.match(l) for l in ins)]
if not hits:
    print(f"NOSTORE {name} {target}")
    sys.exit(1)
inside = any(on_cycle(succ, i) for i in hits)
want = expect == 'inside'
print(f"{'OK' if inside == want else 'WRONG'} {name} store {'inside' if inside else 'outside'} loop (expected {expect})")
sys.exit(0 if inside == want else 1)
PY

check() {
    python3 "$tmp/placement.py" "$tmp/run.s" "$@" || fail=1
}

check acc_dowhile outside '\(%rdi\)'
check store_conditional inside 'g_last\(%rip\)'
check store_with_call inside 'g_last\(%rip\)'
check store_then_alias_load inside 'g_last\(%rip\)'
check store_volatile inside 'g_vol\(%rip\)'

if [ "$fail" -ne 0 ]; then
    echo "loop_store_motion FAILED"
    exit 1
fi
echo "loop_store_motion PASSED"
