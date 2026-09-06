#!/usr/bin/env python3
"""Unified Differential Testing & Fuzzing Harness for LCCC.

Performs multi-threaded differential fuzzing and execution validation between
LCCC and reference compilers (GCC, Clang) across multiple optimization levels.

Engines Supported:
  1. synthetic    : Built-in random C generator (zero external dependencies).
  2. csmith       : Csmith random C program generator (if installed).
  3. yarpgen      : YARPGen LLVM/GCC/CCC differential generator (if installed).
  4. stress_suite : Internal suite of specialized stress test generators.

Features:
  - Exact stdout/exit-status checksum validation across ALL selected
    reference compilers (a divergence against any reference fails).
  - Multi-threaded worker pool with per-iteration timeout and memory bounds.
  - Automatic isolation and reproducer preservation into `artifacts/repros/`.
  - Multi-architecture support: x86_64, i686, aarch64, riscv64 (with qemu runners).
  - JSON output summary for automated CI gates.
  - `--check-engines` wiring self-test: every advertised engine must be a real
    implementation; an unimplemented engine is a hard error, never a silent
    zero-test "success".

Legacy compatibility:
  `scripts/csmith_diff.py` and `scripts/yarpgen_diff.py` forward here after
  translating their historical flag spellings (--ccc, --clang, --gcc, --jobs,
  --tests, --seed-start, --include, --out-dir, ...).  The historical
  `--tests 0` (run forever) spelling is honoured via `--count 0`.

Examples:
  scripts/fuzz_diff.py --engine synthetic --count 100 -j4
  scripts/fuzz_diff.py --engine stress_suite -j2
  scripts/fuzz_diff.py --engine csmith --csmith /usr/bin/csmith --count 50
  scripts/fuzz_diff.py --engine synthetic --arch i686 --runner qemu-i386
  scripts/fuzz_diff.py --check-engines
"""
from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parent.parent
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc"
DEFAULT_REPRO_DIR = REPO / "artifacts" / "repros"


# ─────────────────────────────────────────────────────────────────────────────
# Synthetic Random C Code Generator
# ─────────────────────────────────────────────────────────────────────────────

class SyntheticCGenerator:
    """Generates deterministic, non-UB randomized C programs for differential testing."""

    def __init__(self, seed: int):
        self.rng = random.Random(seed)
        self.seed = seed

    def generate(self) -> str:
        r = self.rng
        num_globals = r.randint(4, 10)
        num_funcs = r.randint(2, 5)

        lines: list[str] = [
            "/* Synthetic differential test program */",
            f"/* Seed: {self.seed} */",
            "#include <stdint.h>",
            "#include <stdio.h>",
            "#include <string.h>",
            "",
        ]

        # Global variables
        for i in range(num_globals):
            ty = r.choice(["int32_t", "uint32_t", "int64_t", "uint64_t", "uint8_t", "int16_t"])
            val = r.randint(0, 0xFFFF)
            lines.append(f"static volatile {ty} g_{i} = {val};")
        lines.append("")

        # Global struct definition
        lines.append("struct S0 {")
        lines.append("    int32_t a;")
        lines.append("    uint16_t b;")
        lines.append("    int64_t c;")
        lines.append("    uint8_t d[4];")
        lines.append("};")
        lines.append("static struct S0 g_s0 = { 1234, 56, 789012345678ULL, { 1, 2, 3, 4 } };")
        lines.append("")

        # Helper functions
        for f_idx in range(num_funcs):
            lines.append(f"static int64_t func_{f_idx}(int64_t a, int64_t b, int32_t c) {{")
            lines.append("    int64_t acc = a ^ (b << 3);")
            lines.append("    int32_t arr[8];")
            lines.append("    for (int i = 0; i < 8; i++) arr[i] = (int32_t)(c + i * 17);")
            lines.append("    for (int i = 0; i < 4; i++) {")
            lines.append("        acc += arr[i] * (arr[7 - i] + 1);")
            lines.append("        acc = (acc >> 1) ^ (acc << 5);")
            lines.append("    }")
            lines.append(f"    g_{f_idx % num_globals} += (int32_t)acc;")
            lines.append("    return acc ^ g_s0.c;")
            lines.append("}")
            lines.append("")

        # Checksum calculation & main
        lines.append("static uint64_t crc = 0x123456789ABCDEF0ULL;")
        lines.append("static void crc_update(uint64_t val) {")
        lines.append("    crc = (crc ^ val) * 0x100000001B3ULL + 0xCBF29CE484222325ULL;")
        lines.append("}")
        lines.append("")
        lines.append("int main(void) {")
        lines.append("    int64_t x = 42, y = 1337;")
        lines.append("    for (int iter = 0; iter < 20; iter++) {")
        for f_idx in range(num_funcs):
            lines.append(f"        crc_update((uint64_t)func_{f_idx}(x, y, (int32_t)iter));")
        lines.append("        x = (int64_t)(crc & 0xFFFFFF);")
        lines.append("        y = (int64_t)(g_s0.a + (int32_t)iter);")
        lines.append("    }")
        lines.append("    crc_update((uint64_t)g_s0.a);")
        lines.append("    crc_update((uint64_t)g_s0.b);")
        lines.append("    crc_update((uint64_t)g_s0.c);")
        lines.append("    for (int i = 0; i < 4; i++) crc_update((uint64_t)g_s0.d[i]);")
        lines.append("    printf(\"CRC: %016llx\\n\", (unsigned long long)crc);")
        lines.append("    return 0;")
        lines.append("}")

        return "\n".join(lines) + "\n"


