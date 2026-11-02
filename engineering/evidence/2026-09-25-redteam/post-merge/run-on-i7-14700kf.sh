#!/usr/bin/env bash
# Run on an ACTUAL i7-14700KF, not the virtualized AVX-512 Xeon used for
# development screening. Example: CPU=2 REPS=21 OUT=/path/to/results bash \
#   engineering/evidence/2026-09-25-redteam/post-merge/run-on-i7-14700kf.sh
# Optionally set BASE_LCCC=/path/to/unpatched-compiler to add a third, paired
# arm. The benchmark runner labels that arm "CCC" for historical reasons;
# the binary pathname, version, and SHA-256 in results.json identify it exactly.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../../.." && pwd)
cd "$repo"
if ! grep -q 'i7-14700KF' /proc/cpuinfo; then
    echo 'REFUSED: no i7-14700KF detected; VM numbers are not target-hardware evidence.' >&2
    exit 3
fi
if ! grep -qw avx2 /proc/cpuinfo; then
    echo 'REFUSED: target lacks AVX2.' >&2
    exit 3
fi
if grep -m1 '^flags' /proc/cpuinfo | grep -qw hypervisor; then
    echo 'REFUSED: virtualized CPU; target timings require the physical i7-14700KF.' >&2
    exit 3
fi
swap_tool=$(command -v swapon || true)
swap_tool=${swap_tool:-/sbin/swapon}
if [[ ! -x "$swap_tool" ]] || ! "$swap_tool" --show --noheadings | grep -q '[^[:space:]]'; then
    echo 'REFUSED: no active swap; provision a swap file before building/benchmarking.' >&2
    exit 3
fi
cpu=${CPU:-0}
if ! taskset -c "$cpu" /bin/true; then
    echo "REFUSED: CPU $cpu is not accessible. Choose an isolated P-core via CPU=..." >&2
    exit 3
fi
lccc=${LCCC:-"$repo/target/fastbuild/lccc"}
if [[ ! -x "$lccc" ]]; then echo "Compiler not executable: $lccc" >&2; exit 2; fi
lccc=$(readlink -f "$lccc")
gcc_bin=$(command -v "${GCC:-gcc}")
out=${OUT:-"$repo/engineering/evidence/i7-14700kf-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out"
out=$(readlink -f "$out")
reps=${REPS:-21}
warmup=${WARMUP:-3}
{
    date -u
    uname -a
    lscpu
    git rev-parse HEAD
    git status --short
    "$lccc" --version
    "$gcc_bin" --version | head -1
    sha256sum "$lccc" "$gcc_bin"
} > "$out/environment.txt"
if command -v perf >/dev/null 2>&1; then
    (perf stat -e cycles,instructions -- /bin/true 2>&1 || true) > "$out/pmu-probe.txt"
    if grep -Eq '<not supported>|No permission|Permission denied' "$out/pmu-probe.txt"; then
        echo 'WARNING: hardware PMU unavailable; do not claim cycles/instructions.' >&2
    fi
fi
args=(--compilers lccc,gcc --lccc "$lccc" --gcc "$gcc_bin")
if [[ -n "${BASE_LCCC:-}" ]]; then
    if [[ ! -x "$BASE_LCCC" ]]; then echo "Base compiler not executable: $BASE_LCCC" >&2; exit 2; fi
    args=(--compilers lccc,ccc,gcc --lccc "$lccc" --ccc "$(readlink -f "$BASE_LCCC")" --gcc "$gcc_bin")
    sha256sum "$BASE_LCCC" >> "$out/environment.txt"
fi
# Uniform flags, same CPU, per-round randomized compiler order, excluded
# warmups, output checks on all rounds, and all raw timings/artifacts retained.
python3 tests/benchmark/run_benchmarks.py "${args[@]}" \
    --opt=-O2 --cflag=-march=x86-64-v3 --cflag=-mtune=raptorlake \
    --cpu "$cpu" --seed 250926 --reps "$reps" --warmup "$warmup" --strict \
    --artifact-dir "$out/artifacts" --json "$out/results.json" \
    --markdown "$out/results.md" 2>&1 | tee "$out/run.log"
echo "Target results: $out/results.md ($out/results.json holds raw rounds)"
