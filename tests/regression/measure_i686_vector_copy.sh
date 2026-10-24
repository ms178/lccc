#!/usr/bin/env bash
# Bounded host measurement for the i686 stack-copy/alignment optimizations.
# This is deliberately a bounded audit, not a timing gate: the two binaries use
# different target ISA ceilings and the result is reported with the host name.
set -euo pipefail
cd "$(dirname "$0")/../.."

ccc=${CCC:-target/fastbuild/lccc}
linker=${LCCC_LD:-$(dirname "$ccc")/lccc-ld}
assembler=${LCCC_TEST_ASSEMBLER:-as}
ccc_path=$(command -v "$ccc" || true)
linker_path=$(command -v "$linker" || true)
[[ -n $ccc_path ]] || { echo "missing LCCC: $ccc" >&2; exit 2; }
[[ -n $linker_path ]] || { echo "missing lccc-ld: $linker" >&2; exit 2; }
ccc=$ccc_path
linker=$linker_path

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p target
out=${I686_VECTOR_COPY_REPORT:-target/i686-vector-copy-benchmark.json}

cat >"$work/start.s" <<'ASM'
.text
.globl _start
_start:
    andl $-16, %esp
    call main
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
.section .note.GNU-stack,"",@progbits
ASM
"$assembler" --32 "$work/start.s" -o "$work/start.o"

cat >"$work/bench.c" <<'C'
typedef unsigned v4u __attribute__((vector_size(16)));
typedef unsigned v8u __attribute__((vector_size(32)));
typedef struct { v4u v[4]; } block64;
static volatile unsigned sink;

__attribute__((noinline)) static block64 identity_block(block64 value) {
    return value;
}
__attribute__((noinline)) static block64 relay_block(block64 value) {
    return identity_block(value);
}
__attribute__((noinline)) static v8u identity_wide(v8u value) {
    return value;
}
__attribute__((noinline)) static v4u pick_variadic(v4u a, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, a);
    v4u b = __builtin_va_arg(ap, v4u);
    __builtin_va_end(ap);
    return __builtin_shufflevector(a, b, 0, 5, 2, 7);
}

int main(void) {
    enum { iterations = 5000000 };
    block64 value = {{{1,2,3,4},{5,6,7,8},{9,10,11,12},{13,14,15,16}}};
    v8u wide = {1,2,3,4,5,6,7,8};
    unsigned acc = 0;
    for (unsigned i = 0; i < iterations; ++i) {
        value.v[0][0] = i;
        value = relay_block(value);
        wide = identity_wide(wide);
        v4u r = pick_variadic(value.v[0], value.v[1]);
        acc += r[0] + r[3] + value.v[2][2] + wide[0];
    }
    sink = acc;
    unsigned long long expected64 =
        (unsigned long long)iterations * (iterations - 1) / 2
        + (unsigned long long)iterations * (8 + 11 + 1);
    return acc == (unsigned)expected64 ? 0 : 1;
}
C

common=(-m32 -fno-pic -fno-pie -fno-stack-protector -Wno-psabi -O2)
for mode in sse no-sse; do
    case $mode in
        sse) isa=(-msse) ;;
        no-sse) isa=(-mno-sse) ;;
    esac
    "$ccc" "${common[@]}" "${isa[@]}" -c "$work/bench.c" -o "$work/$mode.o"
    "$linker" -m elf_i386 -static -e _start \
        "$work/start.o" "$work/$mode.o" -o "$work/$mode"
    "$work/$mode"
    objdump -d "$work/$mode.o" >"$work/$mode.dis"
done

python3 - "$work" "$out" "$ccc" <<'PY'
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

work = Path(sys.argv[1])
out = Path(sys.argv[2])
ccc = sys.argv[3]
rows = {}
for mode in ("sse", "no-sse"):
    samples = []
    for _ in range(7):
        start = time.perf_counter()
        subprocess.run(
            [str(work / mode)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        samples.append(time.perf_counter() - start)
    dis = (work / f"{mode}.dis").read_text()
    rows[mode] = {
        "samples_s": samples,
        "median_s": statistics.median(samples),
        "movups_instructions": sum("movups" in line for line in dis.splitlines()),
        "esp_lea_instructions": sum(
            "lea" in line and "%esp" in line for line in dis.splitlines()
        ),
    }
ratio = rows["sse"]["median_s"] / rows["no-sse"]["median_s"]
host = "unknown"
try:
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            host = line.split(":", 1)[1].strip()
            break
except OSError:
    pass
report = {
    "schema": 1,
    "scope": "bounded i686 SSE versus no-SSE target comparison; not a universal performance claim",
    "compiler": subprocess.check_output([ccc, "--version"], text=True).splitlines()[0],
    "host": host,
    "iterations_per_run": 5000000,
    "repetitions": 7,
    "correctness": "both executables returned zero",
    "rows": rows,
    "sse_over_no_sse_median": ratio,
}
out.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY
