#!/bin/bash
set -euo pipefail
source /home/user/.cache/redteam94/env.sh
R=/home/user/.cache/redteam103
cd /home/user/lccc
cargo fmt --all -- --check > "$R/fmt.log" 2>&1
for t in test_evidence_integrity test_comparison_timing_provenance test_compiler_comparison_reports test_compiler_evidence_publication test_benchmark_completion test_benchmark_input_contracts; do
 python3 -m unittest discover -s tests/corpus -p "$t.py" -v > "$R/$t.log" 2>&1
 tail -4 "$R/$t.log"
done
bash scripts/build_lccc_fast.sh > "$R/candidate-build.log" 2>&1
cp target/fastbuild/lccc "$R/candidate"
RUSTFLAGS="$(cat target/lccc-rustflags)" cargo test --profile fastbuild --locked -j2 --lib > "$R/unit.log" 2>&1
tail -4 "$R/unit.log"
cd /home/user/.cache/lccc-main97
git checkout --detach 2f477ce01749824ccab73ead7ccac90abd77fa08
bash scripts/build_lccc_fast.sh > "$R/main-build.log" 2>&1
cp target/fastbuild/lccc "$R/main2f47"
printf 'INTEGRATION VALIDATED AND BOTH COMPILERS BUILT\n'
