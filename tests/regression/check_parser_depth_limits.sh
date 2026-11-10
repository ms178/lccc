#!/usr/bin/env bash
# Parser resource-limit and unsupported-input refusal gate (gcc 16.2 oracle).
#
# gcc.c-torture/compile/limits-pointer.c expands PTR4 to 10,000 pointer stars.
# lccc's old POINTER_DEPTH_BUDGET (4096, calibrated for a hypothetical 8 MB
# stack) rejected it, while gcc 16.2 compiles it in 0.015 s.  The compiler
# actually runs on a 64 MB thread, so the bound was re-measured (see
# Parser::POINTER_DEPTH_BUDGET in src/frontend/parser/parse.rs) and raised to
# 65,536 -- still >6x below the ~400,000-star clone/drop stack-overflow point.
#
# This gate pins BOTH ends of the measurement: the corpus-depth case and the
# near-limit case must compile quickly, and a depth beyond the budget must
# produce ONE clean diagnostic and never an abort/segv/signal.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
[ -x "$CCC" ] || { echo "check_parser_depth_limits: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-parser-depth.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

pass=0
fail=0
check() {
    if [ "$2" -eq 0 ]; then pass=$((pass + 1)); else printf 'FAIL: %s\n' "$1"; fail=$((fail + 1)); fi
}

for n in 10000 32768 65536; do
    python3 -c "import sys; sys.stdout.write('int ' + '*'*$n + 'p;\n')" >"$tmp/n$n.c"
    start=$(date +%s%N)
    if "$CCC" -O2 -c "$tmp/n$n.c" -o "$tmp/n.o" >"$tmp/n.log" 2>&1 \
       && ! grep -q 'error:' "$tmp/n.log"; then
        end=$(date +%s%N)
        ms=$(( (end - start) / 1000000 ))
        # 5 s is a generous ceiling: measured 0.010/0.023/0.039 s.
        if [ "$ms" -lt 5000 ]; then
            check "$n stars compile (${ms} ms)" 0
        else
            printf 'FAIL: %s stars took %s ms (>5000 ms)\n' "$n" "$ms"
            check "$n stars time" 1
        fi
    else
        printf 'FAIL: %s stars: %s\n' "$n" "$(head -3 "$tmp/n.log")"
        check "$n stars compile" 1
    fi
done

# Beyond the budget: exactly one clean diagnostic, no signal termination.
python3 -c "import sys; sys.stdout.write('int ' + '*'*80000 + 'p;\n')" >"$tmp/over.c"
set +e
"$CCC" -O2 -c "$tmp/over.c" -o "$tmp/n.o" >"$tmp/over.log" 2>&1
rc=$?
set -e
count=$(grep -c 'pointer nesting too deep (exceeds 65536 levels)' "$tmp/over.log" || true)
if [ "$rc" -eq 1 ] && [ "$count" -eq 1 ]; then
    check '80000 stars: one clean budget diagnostic, rc=1' 0
else
    printf 'FAIL: 80000 stars rc=%s diagnostics=%s: %s\n' "$rc" "$count" "$(head -3 "$tmp/over.log")"
    check '80000 stars clean rejection' 1
fi

# ---------------------------------------------------------------------------
# The other three resource budgets (block / expression / type nesting).
#
# gcc.c-torture/compile/limits-caselabels.c, limits-exprparen.c and
# limits-structnest.c deliberately exceed them.  A clean, bounded rejection
# is the contract: rc=1, ONE diagnostic naming the budget, no signal, no
# panic, no stack overflow.  The legal-depth legs prove the budgets are not
# accidentally too small for real code.  Both sides are measured here so a
# future "fix" that simply raises a bound into a stack overflow is caught.
# ---------------------------------------------------------------------------
budget_case() {
    label=$1
    expected=$2           # 0 = must compile, 1 = clean rejection
    pattern=$3            # diagnostic substring required for a rejection
    src=$4

    set +e
    "$CCC" -O0 -c "$src" -o "$tmp/budget.o" >"$tmp/budget.log" 2>&1
    rc=$?
    set -e

    if [ "$expected" -eq 0 ]; then
        if [ "$rc" -eq 0 ] && ! grep -q 'error:' "$tmp/budget.log"; then
            check "$label compiles within the budget" 0
        else
            printf 'FAIL: %s rc=%s: %s\n' "$label" "$rc" "$(head -2 "$tmp/budget.log")"
            check "$label compiles within the budget" 1
        fi
        return
    fi

    # rc=1 only: 101 is a Rust panic, 134/139/141 are signals.
    count=$(grep -c "error: .*$pattern" "$tmp/budget.log" || true)
    errs=$(grep -c 'error:' "$tmp/budget.log" || true)
    # Two `error:` lines are expected: the primary budget diagnostic plus the
    # driver's "1 parse error(s)" summary.  Anything more is cascading noise.
    if [ "$rc" -eq 1 ] && [ "$count" -eq 1 ] && [ "$errs" -le 2 ] \
       && ! grep -qi 'panicked\|internal error\|segmentation fault' "$tmp/budget.log"; then
        check "$label: one clean budget diagnostic, rc=1" 0
    else
        printf 'FAIL: %s rc=%s diagnostics=%s matching=%s: %s\n' \
            "$label" "$rc" "$errs" "$count" "$(head -3 "$tmp/budget.log")"
        check "$label clean rejection" 1
    fi
}

python3 - "$tmp" <<'PY'
import pathlib, sys
d = pathlib.Path(sys.argv[1])
for n in (2000, 4000, 4200, 5000):
    (d / f"block{n}.c").write_text(
        "int f(void) {" + "{" * (n - 1) + " return 0; " + "}" * n + "\n")
for n in (1000, 2000, 34000):
    (d / f"expr{n}.c").write_text("int x = " + "(" * n + "1" + ")" * n + ";\n")
for n in (1000, 2100):
    src = "".join(f"struct A{i} {{ " for i in range(n)) + "int v; " \
          + "".join(f"}} a{i}; " for i in range(n))
    (d / f"type{n}.c").write_text(src + "\n")
PY

budget_case 'block nesting 4000'   0 '' "$tmp/block4000.c"
budget_case 'block nesting 4200'   1 'block nesting too deep' "$tmp/block4200.c"
budget_case 'block nesting 5000'   1 'block nesting too deep' "$tmp/block5000.c"
budget_case 'expression nesting 1000' 0 '' "$tmp/expr1000.c"
budget_case 'expression nesting 2000' 1 'expression nesting too deep' "$tmp/expr2000.c"
budget_case 'expression nesting 34000' 1 'expression nesting too deep' "$tmp/expr34000.c"
budget_case 'type nesting 1000'   0 '' "$tmp/type1000.c"
budget_case 'type nesting 2100'   1 'type nesting too deep' "$tmp/type2100.c"

# ---------------------------------------------------------------------------
# Unsupported-input refusal contracts (self-contained; no corpus dependency).
#
# These three spellings appear in gcc.c-torture/compile as dg-options.  A
# compiler must either implement the request or refuse it loudly; the worst
# outcome is silently ignoring a data-model option, because the resulting
# object disagrees with the code it is linked against.  The GIMPLE case is
# GCC's own internal SSA IR, not C: LCCC deliberately has no GIMPLE front
# end, so the contract is a bounded, panic-free rc=1 refusal (a cascade of
# syntax errors is acceptable for invalid C, an unbounded one is not).
# ---------------------------------------------------------------------------
cat >"$tmp/unsupported.c" <<'EOF'
struct S { char c; int i; short s; };
struct S g = { 1, 2, 3 };
int f(void) { return (int)sizeof(struct S) + g.i; }
EOF
cat >"$tmp/gimple.c" <<'EOF'
unsigned __GIMPLE (ssa)
foo (unsigned a, unsigned b)
{
  unsigned j;
  __BB(2):
  if (a_6(D) != 0u) goto __BB3; else goto __BB4;
  __BB(3):
  j_10 = __PHI (__BB2: b_11(D));
  goto __BB4;
  __BB(4):
  return j_10;
}
EOF

set +e
"$CCC" -fpack-struct -c "$tmp/unsupported.c" -o "$tmp/pack.o" >"$tmp/pack.log" 2>&1
pack_rc=$?
"$CCC" -march=skylake -c "$tmp/unsupported.c" -o "$tmp/skylake.o" >"$tmp/skylake.log" 2>&1
skylake_rc=$?
"$CCC" -fgimple -c "$tmp/gimple.c" -o "$tmp/gimple.o" >"$tmp/gimple.log" 2>&1
gimple_rc=$?
set -e

pack_errs=$(grep -c 'error:' "$tmp/pack.log" || true)
if [ "$pack_rc" -eq 0 ] && [ "$pack_errs" -eq 0 ] && [ -s "$tmp/pack.o" ]; then
    check '-fpack-struct is implemented (bare form compiles and emits an object)' 0
else
    printf 'FAIL: -fpack-struct rc=%s errors=%s: %s\n' "$pack_rc" "$pack_errs" \
        "$(head -2 "$tmp/pack.log")"
    check '-fpack-struct implemented' 1
fi

# The layouts themselves are pinned against GCC in
# tests/regression/check_pack_struct_mcpu.py; this gate additionally pins
# that an invalid cap still fails closed with GCC's diagnostic.
set +e
"$CCC" -fpack-struct=3 -c "$tmp/unsupported.c" -o "$tmp/pack3.o" >"$tmp/pack3.log" 2>&1
pack3_rc=$?
set -e
if [ "$pack3_rc" -eq 1 ] && grep -q 'small power of two, not 3' "$tmp/pack3.log"; then
    check '-fpack-struct=3 fails closed with GCCs diagnostic' 0
else
    printf 'FAIL: -fpack-struct=3 rc=%s: %s\n' "$pack3_rc" "$(head -2 "$tmp/pack3.log")"
    check '-fpack-struct=3 fails closed' 1
fi

if [ "$skylake_rc" -eq 0 ] && [ -s "$tmp/skylake.o" ] \
   && ! grep -q 'error:' "$tmp/skylake.log"; then
    check '-march=skylake is accepted on x86-64 and emits an object' 0
else
    printf 'FAIL: -march=skylake rc=%s: %s\n' "$skylake_rc" "$(head -2 "$tmp/skylake.log")"
    check '-march=skylake accepted' 1
fi

gimple_errs=$(grep -c 'error:' "$tmp/gimple.log" || true)
if [ "$gimple_rc" -eq 1 ] && [ "$gimple_errs" -ge 1 ] && [ "$gimple_errs" -le 40 ] \
   && ! grep -qi 'panicked\|internal error\|segmentation fault' "$tmp/gimple.log"; then
    check "GCC GIMPLE IR is refused cleanly (rc=1, $gimple_errs bounded errors, no ICE)" 0
else
    printf 'FAIL: GIMPLE rc=%s errors=%s: %s\n' "$gimple_rc" "$gimple_errs" \
        "$(head -3 "$tmp/gimple.log")"
    check 'GCC GIMPLE IR clean refusal' 1
fi

# ---------------------------------------------------------------------------
# DA-01 scaling shape: block-scope declarations must not be quadratic.
#
# 100k declarations in one block once cost ~41 s of sema (linear scans in
# the local-object table plus a byte-rescan per span resolution). The fix
# is indexed on both paths; this leg pins the SHAPE, not a wall time: the
# 80k case's sema time must stay below 3.0x the 40k case's (a quadratic
# regression reads ~4x), with the phase timer as the instrument so parse
# and lowering noise cannot mask it.
# ---------------------------------------------------------------------------
for n in 40000 80000; do
    python3 -c "
import sys
sys.stdout.write('void f(void){\n')
for i in range($n):
    sys.stdout.write('int v%d;\n' % i)
sys.stdout.write('}\n')" >"$tmp/decl$n.c"
done
sema_time() {
    CCC_TIME_PHASES=1 "$CCC" -O0 -w -c "$1" -o "$tmp/decl.o" 2>&1 \
        | sed -n 's/.*\[TIME\] sema: \([0-9.]*\)s.*/\1/p'
}
t40=$(sema_time "$tmp/decl40000.c")
t80=$(sema_time "$tmp/decl80000.c")
if python3 -c "
import sys
t40, t80 = float('$t40' or 0), float('$t80' or 0)
sys.exit(0 if t40 > 0 and t80 < 3.0 * t40 + 0.05 else 1)"; then
    check "declaration scaling is sub-quadratic (sema 40k=${t40}s 80k=${t80}s)" 0
else
    printf 'FAIL: declaration scaling sema 40k=%s 80k=%s (ratio bound 3.0x)\n' "$t40" "$t80"
    check 'declaration scaling is sub-quadratic' 1
fi

printf 'check_parser_depth_limits: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
