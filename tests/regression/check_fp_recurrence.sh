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
cat "$work/expected"
oracle="$root/tests/regression/fp_recurrence/oracle.c"
"$HOST_CC" -O0 -ffp-contract=off "$oracle" -o "$work/oracle-ref"
"$work/oracle-ref" > "$work/oracle-expected"
for arch in x86-64 x86-64-v3; do
    for opt in 0 1 2 3; do
        CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort "$CCC" -O"$opt" -march="$arch" \
            -ffp-contract=off "$src" -lm -o "$work/run"
        "$work/run" > "$work/actual"
        cmp "$work/expected" "$work/actual"
        CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort "$CCC" -O"$opt" -march="$arch" \
            -ffp-contract=off "$oracle" -o "$work/oracle"
        "$work/oracle" > "$work/oracle-actual"
        cmp "$work/oracle-expected" "$work/oracle-actual"
        echo "PASS exact FP bits/exceptions and oracle digest: $arch -O$opt"
    done
    # Positive optimization shapes have a fixed FP environment. Executed
    # exact-bit/FENV cases above always keep the pragma enabled.
    for mode in on no_square no_double; do
        disabled=""
        [[ $mode != no_square ]] || disabled=loop_square
        [[ $mode != no_double ]] || disabled=fp_double
        CCC_DISABLE_PASSES="$disabled" CCC_VALIDATE_SSA=1 CCC_VERIFY_IR=abort \
            "$CCC" -O2 -march="$arch" -DNO_FENV_PRAGMA -ffp-contract=off -S "$src" -o "$work/$arch-$mode.s"
        # Runtime checks above retain FENV_ACCESS; shape benefits below use
        # a separately compiled non-observable environment, with this control.
        CCC_DISABLE_PASSES="$disabled" "$CCC" -O2 -march="$arch" \
            -ffp-contract=off -S "$src" -o "$work/$arch-observable-$mode.s"
    done
done
# The active FENV policy forbids automatic contraction. Test both sides,
# without removing any bitwise/exception/dynamic-rounding checks above.
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=fast -S "$src" -o "$work/observable-contract.s"
"$CCC" -O2 -march=x86-64-v3 -DNO_FENV_PRAGMA -ffp-contract=fast -S "$src" -o "$work/contract.s"
# Independent minimal shape control, alongside the complete semantic fixture.
shape="$root/tests/regression/fp_recurrence/contract_shape.c"
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=fast -S \
    "$shape" -o "$work/shape-contract.s"
for flags in '-march=x86-64' '-march=x86-64-v2' '-march=x86-64-v3 -mno-fma'; do
    # Deliberate word splitting of this fixed test-only flag list.
    "$CCC" -O2 $flags -ffp-contract=fast -DNO_FENV_PRAGMA -S "$src" -o "$work/no-fma-${flags// /_}.s"
done
for mode in on no_square; do
    disabled=""
    [[ $mode != no_square ]] || disabled=loop_square
    CCC_DISABLE_PASSES="$disabled" "$CCC" -O2 -march=x86-64-v3 \
        -DWIDTH=128 -DHEIGHT=128 -S "$root/tests/benchmark/programs/mandelbrot.c" -o "$work/mandel-$mode.s"
    CCC_DISABLE_PASSES="$disabled" "$CCC" -O2 -march=x86-64-v3 \
        -DWIDTH=128 -DHEIGHT=128 "$root/tests/benchmark/programs/mandelbrot.c" -o "$work/mandel-$mode"
    "$work/mandel-$mode" > "$work/mandel-$mode.out"
done
cmp "$work/mandel-on.out" "$work/mandel-no_square.out"
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
        assert muls(body(f'{arch}-no_double.s', name)) == 1, (arch, name, 'control')
        assert muls(body(f'{arch}-on.s', name)) == 0, (arch, name, 'doubling')
    before = muls(body(f'{arch}-no_square.s', 'square_loop'))
    after = muls(body(f'{arch}-on.s', 'square_loop'))
    assert before >= 2 and after == before-1, (arch, before, after)
    assert muls(body(f'{arch}-observable-on.s', 'square_loop')) == muls(body(f'{arch}-observable-no_square.s', 'square_loop')), (arch, 'observable square')
    assert muls(body(f'{arch}-on.s', 'changing_round')) == muls(body(f'{arch}-no_square.s', 'changing_round')), arch
    assert muls(body(f'{arch}-on.s', 'twice_plus')) == 0, (arch, 'strict doubling')
    assert muls(body(f'{arch}-no_double.s', 'twice_plus')) == 1, (arch, 'strict control')
for file in p.glob('no-fma-*.s'):
    assert muls(body(file.name, 'twice_plus')) == 0, file.name
# No numeric-label/RA assumptions: the complete main has exactly one fewer
# scalar multiply-or-FMA when carrying a square, even with contraction active.
def mul_fma(text):
    return len(re.findall(r'^\s*v?(?:mul(?:sd|ss)|f(?:n?madd|n?msub)\d*sd)\s', text, re.M))
before = mul_fma(body('mandel-no_square.s', 'main'))
after = mul_fma(body('mandel-on.s', 'main'))
assert after == before - 1, ('mandelbrot square/FMA', before, after)
def inner_fp_loop(text):
    lines = [line.split('#', 1)[0].strip() for line in text.splitlines()]
    labels = {line[:-1]: i for i, line in enumerate(lines) if re.fullmatch(r'\.LBB\d+:', line)}
    loops = []
    for end, line in enumerate(lines):
        edge = re.fullmatch(r'j\w+\s+(\.LBB\d+)', line)
        if edge and edge[1] in labels and labels[edge[1]] < end:
            start = labels[edge[1]]
            snippet = '\n'.join(lines[start:end+1])
            if mul_fma(snippet): loops.append((end-start, snippet))
    assert loops, 'no floating backedge loop in Mandelbrot control'
    return min(loops)[1]
assert mul_fma(inner_fp_loop(body('mandel-on.s', 'main'))) == mul_fma(inner_fp_loop(body('mandel-no_square.s', 'main'))) - 1

for name in ('twice_plus', 'scaled_fma'):
    assert not re.search(r'vfmadd\d*sd', body('observable-contract.s', name)), name
    assert re.search(r'vfmadd\d*sd', body('contract.s', name)), name
    assert re.search(r'vfmadd\d*sd', body('shape-contract.s', name)), (name, 'minimal positive control')

def redundant_edges(file):
    lines = [line.split('#', 1)[0].rstrip() for line in (p / file).read_text().splitlines()]
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
