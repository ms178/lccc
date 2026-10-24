#!/usr/bin/env python3
"""Correctness and measurement harness for IPA-PTA and live-range shrinkage.

The correctness lane is deliberately differential and fail-closed: every
feature-enabled executable is compared with the same compiler without the two
options and, when available, with the strict GCC/Clang/ICX oracles.  A small
positive witness also checks the IPA-PTA statistics stream, so a silent no-op
implementation cannot turn a green runtime comparison into a false claim.

Performance is reported, not asserted as universal superiority.  Process
execution is interleaved between baseline and feature builds and summarized by
medians; callers can archive the JSON and apply a workload-specific regression
policy rather than hiding noise behind a single timing.

Examples:
  python3 tests/regression/check_ipa_pta_live_range.py \
      --lccc target/fastbuild/lccc --gcc gcc-16 --clang clang-23 \
      --output target/ipa-lrs-report.json
  python3 tests/regression/check_ipa_pta_live_range.py --skip-performance
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Iterable


FEATURES = ["-fipa-pta", "-flive-range-shrinkage"]
STRICT = ["-std=gnu17", "-O2", "-fno-fast-math"]


@dataclass(frozen=True)
class Case:
    name: str
    source: str
    helper: str = ""
    performance: bool = False


CASES = [
    Case(
        "ipa-readonly-forwarding",
        r'''
#include <stdio.h>
int cell = 7;
static volatile int observed;
static __attribute__((noinline)) void read_only(int *p) {
    /* The volatile access prevents the earlier purity pass from deleting
       this call.  Its only write is to a distinct object, so IPA must retain
       the caller's load of cell while still modelling the side effect. */
    observed = *(volatile int *)p;
}
int main(void) {
    /* A global keeps this witness after early scalar replacement: the
       call must be proven read-only before the second load can forward.
       Enough iterations make process startup negligible in the timing row. */
    int before = 0, after = 0;
    unsigned sum = 0;
    for (unsigned i = 0; i < 20000000; ++i) {
        before = cell;
        read_only(&cell);
        after = cell;
        sum = sum * 1664525u + (unsigned)before + (unsigned)after + 1013904223u;
    }
    printf("%d %d %08x\n", before, after, sum);
    return before != 7 || after != 7 || observed != 7;
}
''',
        performance=True,
    ),
    Case(
        "ipa-direct-write-must-not-forward",
        r'''
#include <stdio.h>
static __attribute__((noinline)) void write_value(int *p) { *p = 19; }
int main(void) {
    int x = 3;
    int *p = &x;
    int before = *p;
    write_value(p);
    int after = *p;
    printf("%d %d\n", before, after);
    return before != 3 || after != 19;
}
''',
    ),
    Case(
        "ipa-indirect-readonly",
        r'''
#include <stdio.h>
static __attribute__((noinline)) void read_only(int *p) {
    if (*p == -54321) __builtin_trap();
}
int main(void) {
    int x = 23;
    int *p = &x;
    void (*fp)(int *) = read_only;
    int before = *p;
    fp(p);
    int after = *p;
    printf("%d %d\n", before, after);
    return before != 23 || after != 23;
}
''',
    ),
    Case(
        "ipa-external-escape-and-clobber",
        r'''
#include <stdio.h>
extern void external_write(int *);
int main(void) {
    int x = 5;
    int *p = &x;
    int before = *p;
    external_write(p);
    int after = *p;
    printf("%d %d\n", before, after);
    return before != 5 || after != 37;
}
''',
        helper="void external_write(int *p) { *p = 37; }\n",
    ),
    Case(
        "ipa-variadic-pointer-write",
        r'''
#include <stdio.h>
#include <stdarg.h>
static __attribute__((noinline)) void write_variadic(int named, ...) {
    va_list ap;
    va_start(ap, named);
    int *p = va_arg(ap, int *);
    *p = named + 40;
    va_end(ap);
}
int main(void) {
    int x = 1;
    int *p = &x;
    int before = *p;
    write_variadic(2, p);
    int after = *p;
    printf("%d %d\n", before, after);
    return before != 1 || after != 42;
}
''',
    ),
    Case(
        "ipa-recursive-summary",
        r'''
#include <stdio.h>
static __attribute__((noinline)) void recursive_write(int *p, unsigned n) {
    if (n) recursive_write(p, n - 1);
    else *p = 61;
}
int main(void) {
    int x = 2;
    int *p = &x;
    int before = *p;
    recursive_write(p, 5);
    int after = *p;
    printf("%d %d\n", before, after);
    return before != 2 || after != 61;
}
''',
    ),
    Case(
        "lrs-independent-pressure",
        r'''
#include <stdio.h>
#include <stdint.h>
static __attribute__((noinline)) uint32_t pressure(uint32_t x) {
    /* Enough independent definitions to exceed the x86-64 GP budget.  Most
       inputs die at the paired operations, giving the scheduler a real
       pressure-reducing alternative rather than a cosmetic permutation. */
    uint32_t a0 = x + 1, a1 = x ^ 0x13579bdfu, a2 = x * 3 + 7;
    uint32_t a3 = x + 11, a4 = x ^ 0x2468ace0u, a5 = x * 5 + 13;
    uint32_t a6 = x + 17, a7 = x ^ 0x55aa33ccu, a8 = x * 7 + 19;
    uint32_t a9 = x + 23, a10 = x ^ 0x10203040u, a11 = x * 9 + 29;
    uint32_t a12 = x + 31, a13 = x ^ 0x31415926u, a14 = x * 11 + 37;
    uint32_t a15 = x + 41, a16 = x ^ 0x27182818u, a17 = x * 13 + 43;
    uint32_t a18 = x + 47, a19 = x ^ 0xdeadbeefu;
    uint32_t b0 = a0 * 31 + a1, b1 = a2 * 37 + a3;
    uint32_t b2 = a4 * 41 + a5, b3 = a6 * 43 + a7;
    uint32_t b4 = a8 * 47 + a9, b5 = a10 * 53 + a11;
    uint32_t b6 = a12 * 59 + a13, b7 = a14 * 61 + a15;
    uint32_t b8 = a16 * 67 + a17, b9 = a18 * 71 + a19;
    uint32_t c0 = b0 + b1 * 3, c1 = b2 + b3 * 5, c2 = b4 + b5 * 7;
    uint32_t c3 = b6 + b7 * 11, c4 = b8 + b9 * 13;
    uint32_t d0 = c0 ^ (c1 + 0x9e3779b9u), d1 = c2 ^ (c0 + 0x7f4a7c15u);
    uint32_t d2 = c3 ^ (c4 + 0x6a09e667u);
    return d0 + d1 * 3 + d2 * 5 + b0 + b2 + b4 + b6 + b8
         + a1 + a5 + a9 + a13 + a17;
}
int main(void) {
    uint32_t sum = 0;
    for (uint32_t i = 0; i < 4000000; ++i) sum = sum * 1664525u + pressure(i) + 1013904223u;
    printf("%08x\n", sum);
    return 0;
}
''',
        performance=True,
    ),
    Case(
        "lrs-low-latency-pressure",
        r'''
#include <stdio.h>
#include <stdint.h>
static __attribute__((noinline)) uint32_t low_latency(uint32_t x) {
    uint32_t a0 = x + 1, a1 = x ^ 0x13579bdfu, a2 = x + 7;
    uint32_t a3 = x + 11, a4 = x ^ 0x2468ace0u, a5 = x + 13;
    uint32_t a6 = x + 17, a7 = x ^ 0x55aa33ccu, a8 = x + 19;
    uint32_t a9 = x + 23, a10 = x ^ 0x10203040u, a11 = x + 29;
    uint32_t a12 = x + 31, a13 = x ^ 0x31415926u, a14 = x + 37;
    uint32_t a15 = x + 41, a16 = x ^ 0x27182818u, a17 = x + 43;
    uint32_t a18 = x + 47, a19 = x ^ 0xdeadbeefu;
    uint32_t b0 = a0 + a1, b1 = a2 + a3, b2 = a4 + a5, b3 = a6 + a7;
    uint32_t b4 = a8 + a9, b5 = a10 + a11, b6 = a12 + a13;
    uint32_t b7 = a14 + a15, b8 = a16 + a17, b9 = a18 + a19;
    return b0 + b1 + b2 + b3 + b4 + b5 + b6 + b7 + b8 + b9
         + a1 + a5 + a9 + a13 + a17;
}
int main(void) {
    uint32_t sum = 0;
    for (uint32_t i = 0; i < 4000000; ++i) sum += low_latency(i);
    printf("%08x\n", sum);
    return 0;
}
''',
        performance=True,
    ),
    Case(
        "lrs-memory-and-volatile-boundaries",
        r'''
#include <stdio.h>
static volatile int observed;
static __attribute__((noinline)) int boundary(int x) {
    int a = x + 3, b = x * 5, c = x ^ 17;
    observed = a;
    int d = b + c, e = a ^ b;
    return d + e + observed;
}
int main(void) {
    int sum = 0;
    for (int i = 0; i < 100; ++i) sum += boundary(i);
    printf("%d\n", sum);
    return 0;
}
''',
    ),
]


def run(cmd: list[str], *, timeout: int = 30, env: dict[str, str] | None = None,
        cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, env=env, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          timeout=timeout, check=False)


def host_cpu() -> dict[str, str]:
    result = {"machine": platform.machine()}
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith(("model name", "Hardware", "flags", "Features")):
                key, value = line.split(":", 1)
                result.setdefault(key.strip(), value.strip())
    except OSError:
        pass
    return result


def escape_annotation(text: str) -> str:
    return (text.replace("%", "%25").replace("\r", "%0D")
            .replace("\n", "%0A"))


def forwarding_counts(text: str) -> list[int]:
    return [int(value) for value in re.findall(r"redundant-loads-forwarded=(\d+)", text)]


def shrinkage_counts(text: str) -> list[int]:
    return [int(value) for value in re.findall(r"\[LR-SHRINK\][^\n]*?changed=(\d+)", text)]


def version(compiler: str) -> str:
    try:
        result = run([compiler, "--version"], timeout=10)
        return (result.stdout or result.stderr).splitlines()[0]
    except (OSError, subprocess.TimeoutExpired, IndexError):
        return "unavailable"


def compile_and_run(compiler: str, case: Case, directory: Path, features: bool,
                    gcc_include: str, stats: bool = False) -> dict:
    return compile_and_run_flags(compiler, case, directory, features, gcc_include,
                                 stats=stats, extra_flags=())


def compile_and_run_flags(compiler: str, case: Case, directory: Path, features: bool,
                          gcc_include: str, stats: bool = False,
                          extra_flags: Iterable[str] = ()) -> dict:
    extra_flags = tuple(extra_flags)
    tag = "feature" if features else "baseline"
    compiler_tag = re.sub(r"[^A-Za-z0-9_.-]+", "-", Path(compiler).name)
    tag = f"{compiler_tag}-{tag}"
    suffix = "-".join(extra_flags)
    if suffix:
        safe_suffix = "".join(c if c.isalnum() else "-" for c in suffix)
        tag = f"{tag}-{safe_suffix}"
    source = directory / f"{case.name}-{tag}.c"
    source.write_text(case.source)
    files = [str(source)]
    if case.helper:
        helper = directory / f"{case.name}-{tag}-helper.c"
        helper.write_text(case.helper)
        files.append(str(helper))
    executable = directory / f"{case.name}-{tag}.exe"
    cmd = [compiler, *STRICT]
    if Path(compiler).name in {"icx", "icpx"}:
        cmd.append("-fp-model=strict")
    cmd.extend([*(FEATURES if features else []), *extra_flags,
                f"-I{gcc_include}", *files, "-o", str(executable)])
    env = os.environ.copy()
    if stats:
        env["CCC_IPA_PTA_STATS"] = "1"
        env["CCC_LR_SHRINK_STATS"] = "1"
    compiled = run(cmd, timeout=90, env=env)
    record = {
        "compiler": compiler,
        "case": case.name,
        "features": features,
        "compile_command": cmd,
        "executable": str(executable),
        "compile_returncode": compiled.returncode,
        "compile_stderr": compiled.stderr[-12000:],
    }
    if compiled.returncode:
        return record
    try:
        executed = run([str(executable)], timeout=30, env=env)
    except subprocess.TimeoutExpired:
        record["runtime_timeout"] = True
        return record
    record.update({
        "runtime_returncode": executed.returncode,
        "stdout": executed.stdout,
        "stderr": executed.stderr[-12000:],
    })
    return record


def oracle_compilers(args: argparse.Namespace) -> list[str]:
    result = []
    for candidate in (args.gcc, args.clang, args.icx):
        if candidate and shutil.which(candidate):
            result.append(candidate)
    return result


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG", "clang-23"))
    parser.add_argument("--icx", default=os.environ.get("ICX", "icx"))
    parser.add_argument("--skip-oracles", action="store_true")
    parser.add_argument("--skip-performance", action="store_true")
    parser.add_argument("--skip-matrix", action="store_true",
                        help="only run the default -O2 correctness configuration")
    parser.add_argument("--reps", type=int, default=5)
    parser.add_argument("--output", type=Path, default=Path("target/ipa-lrs-report.json"))
    args = parser.parse_args(argv)
    if args.reps < 1:
        parser.error("--reps must be positive")
    if not shutil.which(args.lccc) and not Path(args.lccc).exists():
        parser.error(f"LCCC compiler not found: {args.lccc}")
    gcc_for_include = args.gcc if shutil.which(args.gcc) else "gcc"
    try:
        gcc_include = subprocess.check_output(
            [gcc_for_include, "-print-file-name=include"], text=True, timeout=15
        ).strip()
    except (OSError, subprocess.TimeoutExpired, subprocess.CalledProcessError) as exc:
        parser.error(f"cannot determine GCC include directory: {exc}")

    report = {
        "schema": 1,
        "features": FEATURES,
        "host": {"platform": platform.platform(), **host_cpu()},
        "compilers": {c: version(c) for c in [args.lccc, *oracle_compilers(args)]},
        "cases": [],
        "performance": [],
    }
    failures = []
    forwarding_witness = False
    shrinkage_witness = False
    with tempfile.TemporaryDirectory(prefix="lccc-ipa-lrs-") as temporary:
        directory = Path(temporary)
        for case in CASES:
            baseline = compile_and_run(args.lccc, case, directory, False, gcc_include, stats=True)
            feature = compile_and_run(args.lccc, case, directory, True, gcc_include, stats=True)
            entry = {"name": case.name, "lccc_baseline": baseline, "lccc_feature": feature,
                     "oracles": [], "variants": []}
            if any(count > 0 for count in forwarding_counts(feature.get("compile_stderr", ""))):
                forwarding_witness = True
            if any(count > 0 for count in shrinkage_counts(feature.get("compile_stderr", ""))):
                shrinkage_witness = True
            if not args.skip_matrix:
                for label, flags in (("O0", ("-O0",)), ("O1", ("-O1",)),
                                     ("O3", ("-O3",)), ("PIC", ("-fPIC",))):
                    variant_baseline = compile_and_run_flags(
                        args.lccc, case, directory, False, gcc_include, extra_flags=flags)
                    variant_feature = compile_and_run_flags(
                        args.lccc, case, directory, True, gcc_include, stats=True,
                        extra_flags=flags)
                    if any(count > 0 for count in forwarding_counts(variant_feature.get("compile_stderr", ""))):
                        forwarding_witness = True
                    if any(count > 0 for count in shrinkage_counts(variant_feature.get("compile_stderr", ""))):
                        shrinkage_witness = True
                    variant = {"name": label, "flags": flags,
                               "baseline": variant_baseline, "feature": variant_feature}
                    entry["variants"].append(variant)
                    for record in (variant_baseline, variant_feature):
                        if record.get("compile_returncode") != 0:
                            failures.append(f"{case.name}/{label}: LCCC compilation failed")
                        elif record.get("runtime_returncode") != 0:
                            failures.append(f"{case.name}/{label}: LCCC runtime failed")
                    if (variant_baseline.get("stdout") != variant_feature.get("stdout")
                            or variant_baseline.get("runtime_returncode") != variant_feature.get("runtime_returncode")):
                        failures.append(f"{case.name}/{label}: baseline/feature output divergence")
            for record in (baseline, feature):
                if record.get("compile_returncode") != 0:
                    failures.append(f"{case.name}: LCCC compilation failed")
                elif record.get("runtime_returncode") != 0:
                    failures.append(f"{case.name}: LCCC runtime failed")
            if baseline.get("stdout") != feature.get("stdout") or baseline.get("runtime_returncode") != feature.get("runtime_returncode"):
                failures.append(f"{case.name}: baseline/feature output divergence")
            if not args.skip_oracles:
                for oracle in oracle_compilers(args):
                    # ICX's default FP contraction/value model is not a
                    # strict correctness oracle; these fixtures are integer
                    # only, but keep the strict policy explicit for future
                    # floating-point cases.
                    oracle_result = compile_and_run(oracle, case, directory, False, gcc_include)
                    entry["oracles"].append(oracle_result)
                    if oracle_result.get("compile_returncode") == 0:
                        if oracle_result.get("runtime_returncode") != 0:
                            failures.append(f"{case.name}: {oracle} runtime failed")
                        if (oracle_result.get("stdout") != baseline.get("stdout")
                                or oracle_result.get("runtime_returncode") != baseline.get("runtime_returncode")):
                            failures.append(f"{case.name}: {oracle} output differs from LCCC baseline")
                    else:
                        failures.append(f"{case.name}: {oracle} compilation failed")
            report["cases"].append(entry)

        if not forwarding_witness:
            failures.append("no IPA forwarding opportunity was exercised in the correctness matrix")
        if not shrinkage_witness:
            failures.append("no live-range shrinkage opportunity was exercised in the correctness matrix")

        if not args.skip_performance:
            entries = {entry["name"]: entry for entry in report["cases"]}
            for case in CASES:
                if not case.performance:
                    continue
                entry = entries[case.name]
                records = {
                    "baseline": entry["lccc_baseline"],
                    "feature": entry["lccc_feature"],
                }
                for oracle in entry.get("oracles", []):
                    records[f"oracle:{oracle['compiler']}"] = oracle
                executables = {
                    label: Path(record["executable"])
                    for label, record in records.items()
                    if record.get("compile_returncode") == 0
                    and record.get("executable")
                    and Path(record["executable"]).exists()
                }
                if "baseline" not in executables or "feature" not in executables:
                    continue
                timings = {label: [] for label in executables}
                labels = list(executables)
                # Rotate the complete set, rather than timing one compiler
                # in a separate thermal/scheduling phase. Compiler-specific
                # executable names also prevent an oracle from overwriting
                # the LCCC baseline before this loop.
                for rep in range(args.reps):
                    order = labels[rep % len(labels):] + labels[:rep % len(labels)]
                    for label in order:
                        started = time.perf_counter()
                        result = run([str(executables[label])], timeout=60)
                        elapsed = time.perf_counter() - started
                        if result.returncode == 0:
                            timings[label].append(elapsed)
                medians = {
                    label: statistics.median(values) if values else None
                    for label, values in timings.items()
                }
                performance = {
                    "case": case.name,
                    "repetitions": args.reps,
                    "seconds": timings,
                    "median_seconds": medians,
                    "min_seconds": {k: min(v) if v else None for k, v in timings.items()},
                    "max_seconds": {k: max(v) if v else None for k, v in timings.items()},
                    "median_absolute_deviation": {
                        k: statistics.median([abs(x - medians[k]) for x in v]) if v else None
                        for k, v in timings.items()
                    },
                }
                if timings["baseline"] and timings["feature"]:
                    performance["feature_over_baseline"] = (
                        medians["feature"] / medians["baseline"]
                    )
                    performance["feature_over_oracle"] = {
                        label: medians["feature"] / medians[label]
                        for label in labels
                        if label.startswith("oracle:") and medians[label]
                    }
                report["performance"].append(performance)

    report["failures"] = failures
    if failures:
        if not forwarding_witness or not shrinkage_witness:
            for entry in report["cases"]:
                print(f"IPA/LRS STATS {entry['name']}:\n{entry['lccc_feature'].get('compile_stderr', '')[-8000:]}", file=sys.stderr)
        def diagnostic_records(entry):
            yield entry["lccc_baseline"]
            yield entry["lccc_feature"]
            for variant in entry.get("variants", []):
                yield variant["baseline"]
                yield variant["feature"]
            yield from entry.get("oracles", [])
        for entry in report["cases"]:
            for record in diagnostic_records(entry):
                if record.get("compile_returncode"):
                    detail = record.get("compile_stderr", "")[-4000:]
                    print(f"DIAGNOSTIC {entry['name']} {record['compiler']}:\n{detail}", file=sys.stderr)
                elif record.get("runtime_returncode"):
                    detail = record.get("stderr", "")[-4000:]
                    print(f"RUNTIME DIAGNOSTIC {entry['name']} {record['compiler']}:\n{detail}", file=sys.stderr)
        if os.environ.get("GITHUB_ACTIONS"):
            for failure in failures:
                print(f"::error title=IPA/LRS harness::{escape_annotation(failure)}")
            if not forwarding_witness or not shrinkage_witness:
                # Hosted log/artifact downloads are not available in every
                # CI environment. Keep the decisive counter evidence in the
                # check-run annotations as well as the JSON report.
                for entry in report["cases"]:
                    records = [entry["lccc_feature"]]
                    records.extend(v["feature"] for v in entry.get("variants", []))
                    stats = []
                    for record in records:
                        for line in record.get("compile_stderr", "").splitlines():
                            if "[IPA-PTA]" in line or "[LR-SHRINK]" in line:
                                stats.append(line)
                    if stats:
                        detail = f"{entry['name']}: " + " | ".join(stats[-24:])
                        print(f"::error title=IPA/LRS statistics::{escape_annotation(detail)}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    if os.environ.get("GITHUB_ACTIONS"):
        # Keep measurements discoverable even when the hosted artifact
        # download endpoint is unavailable. These are notices, not assertions
        # of superiority; the JSON retains every repetition for analysis.
        for entry in report["cases"]:
            records = [entry["lccc_feature"]]
            records.extend(v["feature"] for v in entry.get("variants", []))
            ipa = [count for record in records for count in forwarding_counts(record.get("compile_stderr", ""))]
            lrs = [count for record in records for count in shrinkage_counts(record.get("compile_stderr", ""))]
            if any(ipa) or any(lrs):
                detail = f"{entry['name']}: IPA forwarded max={max(ipa or [0])}; LRS changed max={max(lrs or [0])}"
                print(f"::notice title=IPA/LRS optimization witness::{escape_annotation(detail)}")
        context = {
            "repetitions": args.reps,
            "host": {k: v for k, v in report["host"].items()
                     if k in {"platform", "machine", "model name", "Hardware"}},
            "compilers": report["compilers"],
        }
        print(f"::notice title=IPA/LRS measurement context::{escape_annotation(json.dumps(context, sort_keys=True))}")
        for item in report["performance"]:
            detail = json.dumps({
                "case": item["case"],
                "repetitions": item["repetitions"],
                "feature_over_baseline": item.get("feature_over_baseline"),
                "feature_over_oracle": item.get("feature_over_oracle", {}),
                "median_seconds": item["median_seconds"],
                "median_absolute_deviation": item["median_absolute_deviation"],
            }, sort_keys=True)
            print(f"::notice title=IPA/LRS measured performance::{escape_annotation(detail)}")
    print(f"IPA/LRS cases: {len(CASES)}; failures: {len(failures)}")
    for item in report["performance"]:
        print(item["case"], item.get("feature_over_baseline"), item["median_seconds"])
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"report: {args.output}", file=sys.stderr)
        return 1
    print(f"report: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
