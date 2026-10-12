#!/usr/bin/env bash
# Exact FP/FENV semantics and causal codegen contracts for recurrence reuse,
# doubling and late empty-branch threading. No fast-math assumptions.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
HOST_CC=${HOST_CC:-gcc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src="$root/tests/regression/fp_recurrence/cases.c"
# The host is an unoptimized execution control. GCC's optimized dynamic-FENV
# behavior is not the oracle; the fixture checks volatile arithmetic bitwise.
"$HOST_CC" -O0 -frounding-math -ffp-contract=off "$src" -lm -o "$work/ref"
"$work/ref" > "$work/expected"
for arch in x86-64 x86-64-v3; do
    for opt in 0 1 2 3; do
        CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort "$CCC" -O"$opt" -march="$arch" \
            -ffp-contract=off "$src" -lm -o "$work/run"
        "$work/run" > "$work/actual"
        cmp "$work/expected" "$work/actual"
        echo "PASS exact FP bits/exceptions: $arch -O$opt"
    done
    for mode in on off; do
        disabled=""
        [[ $mode != off ]] || disabled="loop_square,fp_double"
        CCC_DISABLE_PASSES="$disabled" CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort \
            "$CCC" -O2 -march="$arch" -ffp-contract=off -S "$src" -o "$work/$arch-$mode.s"
    done
done
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=fast -S "$src" -o "$work/contract.s"
# x87 is deliberately outside the new FP passes' scope.
printf '%s\n' 'double twice(double x) { return 2.0*x; }' > "$work/i686.c"
"$CCC" -m32 -O2 -S "$work/i686.c" -o "$work/i686-on.s"
CCC_DISABLE_PASSES=loop_square,fp_double "$CCC" -m32 -O2 -S "$work/i686.c" -o "$work/i686-off.s"
cmp "$work/i686-on.s" "$work/i686-off.s"
# Use a shipped, branch-heavy workload to pin the post-allocation opportunity.
for mode in on off; do
    skip=""
    [[ $mode != off ]] || skip=jump_thread
    CCC_PEEPHOLE_SKIP="$skip" "$CCC" -O2 -march=x86-64-v3 -S \
        "$root/tests/benchmark/programs/expat_xml_scan.c" -o "$work/jump-$mode.s"
done
python3 - "$work" <<'PY'
import re, sys
from pathlib import Path
p = Path(sys.argv[1])
def body(file, name):
    text = (p / file).read_text()
    m = re.search(r'^'+re.escape(name)+r':\n(.*?)^\.size\s+'+re.escape(name)+r',', text, re.M | re.S)
    assert m, (file, name)
    return m.group(1)
def muls(text):
    return len(re.findall(r'^\s*v?mul(?:sd|ss)\s', text, re.M))
for arch in ('x86-64', 'x86-64-v3'):
    for name in ('twice64', 'twice32'):
        assert muls(body(f'{arch}-off.s', name)) == 1, (arch, name, 'control')
        assert muls(body(f'{arch}-on.s', name)) == 0, (arch, name, 'doubling')
    before = muls(body(f'{arch}-off.s', 'square_loop'))
    after = muls(body(f'{arch}-on.s', 'square_loop'))
    assert before >= 2 and after == before-1, (arch, before, after)
    assert muls(body(f'{arch}-on.s', 'changing_round')) == muls(body(f'{arch}-off.s', 'changing_round')), arch
for name in ('twice_plus', 'scaled_fma'):
    assert re.search(r'vfmadd\d*sd', body('contract.s', name)), name

def redundant_edges(file):
    lines = (p / file).read_text().splitlines()
    labels = {s.strip()[:-1]: i for i, s in enumerate(lines) if re.fullmatch(r'\s*\.LBB\d+:', s)}
    redirects = set()
    for label, i in labels.items():
        for s in lines[i+1:]:
            s = s.strip()
            if not s or s.startswith('#') or re.fullmatch(r'\.LBB\d+:', s):
                continue
            if re.fullmatch(r'jmp\s+\.LBB\d+', s): redirects.add(label)
            break
    return sum(bool((m := re.fullmatch(r'\s*j\w+\s+(\.LBB\d+)\s*', s)) and m[1] in redirects) for s in lines)
assert redundant_edges('jump-off.s') > 0, 'missing negative control'
assert redundant_edges('jump-on.s') == 0, 'eligible empty trampoline survived'
print('PASS causal FP, contraction, x87 and jump-thread codegen contracts')
PY
