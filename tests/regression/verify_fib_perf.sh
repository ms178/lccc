#!/usr/bin/env bash
# Verify Fibonacci performance claim: LCCC should be dramatically faster
# than GCC on recursive Fibonacci thanks to the rec2iter optimization.
#
# Usage: ./tests/verify_fib_perf.sh [path-to-lccc] [reps]
set -euo pipefail

LCCC="${1:-target/release/lccc}"
REPS="${2:-5}"
GCC_INC="$(gcc -print-file-name=include)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC="$SCRIPT_DIR/benchmark/programs/fib.c"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "=== Fibonacci Performance Verification ==="
echo "LCCC: $LCCC"
echo "Reps: $REPS"
echo ""

# 1. Compile with both compilers
echo "Compiling..."
gcc -O2 -o "$TMPDIR/fib_gcc" "$SRC"
"$LCCC" -I"$GCC_INC" -O2 -o "$TMPDIR/fib_lccc" "$SRC"

# 2. Verify correctness
GCC_OUT=$("$TMPDIR/fib_gcc")
LCCC_OUT=$("$TMPDIR/fib_lccc")
if [ "$GCC_OUT" != "$LCCC_OUT" ]; then
    echo "FAIL: Output mismatch"
    echo "  GCC:  $GCC_OUT"
    echo "  LCCC: $LCCC_OUT"
    exit 1
fi
echo "Correctness: PASS ($GCC_OUT)"

# 3. Verify rec2iter fires
REC_OUT=$(LCCC_DEBUG_RECURSION=1 "$LCCC" -I"$GCC_INC" -O2 -o /dev/null "$SRC" 2>&1 || true)
if echo "$REC_OUT" | grep -q "REC→ITER.*Transformed"; then
    echo "rec2iter pass: ACTIVE"
else
    echo "FAIL: rec2iter pass did not fire!"
    echo "  Debug output: $REC_OUT"
    exit 1
fi

# 4. Benchmark
echo ""
echo "Benchmarking ($REPS runs each)..."

# Use Python's monotonic clock instead of shell `time` or `bc`.  The old
# version required `bc`, which is absent from minimal CI images and made a
# valid compiler result fail before the assertion.  Python is already a
# repository test prerequisite and gives a locale-independent float.
best_gcc=$(python3 - "$TMPDIR/fib_gcc" "$REPS" <<'PY'
import subprocess
import sys
import time

binary, repetitions = sys.argv[1], int(sys.argv[2])
samples = []
for _ in range(repetitions):
    start = time.perf_counter()
    completed = subprocess.run([binary], stdout=subprocess.DEVNULL, check=False)
    if completed.returncode:
        raise SystemExit(f"{binary} exited with {completed.returncode}")
    samples.append(time.perf_counter() - start)
print(f"{min(samples):.9f}")
PY
)
best_lccc=$(python3 - "$TMPDIR/fib_lccc" "$REPS" <<'PY'
import subprocess
import sys
import time

binary, repetitions = sys.argv[1], int(sys.argv[2])
samples = []
for _ in range(repetitions):
    start = time.perf_counter()
    completed = subprocess.run([binary], stdout=subprocess.DEVNULL, check=False)
    if completed.returncode:
        raise SystemExit(f"{binary} exited with {completed.returncode}")
    samples.append(time.perf_counter() - start)
print(f"{min(samples):.9f}")
PY
)
ratio=$(awk -v gcc="$best_gcc" -v lccc="$best_lccc" 'BEGIN { print gcc / lccc }')
printf "GCC  best: %.4fs\n" "$best_gcc"
printf "LCCC best: %.4fs\n" "$best_lccc"
printf "Speedup:   %.0fx faster\n" "$ratio"

# 5. Assert minimum speedup (conservative: 10x, we typically see 100-400x)
MIN_SPEEDUP=10
if awk -v ratio="$ratio" -v minimum="$MIN_SPEEDUP" 'BEGIN { exit !(ratio < minimum) }'; then
    echo ""
    echo "FAIL: Expected at least ${MIN_SPEEDUP}x speedup, got ${ratio}x"
    echo "The rec2iter optimization may not be working correctly."
    exit 1
fi

echo ""
echo "PASS: LCCC is $(printf '%.0f' "$ratio")x faster than GCC on recursive Fibonacci"

# 6. Run the comprehensive correctness test
echo ""
echo "Running comprehensive rec2iter correctness test..."
"$LCCC" -I"$GCC_INC" -O2 -o "$TMPDIR/fib_comprehensive" "$SCRIPT_DIR/fib_rec2iter.c"
COMP_OUT=$("$TMPDIR/fib_comprehensive")
if [ "$COMP_OUT" = "ALL PASS" ]; then
    echo "Comprehensive test: PASS (includes fib(90) — impossible without rec2iter)"
else
    echo "FAIL: Comprehensive test output: $COMP_OUT"
    exit 1
fi