# ─────────────────────────────────────────────────────────────────────────────
# Test Execution & Differential Verification
# ─────────────────────────────────────────────────────────────────────────────

@dataclass
class DiffResult:
    test_id: str
    status: str  # "PASS", "MISCOMPILE", "LCCC_CRASH", "REF_CRASH", "TIMEOUT", "SKIP"
    message: str
    source_path: str | None = None
    lccc_stdout: str = ""
    ref_stdout: str = ""
    opt_level: str = "-O2"


@dataclass
class CompileUnit:
    """One generated test case: sources (relative names), extra compile flags."""
    files: dict[str, str] = field(default_factory=dict)  # relative name -> contents
    extra_flags: list[str] = field(default_factory=list)
    keep_dir: Path | None = None  # when set, the case dir must be preserved


def run_command(cmd: list[str], timeout: float = 30.0, cwd: Path | None = None) -> tuple[int, str, str]:
    """Run a command with a timeout.

    The child runs in its own process group so that a timeout kills the whole
    group: generated stress programs spawn their own children, and an orphaned
    grandchild would keep burning CPU and contaminate every later measurement
    on the host (this actually happened: timed-out unroll_stress arms survived
    their parent and polluted a benchmark run).
    """
    import signal
    try:
        p = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            cwd=cwd,
            start_new_session=True,  # new process group = we can kill the tree
        )
        try:
            out, err = p.communicate(timeout=timeout)
            return p.returncode, out, err
        except subprocess.TimeoutExpired:
            # Kill the entire process group (negative PID), then reap.
            try:
                os.killpg(os.getpgid(p.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                p.kill()
            try:
                out, err = p.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                out, err = "", ""
            return 124, out or "", (err or "timeout expired")
    except Exception as e:
        return -1, "", str(e)


def evaluate_single_test(
    test_id: str,
    unit: CompileUnit,
    lccc_path: str,
    ref_compilers: list[str],
    opt_levels: list[str],
    runner: str | None,
    extra_cflags: list[str],
    repro_dir: Path,
    compile_timeout: float,
    run_timeout: float,
) -> DiffResult:
    with tempfile.TemporaryDirectory(prefix=f"diff_{test_id}_") as tmpdir:
        tmp = Path(tmpdir)
        for name, contents in unit.files.items():
            dest = tmp / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(contents)

        source_names = sorted(unit.files)
        primary = source_names[0]

        for opt in opt_levels:
            # Baseline: build+run with EVERY reference compiler; all must
            # agree on (exit status, stdout, stderr) before LCCC is judged.
            ref_runs: dict[str, tuple[int, str, str]] = {}
            ref_baseline: tuple[int, str, str] | None = None
            for ref_cc in ref_compilers:
                ref_bin = tmp / f"ref_{ref_cc.replace('/', '_')}_{opt}"
                ref_build_cmd = [ref_cc, opt, *extra_cflags, *unit.extra_flags,
                                 *source_names, "-o", str(ref_bin), "-lm"]
                rc, out, err = run_command(ref_build_cmd, timeout=compile_timeout, cwd=tmp)
                if rc != 0:
                    return DiffResult(test_id, "SKIP", f"reference {ref_cc} build failed: {err.strip()}",
                                      opt_level=opt)
                ref_run_cmd = ([runner] if runner else []) + [str(ref_bin)]
                ref_rc, ref_out, ref_err = run_command(ref_run_cmd, timeout=run_timeout)
                if ref_rc == 124:
                    return DiffResult(test_id, "SKIP", f"reference {ref_cc} timed out", opt_level=opt)
                if ref_rc != 0:
                    return DiffResult(test_id, "SKIP",
                                      f"reference {ref_cc} non-zero exit ({ref_rc})", opt_level=opt)
                run_tuple = (ref_rc, ref_out, ref_err)
                if ref_baseline is None:
                    ref_baseline = run_tuple
                elif run_tuple != ref_baseline:
                    return DiffResult(test_id, "SKIP",
                                      f"reference compilers disagree ({ref_cc} differs); not a valid oracle",
                                      opt_level=opt)
                ref_runs[ref_cc] = run_tuple

            assert ref_baseline is not None  # ref_compilers is non-empty by construction

            # Build and run with LCCC
            lccc_bin = tmp / f"lccc_{opt}"
            lccc_build_cmd = [lccc_path, opt, *extra_cflags, *unit.extra_flags,
                              *source_names, "-o", str(lccc_bin), "-lm"]
            rc, out, err = run_command(lccc_build_cmd, timeout=compile_timeout, cwd=tmp)
            if rc != 0:
                repro_file = repro_dir / f"crash_{test_id}_{opt}.c"
                repro_dir.mkdir(parents=True, exist_ok=True)
                repro_file.write_text(unit.files[primary])
                return DiffResult(
                    test_id,
                    "LCCC_CRASH",
                    f"lccc compile error: {err.strip()[:200]}",
                    str(repro_file),
                    opt_level=opt,
                )

            lccc_run_cmd = ([runner] if runner else []) + [str(lccc_bin)]
            lccc_rc, lccc_out, lccc_err = run_command(lccc_run_cmd, timeout=run_timeout)

            if lccc_rc != ref_baseline[0] or lccc_out != ref_baseline[1] or lccc_err != ref_baseline[2]:
                repro_file = repro_dir / f"miscompile_{test_id}_{opt}.c"
                repro_dir.mkdir(parents=True, exist_ok=True)
                repro_file.write_text(unit.files[primary])
                msg = (f"output mismatch vs {','.join(ref_compilers)}: "
                       f"ref={ref_baseline[1].strip()!r} (rc={ref_baseline[0]}) "
                       f"vs lccc={lccc_out.strip()!r} (rc={lccc_rc})")
                return DiffResult(
                    test_id,
                    "MISCOMPILE",
                    msg,
                    str(repro_file),
                    lccc_stdout=lccc_out,
                    ref_stdout=ref_baseline[1],
                    opt_level=opt,
                )

        return DiffResult(test_id, "PASS", "All optimization levels matched every reference")


def run_synthetic_pool(
    count: int,
    seed: int,
    workers: int,
    evaluate,
) -> list[DiffResult]:
    results: list[DiffResult] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures: list[concurrent.futures.Future] = []
        iteration = 0
        # count == 0 means "run forever" (historical --tests 0 spelling).
        # In-flight submissions are bounded so infinite mode cannot grow the
        # pending-future queue without limit.
        while iteration < count or count == 0:
            if count == 0 and len(futures) >= max(1, workers * 2):
                done = [f for f in futures if f.done()]
                for f in done:
                    results.append(f.result())
                    futures.remove(f)
                if not done:
                    time.sleep(0.05)
                continue
            case_seed = seed + iteration
            test_id = f"synth_{case_seed}"
            unit = CompileUnit(files={"test.c": SyntheticCGenerator(case_seed).generate()})
            futures.append(pool.submit(evaluate, test_id, unit))
            iteration += 1
            # For infinite mode, harvest completed futures as we go.
            if count == 0:
                done = [f for f in futures if f.done()]
                for f in done:
                    results.append(f.result())
                    futures.remove(f)

        done_count = 0
        for f in concurrent.futures.as_completed(futures):
            res = f.result()
            results.append(res)
            done_count += 1
            status_char = "." if res.status == "PASS" else "X"
            print(status_char, end="", flush=True)
            if done_count % 50 == 0 or done_count == len(futures):
                print(f" [{done_count}/{len(futures)}]")
    return results


# ─────────────────────────────────────────────────────────────────────────────
# Csmith engine
# ─────────────────────────────────────────────────────────────────────────────

def run_csmith_pool(
    count: int,
    seed: int,
    workers: int,
    evaluate,
    csmith_bin: str,
    include_dirs: list[str],
    compile_timeout: float,
) -> list[DiffResult]:
    if not shutil.which(csmith_bin) and not Path(csmith_bin).is_file():
        sys.exit(
            f"error: csmith binary '{csmith_bin}' not found.\n"
            "        Install csmith (https://github.com/csmith-project/csmith) or use\n"
            "        --engine synthetic for the dependency-free generator."
        )

    def generate(test_id: str) -> CompileUnit:
        case_seed = int(test_id.split("_", 1)[1])
        rc, out, err = run_command([csmith_bin, "--seed", str(case_seed)], timeout=compile_timeout)
        if rc != 0:
            raise RuntimeError(f"csmith generation failed (seed {case_seed}): {err.strip()[:200]}")
        flags = [f"-I{d}" for d in include_dirs]
        return CompileUnit(files={"test.c": out}, extra_flags=flags)

    return _generate_and_evaluate("csmith", count, seed, workers, evaluate, generate)


# ─────────────────────────────────────────────────────────────────────────────
# YARPGen engine
# ─────────────────────────────────────────────────────────────────────────────

def run_yarpgen_pool(
    count: int,
    seed: int,
    workers: int,
    evaluate,
    yarpgen_bin: str,
    compile_timeout: float,
) -> list[DiffResult]:
    if not shutil.which(yarpgen_bin) and not Path(yarpgen_bin).is_file():
        sys.exit(
            f"error: yarpgen binary '{yarpgen_bin}' not found.\n"
            "        Build YARPGen (https://github.com/intel/yarpgen) or use\n"
            "        --engine synthetic for the dependency-free generator."
        )

    def generate(test_id: str) -> CompileUnit:
        case_seed = int(test_id.split("_", 1)[1])
        with tempfile.TemporaryDirectory(prefix=f"yarpgen_{case_seed}_") as gendir:
            rc, out, err = run_command(
                [yarpgen_bin, "--std=c", "--seed", str(case_seed), "-o", gendir],
                timeout=compile_timeout,
            )
            if rc != 0:
                raise RuntimeError(f"yarpgen generation failed (seed {case_seed}): {(err or out).strip()[:200]}")
            files: dict[str, str] = {}
            for name in ("driver.c", "func.c", "proto.h", "data.h"):
                p = Path(gendir) / name
                if p.is_file():
                    files[name] = p.read_text(errors="replace")
        if "driver.c" not in files or "func.c" not in files:
            raise RuntimeError(f"yarpgen did not produce driver.c/func.c (seed {case_seed})")
        # YARPGen programs target C99 semantics.
        return CompileUnit(files=files, extra_flags=["-std=c99", "-w"])

    return _generate_and_evaluate("yarpgen", count, seed, workers, evaluate, generate)


def _generate_and_evaluate(engine: str, count: int, seed: int, workers: int, evaluate, generate) -> list[DiffResult]:
    """Shared pool driver: generation in worker threads, then evaluation."""
    results: list[DiffResult] = []
    lock_print = lambda msg: print(msg, end="", flush=True)  # noqa: E731

    def one(iteration: int) -> DiffResult:
        test_id = f"{engine}_{seed + iteration}"
        try:
            unit = generate(test_id)
        except RuntimeError as e:
            return DiffResult(test_id, "SKIP", str(e))
        return evaluate(test_id, unit)

    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures: list[concurrent.futures.Future] = []
        iteration = 0
        while iteration < count or count == 0:
            if count == 0 and len(futures) >= max(1, workers * 2):
                done = [f for f in futures if f.done()]
                for f in done:
                    results.append(f.result())
                    futures.remove(f)
                if not done:
                    time.sleep(0.05)
                continue
            futures.append(pool.submit(one, iteration))
            iteration += 1
            if count == 0:
                done = [f for f in futures if f.done()]
                for f in done:
                    results.append(f.result())
                    futures.remove(f)

        done_count = 0
        for f in concurrent.futures.as_completed(futures):
            res = f.result()
            results.append(res)
            done_count += 1
            lock_print("." if res.status == "PASS" else "X")
            if done_count % 50 == 0 or done_count == len(futures):
                print(f" [{done_count}/{len(futures)}]")
    return results


# ─────────────────────────────────────────────────────────────────────────────
# Stress suite engine
# ─────────────────────────────────────────────────────────────────────────────

def run_stress_suite(
    lccc_path: str,
    ref_compilers: list[str],
    opt_levels: list[str],
    runner: str | None,
    repro_dir: Path,
    compile_timeout: float,
    run_timeout: float,
    seed: int = 0,
) -> list[DiffResult]:
    """Runs repository generator scripts to produce specialized stress test cases.

    Two script families are driven:

    * generator scripts that emit a random C program on stdout
      (``gen_fp_stress.py SEED``, ``gen_gep_chain_stress.py``,
      ``gen_slot_stress.py``) - each program is differentially evaluated
      against every reference compiler at every optimization level;
    * self-contained differential testers that own their own comparison
      (``unroll_stress.py --lccc ...``) - invoked directly, their exit code
      becomes the result.
    """
    results: list[DiffResult] = []

    def evaluate_stress(test_id: str, unit: CompileUnit) -> DiffResult:
        return evaluate_single_test(
            test_id,
            unit,
            lccc_path,
            ref_compilers,
            opt_levels,
            runner,
            [],
            repro_dir,
            compile_timeout,
            run_timeout,
        )

    # (name, script, argv-builder) for stdout generators.
    generators = [
        ("gen_fp_stress", REPO / "scripts" / "gen_fp_stress.py",
         lambda: [str(seed)]),
        ("gen_gep_stress", REPO / "scripts" / "gen_gep_chain_stress.py",
         lambda: []),
        ("gen_slot_stress", REPO / "scripts" / "gen_slot_stress.py",
         lambda: []),
    ]
    for name, script_path, argv_builder in generators:
        if not script_path.exists():
            continue
        with tempfile.TemporaryDirectory(prefix=f"stress_{name}_") as tmp:
            gen_cmd = [sys.executable, str(script_path), *argv_builder()]
            rc, code, err = run_command(gen_cmd, cwd=Path(tmp), timeout=compile_timeout)
            if rc != 0 or not code.strip():
                # Some scripts output files rather than stdout
                generated_files = list(Path(tmp).glob("*.c"))
                if generated_files:
                    code = generated_files[0].read_text()
                else:
                    results.append(DiffResult(name, "SKIP", f"generator failed: {err}"))
                    continue

            res = evaluate_stress(name, CompileUnit(files={"test.c": code}))
            results.append(res)

    # Self-contained differential tester: its own exit code is the verdict.
    # Bounded sample (12 configurations in one TU = 3 compile+run arms); the
    # full shape-space sweep is a direct `unroll_stress.py` invocation.
    unroll = REPO / "scripts" / "unroll_stress.py"
    if unroll.exists():
        rc, out, err = run_command(
            [sys.executable, str(unroll), "--lccc", lccc_path, "--quiet",
             "--limit", "12", "--batch", "12", "--seed", str(seed)],
            timeout=max(compile_timeout, 300.0),
        )
        if rc == 0:
            results.append(DiffResult("unroll_stress", "PASS", "self-contained differential tester passed"))
        elif rc == 124:
            results.append(DiffResult("unroll_stress", "SKIP", "self-contained tester timed out"))
        else:
            results.append(DiffResult(
                "unroll_stress", "LCCC_CRASH",
                f"self-contained tester failed (rc={rc}): {(err or out).strip()[:300]}"))

    if not results:
        # Every generator missing is a wiring problem, not a silent pass.
        sys.exit("error: stress_suite produced no cases (no generator scripts found)")
    return results


# ─────────────────────────────────────────────────────────────────────────────
# Legacy flag translation (csmith_diff.py / yarpgen_diff.py compatibility)
# ─────────────────────────────────────────────────────────────────────────────

_LEGACY_PASSTHROUGH = {
    "--compile-timeout", "--run-timeout", "--include", "--csmith", "--yarpgen",
    "--refs", "--repro-dir", "--json", "--runner", "-j",
}
_LEGACY_IGNORED_WITH_WARNING = {
    # Historical knobs of the pre-unification standalone testers that have no
    # unified-harness equivalent.  Accepted (so documented invocations keep
    # working) but reported, never silently dropped.
    "--keep-passing": "case retention is now automatic for failures only",
    "--keep-skipped": "skipped cases are always discarded",
    "--progress-every": "progress is printed per completion",
    "--work-root": "the unified harness manages its own tempdirs",
    "--keep-all": "case retention is now automatic for failures only",
    "--continue-on-divergence": "the unified harness always continues and reports",
}


def translate_legacy_args(argv: list[str], *, default_count: int | None = None) -> list[str]:
    """Translate the historical csmith_diff/yarpgen_diff flag spellings.

    The pre-unification testers accepted --ccc/--clang/--gcc/--jobs/--tests/
    --seed-start/--out-dir.  Their wrapper successors forward argv after this
    translation so every documented historical invocation keeps working.
    """
    out: list[str] = []
    refs: list[str] = []
    count_seen = False
    i = 0
    while i < len(argv):
        a = argv[i]

        def value() -> str:
            nonlocal i
            if i + 1 >= len(argv):
                sys.exit(f"error: {a} requires a value")
            i += 1
            return argv[i]

        if a == "--ccc":
            out.extend(["--lccc", value()])
        elif a in ("--clang", "--gcc"):
            refs.append(value())
        elif a == "--jobs":
            out.extend(["-j", value()])
        elif a == "--tests":
            out.extend(["--count", value()])
            count_seen = True
        elif a == "--seed-start":
            out.extend(["--seed", value()])
        elif a == "--out-dir":
            out.extend(["--repro-dir", value()])
        elif a in _LEGACY_IGNORED_WITH_WARNING:
            print(f"note: {a} is accepted for compatibility but ignored: "
                  f"{_LEGACY_IGNORED_WITH_WARNING[a]}", file=sys.stderr)
        elif a in _LEGACY_PASSTHROUGH or a.startswith("--"):
            out.append(a)
            if a in ("--include", "--csmith", "--yarpgen", "--refs", "--repro-dir",
                     "--json", "--runner", "--compile-timeout", "--run-timeout"):
                out.append(value())
        else:
            out.append(a)
        i += 1

    if refs:
        # Historical default order was clang,gcc; explicit --refs wins when
        # neither --clang nor --gcc was given.
        out.extend(["--refs", ",".join(refs)])
    if default_count is not None and not count_seen and "--count" not in out:
        out.extend(["--count", str(default_count)])
    return out


# ─────────────────────────────────────────────────────────────────────────────
# CLI
# ─────────────────────────────────────────────────────────────────────────────

IMPLEMENTED_ENGINES = ("synthetic", "csmith", "yarpgen", "stress_suite")


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument(
        "--engine",
        choices=IMPLEMENTED_ENGINES,
        default="synthetic",
        help="Differential generation engine (default: synthetic)",
    )
    ap.add_argument(
        "--count",
        type=int,
        default=50,
        help="Number of test iterations to generate and evaluate (default: 50; 0 = infinite)",
    )
    ap.add_argument(
        "-j",
        "--workers",
        type=int,
        default=min(4, os.cpu_count() or 2),
        help="Parallel test worker count",
    )
    ap.add_argument(
        "--lccc",
        default=os.environ.get("LCCC", str(DEFAULT_LCCC)),
        help="Path to LCCC compiler binary",
    )
    ap.add_argument(
        "--refs",
        default="gcc,clang",
        help="Comma-separated reference compilers (default: gcc,clang)",
    )
    ap.add_argument(
        "--opts",
        default="-O0,-O1,-O2,-O3",
        help="Comma-separated optimization levels to verify (default: -O0,-O1,-O2,-O3)",
    )
    ap.add_argument(
        "--arch",
        choices=["x86_64", "i686", "aarch64", "riscv64"],
        default="x86_64",
        help="Target architecture (default: x86_64)",
    )
    ap.add_argument(
        "--runner",
        help="Emulator/runner prefix for executing binaries (e.g. qemu-i386)",
    )
    ap.add_argument(
        "--csmith",
        help="Explicit path to csmith binary",
    )
    ap.add_argument(
        "--include",
        action="append",
        default=[],
        dest="include_dirs",
        help="Extra -I include directory for generated sources (csmith runtime); repeatable",
    )
    ap.add_argument(
        "--yarpgen",
        help="Explicit path to yarpgen binary",
    )
    ap.add_argument(
        "--repro-dir",
        default=str(DEFAULT_REPRO_DIR),
        help="Directory to save minimized failure reproducers",
    )
    ap.add_argument(
        "--json",
        dest="json_out",
        help="Save test results summary to JSON file",
    )
    ap.add_argument(
        "--seed",
        type=int,
        default=int(time.time()),
        help="Initial random seed",
    )
    ap.add_argument(
        "--compile-timeout",
        type=float,
        default=30.0,
        help="Per-compiler compile timeout in seconds (default: 30)",
    )
    ap.add_argument(
        "--run-timeout",
        type=float,
        default=10.0,
        help="Per-binary run timeout in seconds (default: 10)",
    )
    ap.add_argument(
        "--check-engines",
        action="store_true",
        help="Verify every advertised engine is implemented (CI wiring guard) and exit",
    )
    return ap.parse_args()


def check_engines() -> int:
    """CI guard: every advertised engine must have a real pool implementation.

    This exists because the csmith/yarpgen engines once regressed to silent
    no-op stubs that reported TOTAL: 0 / exit 0 - a validation vacuum that no
    test caught.  The dispatch table below is the single source of truth; a
    missing entry fails loudly.
    """
    dispatch = {
        "synthetic": run_synthetic_pool,
        "csmith": run_csmith_pool,
        "yarpgen": run_yarpgen_pool,
        "stress_suite": run_stress_suite,
    }
    missing = [e for e in IMPLEMENTED_ENGINES if e not in dispatch]
    if missing:
        print(f"error: engines advertised but not implemented: {', '.join(missing)}")
        return 1
    print(f"all {len(IMPLEMENTED_ENGINES)} engines implemented: {', '.join(IMPLEMENTED_ENGINES)}")
    return 0


def main() -> int:
    args = parse_args()

    if args.check_engines:
        return check_engines()

    if not Path(args.lccc).is_file():
        sys.exit(f"error: LCCC binary '{args.lccc}' not found. Build first!")

    ref_compilers = [c.strip() for c in args.refs.split(",") if c.strip()]
    available_refs = [c for c in ref_compilers if shutil.which(c) or Path(c).is_file()]
    if not available_refs:
        sys.exit(f"error: none of the reference compilers ({args.refs}) found in PATH")

    opt_levels = [o.strip() for o in args.opts.split(",") if o.strip()]

    # i686 targets need -m32 on the host references for a like-for-like oracle.
    arch_flags: list[str] = []
    if args.arch == "i686":
        arch_flags = ["-m32"]

    print("=" * 70)
    print(" LCCC Differential Fuzzing & Testing Harness")
    print(f" Engine   : {args.engine}")
    print(f" Target   : {args.arch} | Runner: {args.runner or 'native'}")
    print(f" LCCC     : {args.lccc}")
    print(f" Reference: {', '.join(available_refs)}")
    print(f" Opt levels: {', '.join(opt_levels)}")
    print(f" Workers  : {args.workers} | Count: {args.count or 'infinite'} | Seed: {args.seed}")
    print("=" * 70)

    repro_dir = Path(args.repro_dir)

    def evaluate(test_id: str, unit: CompileUnit) -> DiffResult:
        return evaluate_single_test(
            test_id,
            unit,
            args.lccc,
            available_refs,
            opt_levels,
            args.runner,
            arch_flags,
            repro_dir,
            args.compile_timeout,
            args.run_timeout,
        )

    t0 = time.time()
    results: list[DiffResult]

    if args.engine == "stress_suite":
        results = run_stress_suite(
            args.lccc, available_refs, opt_levels, args.runner, repro_dir,
            args.compile_timeout, args.run_timeout, seed=args.seed,
        )
    elif args.engine == "synthetic":
        results = run_synthetic_pool(args.count, args.seed, args.workers, evaluate)
    elif args.engine == "csmith":
        csmith_bin = args.csmith or os.environ.get("CSMITH", "csmith")
        results = run_csmith_pool(
            args.count, args.seed, args.workers, evaluate,
            csmith_bin, args.include_dirs, args.compile_timeout,
        )
    elif args.engine == "yarpgen":
        yarpgen_bin = args.yarpgen or os.environ.get("YARPGEN", "yarpgen")
        results = run_yarpgen_pool(
            args.count, args.seed, args.workers, evaluate,
            yarpgen_bin, args.compile_timeout,
        )
    else:  # pragma: no cover - argparse restricts choices
        sys.exit(f"error: engine '{args.engine}' is not implemented; "
                 f"implemented engines: {', '.join(IMPLEMENTED_ENGINES)}")

    elapsed = time.time() - t0
    pass_cnt = sum(1 for r in results if r.status == "PASS")
    fail_cnt = sum(1 for r in results if r.status in ("MISCOMPILE", "LCCC_CRASH"))
    skip_cnt = sum(1 for r in results if r.status == "SKIP")

    print("\n" + "=" * 70)
    print(f" Differential Fuzzing Results ({elapsed:.2f}s elapsed)")
    print(f" TOTAL : {len(results)}")
    print(f" PASS  : {pass_cnt}")
    print(f" FAIL  : {fail_cnt}")
    print(f" SKIP  : {skip_cnt}")
    print("=" * 70)

    if fail_cnt > 0:
        print("\nFailures:")
        for r in results:
            if r.status in ("MISCOMPILE", "LCCC_CRASH"):
                print(f"  [{r.status}] {r.test_id} ({r.opt_level}): {r.message}")
                if r.source_path:
                    print(f"    Reproducer saved at: {r.source_path}")

    if args.json_out:
        payload = {
            "engine": args.engine,
            "arch": args.arch,
            "total": len(results),
            "pass": pass_cnt,
            "fail": fail_cnt,
            "skip": skip_cnt,
            "elapsed_sec": elapsed,
            "results": [
                {
                    "test_id": r.test_id,
                    "status": r.status,
                    "message": r.message,
                    "opt_level": r.opt_level,
                    "source_path": r.source_path,
                }
                for r in results
            ],
        }
        Path(args.json_out).write_text(json.dumps(payload, indent=2))
        print(f"Saved results JSON to {args.json_out}")

    return 1 if fail_cnt > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
