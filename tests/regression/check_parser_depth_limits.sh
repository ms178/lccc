#!/usr/bin/env bash
# Parser resource-limit regression gate (gcc 16.2 oracle).
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

printf 'check_parser_depth_limits: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
