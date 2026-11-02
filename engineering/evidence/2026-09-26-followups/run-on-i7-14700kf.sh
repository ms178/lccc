#!/usr/bin/env bash
# Execute the latest-main follow-up A/B on a PHYSICAL i7-14700KF P-core.
# Required: CPU=<isolated P-core> BASE_LCCC=<compiler built at e5bc1911>
# Optional: LCCC=<follow-up compiler> GCC=<gcc> OUT=<output directory>
#           REPS=21 WARMUP=3 ONLY=<comma-separated benchmarks>
# The benchmark runner records output checks, raw rounds, hashes, commands,
# and randomized per-round compiler order; this script does not extrapolate
# the development Xeon VM's measured 27x ratio to Raptor Lake.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$repo"
if ! grep -q 'i7-14700KF' /proc/cpuinfo ||
   grep -m1 '^flags' /proc/cpuinfo | grep -qw hypervisor; then
    echo 'REFUSED: physical i7-14700KF required; VM results are not target timings.' >&2
    exit 3
fi
if ! grep -qw avx2 /proc/cpuinfo; then
    echo 'REFUSED: AVX2 is required.' >&2
    exit 3
fi
swapon_bin=$(command -v swapon || true)
swapon_bin=${swapon_bin:-/sbin/swapon}
if [[ ! -x "$swapon_bin" ]] ||
   ! "$swapon_bin" --show --noheadings | grep -q '[^[:space:]]'; then
    echo 'REFUSED: provision and activate swap before building or measuring.' >&2
    exit 3
fi
if [[ ! "${CPU:-}" =~ ^[0-9]+$ ]]; then
    echo 'REFUSED: specify one isolated P-core as a numeric CPU=<id>.' >&2
    exit 3
fi
if ! taskset -c "$CPU" /bin/true; then
    echo "REFUSED: CPU=$CPU is unavailable." >&2
    exit 3
fi
core_cpus=/sys/devices/cpu_core/cpus
core_type="/sys/devices/system/cpu/cpu${CPU}/topology/core_type"
if [[ -r "$core_cpus" ]]; then
    if ! python3 - "$CPU" "$core_cpus" <<'PY'
import pathlib
import sys
cpu = int(sys.argv[1])
cpus = pathlib.Path(sys.argv[2]).read_text().strip()
parts = (part.split('-', 1) for part in cpus.split(','))
sys.exit(0 if any(int(pair[0]) <= cpu <= int(pair[-1]) for pair in parts) else 1)
PY
    then
        echo "REFUSED: CPU=$CPU is not in the physical P-core CPU list." >&2
        exit 3
    fi
elif [[ -r "$core_type" ]]; then
    case $(cat "$core_type") in
        2|64|0x40) ;;  # Linux normalized Core type or Intel CPUID Core type.
        *) echo "REFUSED: CPU=$CPU is not a P-core." >&2; exit 3 ;;
    esac
elif [[ "${P_CORE_CONFIRMED:-}" != 1 ]]; then
    echo 'REFUSED: no P-core identification; set P_CORE_CONFIRMED=1 only after verifying CPU is a P-core.' >&2
    exit 3
fi
if [[ -z "${BASE_LCCC:-}" || ! -x "$BASE_LCCC" ]]; then
    echo 'REFUSED: BASE_LCCC must point to a compiler built from upstream e5bc1911.' >&2
    exit 3
fi
candidate=${LCCC:-"$repo/target/fastbuild/lccc"}
if [[ ! -x "$candidate" ]]; then
    echo "Compiler is not executable: $candidate" >&2
    exit 2
fi
candidate=$(readlink -f "$candidate")
base=$(readlink -f "$BASE_LCCC")
if [[ "$candidate" == "$base" ]] || cmp -s "$candidate" "$base"; then
    echo 'REFUSED: candidate and base compilers are identical.' >&2
    exit 3
fi
gcc_bin=$(command -v "${GCC:-gcc}")
out=${OUT:-"$repo/engineering/evidence/i7-14700kf-followup-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out"
out=$(readlink -f "$out")
{
    date -u
    uname -a
    lscpu
    printf 'P-core CPU list: '; cat "$core_cpus" 2>/dev/null || echo 'unavailable'
    printf 'CPU core type: '; cat "$core_type" 2>/dev/null || echo 'unavailable / operator-confirmed'
    git rev-parse HEAD
    git status --short
    "$candidate" --version
    "$base" --version
    "$gcc_bin" --version | head -1
    sha256sum "$candidate" "$base" "$gcc_bin"
    printf 'Baseline source revision required: e5bc19118c6ea5a54d5539d4a6cff52c97ab6baa\n'
} > "$out/environment.txt"
if command -v perf >/dev/null 2>&1; then
    (perf stat -e cycles,instructions -- /bin/true 2>&1 || true) > "$out/pmu-probe.txt"
    if grep -Eq '<not supported>|No permission|Permission denied' "$out/pmu-probe.txt"; then
        echo 'WARNING: PMU unavailable; do not claim cycles/instructions.' >&2
    fi
fi
only=${ONLY:-lz4_match_extend,linux_find_bit_scaled,mandelbrot,sha256_transform,expat_xml_scan,sqlite_varint}
reps=${REPS:-21}
warmup=${WARMUP:-3}
common=(--compilers lccc,ccc,gcc --lccc "$candidate" --ccc "$base" --gcc "$gcc_bin"
        --opt=-O2 --cflag=-march=x86-64-v3 --cflag=-mtune=raptorlake
        --cpu "$CPU" --reps "$reps" --warmup "$warmup" --strict)
python3 tests/benchmark/run_benchmarks.py "${common[@]}" \
    --only "$only" --seed 260926 \
    --artifact-dir "$out/p0/artifacts" --json "$out/p0/results.json" \
    --markdown "$out/p0/results.md" 2>&1 | tee "$out/p0.log"
# Run the fixed and baseline spectral kernels at the SAME larger N. The
# default N=2000 is near 200 ms on the VM, but may be too short on a 14700KF;
# do not treat sub-200 ms target samples as a performance result. Raise
# SPECTRAL_N and rerun if even the scaled candidate is below that threshold.
spectral_n=${SPECTRAL_N:-4000}
python3 tests/benchmark/run_benchmarks.py "${common[@]}" \
    --only spectral_norm --cflag="-DN=$spectral_n" --seed 260927 \
    --artifact-dir "$out/spectral/artifacts" --json "$out/spectral/results.json" \
    --markdown "$out/spectral/results.md" 2>&1 | tee "$out/spectral.log"
echo "Target raw samples: $out/{p0,spectral}/results.json"
echo 'Review output checks, paired median and minimum, noise, and CPU/PMU metadata before asserting a hardware speedup.'
