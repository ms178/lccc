#!/usr/bin/env python3
"""Unified Differential Testing & Fuzzing Harness for LCCC.

Performs multi-threaded differential fuzzing and execution validation between
LCCC and reference compilers (GCC, Clang) across multiple optimization levels.

Engines Supported:
  1. synthetic    : Built-in random C generator (zero external dependencies).
  2. csmith       : Csmith random C program generator (if installed).
  3. yarpgen      : YARPGen LLVM/GCC/CCC differential generator (if installed).
  4. stress_suite : Internal suite of specialized stress test generators.
                    Sweeps `--count` seeds (seed..seed+count-1, default 8)
                    per generator instead of one fixed seed, and evaluates
                    every generated case under the cartesian product of the
                    repeatable `--config-env KEY=VALUE` configuration axes
                    (both compilers, compile and run, under each combo).
  5. differential : tests/fuzz/differential_fuzz.py generator absorbed as an
                    engine (x86-64 TU soup: fixed-width arithmetic, CFG joins,
                    arrays, structs/bitfields, volatile, postdec, __int128;
                    historical -march=raptorlake flags, Oz->Os on references).
  6. phi_cfg      : tests/fuzz/phi_cfg_fuzz.py generator absorbed as an engine
                    (loop-carried values, branch diamonds, switch joins,
                    continue paths, postfix inc/dec for phi-web coalescing).
  7. intcmp_thread: tests/fuzz/fuzz_intcmp_thread.py generator absorbed as an
                    engine (merge-diamond int-phi compare shapes for the
                    bool_thread threading pass; default seed 20260829).
  8. m32          : forwards to tests/fuzz/m32_differential_fuzz.py (i686
                    -m32 vs gcc -m32, nostdlib int80 exit-fold oracle).
  9. regparm      : forwards to tests/fuzz/regparm_differential.py
                    (-mregparm=3 ABI variant of the m32 oracle).
 10. slot_rmw     : forwards to tests/fuzz/slot_rmw_differential.py (i686
                    slot read-modify-write collapse hinge hammering).
 11. alias_m32    : forwards to tests/fuzz/alias_fuzz_m32.py (adversarial
                    redundant-load elimination / GVN shapes, -m32).
 12. alu_torture  : forwards to tests/fuzz/alu_torture_m32.py (fixed i686 ALU
                    probe: clz/ctz/popcount/bswap, mul/div by constant, LEA;
                    runs once per level, `--count` has no effect).
 13. aarch64      : forwards to tests/fuzz/aarch64_fuzz.py (lccc-arm vs
                    aarch64-linux-gnu-gcc under qemu-aarch64; SKIPs
                    gracefully when the cross toolchain is absent).

The tests/fuzz engines use two absorption mechanisms, both preserving the
standalone scripts byte-for-byte: generator-config engines (5-7) import the
historical generator and evaluate its cases through this harness's oracle
(every reference compiler at every level, GEN-BUG tripwire, repro retention);
forwarding engines (8-13) launch the standalone differential tester - which
owns its nostdlib/int80 or cross-compilation pipeline - and adopt its exit
code as the verdict, exactly like the stress_suite's unroll_stress arm.  Each
of them keeps its historical defaults (seed span, levels, gcc oracle); m32
engines SKIP cleanly on hosts that cannot build/execute ELF32, and aarch64
SKIPs when a cross toolchain is missing.

Features:
  - Exact stdout/exit-status checksum validation across ALL selected
    reference compilers (a divergence against any reference fails).
  - Multi-threaded worker pool with per-iteration timeout and memory bounds.
  - Automatic isolation and reproducer preservation into `artifacts/repros/`.
  - Multi-architecture support: x86_64, i686, aarch64, riscv64 (with qemu runners).
  - Generator-bug tripwire: when a reference AND lccc die by the SAME signal,
    the generated program itself is invalid; the case is reported as GEN-BUG
    and counted as a failure instead of being silently SKIPped.
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
  scripts/fuzz_diff.py --engine differential --count 20 --seed 0
  scripts/fuzz_diff.py --engine slot_rmw --count 2 --seed 1 --refs gcc
  scripts/fuzz_diff.py --check-engines
"""
from __future__ import annotations

import argparse
import concurrent.futures
import functools
import importlib
import itertools
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
DEFAULT_LCCC_ARM = REPO / "target" / "fastbuild" / "lccc-arm"
DEFAULT_REPRO_DIR = REPO / "artifacts" / "repros"
FUZZ_DIR = REPO / "tests" / "fuzz"


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
    # PASS | MISCOMPILE | LCCC_CRASH | GEN-BUG | SKIP
    # (GEN-BUG: the generated program itself dies by a signal under both the
    # reference and lccc - a generator bug, counted as a failure.)
    status: str
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


def run_command(
    cmd: list[str],
    timeout: float = 30.0,
    cwd: Path | None = None,
    env_extra: dict[str, str] | None = None,
) -> tuple[int, str, str]:
    """Run a command with a timeout.

    `env_extra` (when given) is merged over the inherited environment; the
    stress-suite config axes use it so a whole evaluation - reference and
    lccc, compile and run - happens under one declared configuration.

    The child runs in its own process group so that a timeout kills the whole
    group: generated stress programs spawn their own children, and an orphaned
    grandchild would keep burning CPU and contaminate every later measurement
    on the host (this actually happened: timed-out unroll_stress arms survived
    their parent and polluted a benchmark run).
    """
    import signal
    env = None
    if env_extra:
        env = dict(os.environ)
        env.update(env_extra)
    try:
        p = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            cwd=cwd,
            env=env,
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


def _signal_name(rc: int) -> str:
    """Human description of a death-by-signal subprocess returncode."""
    if rc >= 0:
        return f"exit {rc}"
    try:
        import signal
        return f"signal {signal.Signals(-rc).name} ({-rc})"
    except ValueError:
        return f"signal {-rc}"


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
    env_extra: dict[str, str] | None = None,
    ref_level_map: dict[str, str] | None = None,
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
            # Some historical engines spell a size level the references do not
            # accept (gcc has no -Oz); translate it per reference compiler while
            # lccc keeps the requested spelling.
            ref_opt = (ref_level_map or {}).get(opt, opt)
            # Baseline: build+run with EVERY reference compiler; all must
            # agree on (exit status, stdout, stderr) before LCCC is judged.
            ref_runs: dict[str, tuple[int, str, str]] = {}
            ref_baseline: tuple[int, str, str] | None = None
            for ref_cc in ref_compilers:
                ref_bin = tmp / f"ref_{ref_cc.replace('/', '_')}_{opt}"
                ref_build_cmd = [ref_cc, ref_opt, *extra_cflags, *unit.extra_flags,
                                 *source_names, "-o", str(ref_bin), "-lm"]
                rc, out, err = run_command(ref_build_cmd, timeout=compile_timeout, cwd=tmp,
                                           env_extra=env_extra)
                if rc != 0:
                    return DiffResult(test_id, "SKIP", f"reference {ref_cc} build failed: {err.strip()}",
                                      opt_level=opt)
                ref_run_cmd = ([runner] if runner else []) + [str(ref_bin)]
                ref_rc, ref_out, ref_err = run_command(ref_run_cmd, timeout=run_timeout,
                                                       env_extra=env_extra)
                if ref_rc == 124:
                    return DiffResult(test_id, "SKIP", f"reference {ref_cc} timed out", opt_level=opt)
                if ref_rc != 0:
                    if ref_rc < 0:
                        # Generator-bug tripwire: the reference died by signal.
                        # If lccc dies by the SAME signal, the generated program
                        # itself crashes everywhere - a generator bug that must
                        # be reported (GEN-BUG, a failure), never a silent SKIP.
                        sig_bin = tmp / f"lccc_sigprobe_{opt}"
                        sig_build_cmd = [lccc_path, opt, *extra_cflags, *unit.extra_flags,
                                         *source_names, "-o", str(sig_bin), "-lm"]
                        b_rc, _, _ = run_command(sig_build_cmd, timeout=compile_timeout, cwd=tmp,
                                                 env_extra=env_extra)
                        if b_rc == 0:
                            sig_run_cmd = ([runner] if runner else []) + [str(sig_bin)]
                            l_rc, _, _ = run_command(sig_run_cmd, timeout=run_timeout,
                                                     env_extra=env_extra)
                            if l_rc == ref_rc:
                                repro_file = repro_dir / f"genbug_{test_id}_{opt}.c"
                                repro_dir.mkdir(parents=True, exist_ok=True)
                                repro_file.write_text(unit.files[primary])
                                return DiffResult(
                                    test_id,
                                    "GEN-BUG",
                                    f"reference {ref_cc} and lccc both died by "
                                    f"{_signal_name(ref_rc)}: the generated case itself "
                                    f"crashes (generator bug)",
                                    str(repro_file),
                                    opt_level=opt,
                                )
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
            rc, out, err = run_command(lccc_build_cmd, timeout=compile_timeout, cwd=tmp,
                                       env_extra=env_extra)
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
            lccc_rc, lccc_out, lccc_err = run_command(lccc_run_cmd, timeout=run_timeout,
                                                      env_extra=env_extra)

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

def _stress_config_combos(config_env: list[str]) -> list[tuple[str, dict[str, str]]]:
    """Expand repeatable --config-env KEY=VALUE declarations into config combos.

    Every distinct KEY contributes one axis: unset (the default), or set to
    any of the values it was declared with.  The result is the cartesian
    product over those axes - including the empty combination (labelled
    "default") - so two single-valued keys reproduce the slot-stress 4-way
    layout matrix: default | K1 | K2 | K1+K2.  Each combo is applied as its
    own environment for BOTH compilers (reference and lccc), at compile
    time and at run time.
    """
    if not config_env:
        return [("default", {})]
    axes: dict[str, list[str]] = {}
    for kv in config_env:
        k, sep, v = kv.partition("=")
        if not sep or not k:
            sys.exit(f"error: invalid --config-env {kv!r}; expected KEY=VALUE")
        if v not in axes.setdefault(k, []):
            axes[k].append(v)
    combos: list[tuple[str, dict[str, str]]] = []
    keys = list(axes)
    for choice in itertools.product(*([None, *axes[k]] for k in keys)):
        env = {k: v for k, v in zip(keys, choice) if v is not None}
        label = "+".join(f"{k}={v}" for k, v in env.items()) if env else "default"
        combos.append((label, env))
    return combos


def run_stress_suite(
    lccc_path: str,
    ref_compilers: list[str],
    opt_levels: list[str],
    runner: str | None,
    repro_dir: Path,
    compile_timeout: float,
    run_timeout: float,
    seed: int = 0,
    count: int = 8,
    config_env: list[str] | None = None,
) -> list[DiffResult]:
    """Runs repository generator scripts to produce specialized stress test cases.

    Two script families are driven:

    * generator scripts that emit a random C program on stdout
      (``gen_fp_stress.py SEED``, ``gen_gep_chain_stress.py SEED``,
      ``gen_slot_stress.py SEED``) - each generator is driven for ``count``
      seeds (``seed`` .. ``seed+count-1``) so a run sweeps the generators'
      shape space instead of probing one fixed seed, and every generated
      program is differentially evaluated against every reference compiler
      at every optimization level;
    * self-contained differential testers that own their own comparison
      (``unroll_stress.py --lccc ...``) - invoked directly, their exit code
      becomes the result.

    When ``config_env`` declares configuration axes (repeatable
    KEY=VALUE), every generated case is additionally evaluated under each
    cartesian-product combination (see ``_stress_config_combos``), both
    compilers compiled and run under that environment.
    """
    results: list[DiffResult] = []
    combos = _stress_config_combos(config_env or [])
    if len(combos) > 1:
        print(f" stress config matrix ({len(combos)} combos): "
              + ", ".join(label for label, _ in combos))

    def evaluate_stress(test_id: str, unit: CompileUnit,
                        env_extra: dict[str, str]) -> DiffResult:
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
            env_extra=env_extra,
        )

    # (name, script) for stdout generators; each takes the case seed as argv[1].
    generators = [
        ("gen_fp_stress", REPO / "scripts" / "gen_fp_stress.py"),
        ("gen_gep_stress", REPO / "scripts" / "gen_gep_chain_stress.py"),
        ("gen_slot_stress", REPO / "scripts" / "gen_slot_stress.py"),
    ]
    for name, script_path in generators:
        if not script_path.exists():
            continue
        for i in range(count):
            case_seed = seed + i
            test_id = f"{name}_seed{case_seed}"
            with tempfile.TemporaryDirectory(prefix=f"stress_{name}_{case_seed}_") as tmp:
                gen_cmd = [sys.executable, str(script_path), str(case_seed)]
                rc, code, err = run_command(gen_cmd, cwd=Path(tmp), timeout=compile_timeout)
                if rc != 0 or not code.strip():
                    # Some scripts output files rather than stdout
                    generated_files = list(Path(tmp).glob("*.c"))
                    code = generated_files[0].read_text() if generated_files else ""
                if not code.strip():
                    results.append(DiffResult(test_id, "SKIP", f"generator failed: {err}"))
                    continue

            unit = CompileUnit(files={"test.c": code})
            for label, combo_env in combos:
                combo_id = test_id if len(combos) == 1 else f"{test_id}[{label}]"
                results.append(evaluate_stress(combo_id, unit, combo_env))

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
# Legacy tests/fuzz engines (single entry point for every differential engine)
# ─────────────────────────────────────────────────────────────────────────────

def _import_fuzz_module(name: str):
    """Lazily import a generator module from tests/fuzz.

    Import happens only when the engine is actually selected, so
    ``--check-engines`` and the unrelated engines stay dependency-free.
    """
    if str(FUZZ_DIR) not in sys.path:
        sys.path.insert(0, str(FUZZ_DIR))
    try:
        return importlib.import_module(name)
    except ImportError as exc:
        sys.exit(f"error: legacy fuzz engine module {name!r} not importable "
                 f"from {FUZZ_DIR}: {exc}")


# Historical compile flags of tests/fuzz/differential_fuzz.py and
# tests/fuzz/phi_cfg_fuzz.py (x86-64 hosted programs, warning-free compare).
_DIFF_TU_FLAGS = ["-std=gnu11", "-w", "-march=raptorlake", "-mtune=raptorlake",
                  "-fomit-frame-pointer"]
# GCC accepts no -Oz; the historical testers map the lccc -Oz level to -Os on
# the reference side (differential_fuzz.py / phi_cfg_fuzz.py `glevel`).
_OZ_REF_ALIAS = {"Oz": "Os"}


def _legacy_generator_pool(engine: str, module_name: str, attr: str,
                            extra_flags: list[str]):
    """Engine runner built from a tests/fuzz generator function.

    The generator (``module.attr``) takes a case seed and returns C source
    text; each case becomes a CompileUnit evaluated through the standard
    differential oracle (every reference at every level, GEN-BUG tripwire,
    failure repro retention).
    """

    def runner(count: int, seed: int, workers: int, evaluate) -> list[DiffResult]:
        module = _import_fuzz_module(module_name)
        generate_case = getattr(module, attr)

        def generate(test_id: str) -> CompileUnit:
            case_seed = int(test_id.rsplit("_", 1)[1])
            return CompileUnit(files={"test.c": generate_case(case_seed)},
                               extra_flags=list(extra_flags))

        return _generate_and_evaluate(engine, count, seed, workers, evaluate, generate)

    return runner


run_differential_pool = _legacy_generator_pool(
    "differential", "differential_fuzz", "generate", _DIFF_TU_FLAGS)
run_phi_cfg_pool = _legacy_generator_pool(
    "phi_cfg", "phi_cfg_fuzz", "gen", _DIFF_TU_FLAGS)


def run_intcmp_pool(count: int, seed: int, workers: int, evaluate) -> list[DiffResult]:
    """tests/fuzz/fuzz_intcmp_thread.py generator as an engine.

    The original drives ONE rng sequentially across programs, so the sources
    are pre-generated on the main thread (worker threads never touch shared
    rng state) and then evaluated through the standard differential oracle.
    """
    module = _import_fuzz_module("fuzz_intcmp_thread")
    rng = random.Random(seed)
    units = [CompileUnit(files={"test.c": module.gen_program(rng, i)},
                         extra_flags=["-w"])
             for i in range(count)]
    return _evaluate_prebuilt("intcmp_thread", units, workers, evaluate)


def _evaluate_prebuilt(engine: str, units: list[CompileUnit], workers: int,
                       evaluate) -> list[DiffResult]:
    """Evaluate pre-generated units in the worker pool (progress dots)."""
    results: list[DiffResult] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, workers)) as pool:
        futures = [pool.submit(evaluate, f"{engine}_{i}", unit)
                   for i, unit in enumerate(units)]
        done_count = 0
        for f in concurrent.futures.as_completed(futures):
            res = f.result()
            results.append(res)
            done_count += 1
            print("." if res.status == "PASS" else "X", end="", flush=True)
            if done_count % 50 == 0 or done_count == len(futures):
                print(f" [{done_count}/{len(futures)}]")
    return results


# Forwarding engines: the standalone testers own their pipeline (nostdlib
# int80 oracles, custom asm drivers, cross compilation + qemu) and cannot be
# expressed as generator+flags configs; this harness launches them and adopts
# their exit code as the verdict - the same absorption pattern the
# stress_suite uses for scripts/unroll_stress.py.  Spec per engine: backing
# script, whether the CLI takes a --seeds LO:HI span, and the compile-only
# fallback config (generator module + attribute + extra -m32 compile flags)
# used on hosts where the ELF32/int80 execution oracle is unavailable.
_STANDALONE_FUZZ_ENGINES: dict[str, dict[str, Any]] = {
    "m32": {"script": "m32_differential_fuzz.py", "seeds": True,
            "gen": ("m32_differential_fuzz", "gen_c", [])},
    "regparm": {"script": "regparm_differential.py", "seeds": True,
                "gen": ("m32_differential_fuzz", "gen_c", ["-mregparm=3"])},
    "slot_rmw": {"script": "slot_rmw_differential.py", "seeds": True,
                 "gen": ("slot_rmw_differential", "gen_probe", ["-fno-pic"])},
    "alias_m32": {"script": "alias_fuzz_m32.py", "seeds": True,
                  "gen": ("alias_fuzz_m32", "gen", ["-mno-sse", "-mno-mmx"])},
    "alu_torture": {"script": "alu_torture_m32.py", "seeds": False,
                    "gen": ("alu_torture_m32", "PROBE", ["-mno-sse", "-mno-mmx"])},
    "aarch64": {"script": "aarch64_fuzz.py", "seeds": True, "gen": None},
}


# Trivial i386 _start that exits 0 through int $0x80: proves the host gcc
# can BUILD ELF32 (-m32 -nostdlib sidesteps the multilib CRT) and that the
# binaries can actually run their int80 syscall oracle.  Sandboxed kernels
# often link+exec ELF32 fine but BLOCK the i386 syscall gateway (SIGSYS);
# there the standalone testers would compare two identical deaths and pass
# vacuously, so this harness refuses to forward and falls back to
# compile-only validation instead.
_M32_EXIT_PROBE = "\n".join([
    ".globl _start",
    "_start:",
    "    movl $1, %eax",
    "    xorl %ebx, %ebx",
    "    int $0x80",
])
_m32_host_cache: dict[str, tuple[bool, bool]] = {}


def _m32_host_capability(gcc: str) -> tuple[bool, bool]:
    """Probe the host's ELF32 pipeline: (can_build_m32, int80_oracle_works)."""
    if gcc in _m32_host_cache:
        return _m32_host_cache[gcc]
    can_build = False
    oracle = False
    with tempfile.TemporaryDirectory(prefix="fuzz_diff_m32probe_") as td:
        asm = Path(td) / "probe.s"
        binary = Path(td) / "probe"
        asm.write_text(_M32_EXIT_PROBE + "\n")
        rc, _, _ = run_command([gcc, "-m32", "-nostdlib", "-no-pie",
                                str(asm), "-o", str(binary)], timeout=60)
        if rc == 0:
            can_build = True
            rc, _, _ = run_command([str(binary)], timeout=10)
            oracle = rc == 0
    _m32_host_cache[gcc] = (can_build, oracle)
    return can_build, oracle


# Historical m32 compile flag base shared by the standalone testers (their
# per-engine extras are declared in _STANDALONE_FUZZ_ENGINES[...]["gen"]).
_M32_COMPILE_BASE = ["-m32", "-fno-PIE", "-fomit-frame-pointer", "-w"]


def _m32_compile_only(engine: str, lccc_path: str, ref_cc: str,
                      opt_levels: list[str], seed: int, count: int,
                      repro_dir: Path) -> list[DiffResult]:
    """Compile-side fallback for m32 engines on int80-blocked hosts.

    Every generated case must still compile under BOTH lccc -m32 and the
    reference -m32 at every level (the historical flag matrix, including the
    per-engine extras like -mregparm=3 or -fno-pic).  lccc compile failures
    are LCCC_CRASH with the source retained; reference failures SKIP.  A
    pass here proves the compiler side only - it is explicitly NOT a codegen
    oracle, which is why the result message says compile-only.
    """
    module_name, gen_attr, extra_flags = _STANDALONE_FUZZ_ENGINES[engine]["gen"]
    module = _import_fuzz_module(module_name)
    generator = getattr(module, gen_attr)
    fixed_probe = not callable(generator)  # alu_torture ships one static PROBE
    n_cases = 1 if fixed_probe else max(1, count)
    results: list[DiffResult] = []
    for i in range(n_cases):
        case_seed = seed + i
        source = generator if fixed_probe else generator(case_seed)
        with tempfile.TemporaryDirectory(prefix=f"m32co_{engine}_{case_seed}_") as td:
            src = Path(td) / "case.c"
            src.write_text(source)
            for opt in opt_levels:
                test_id = (f"{engine}_seed{case_seed}" if len(opt_levels) == 1 else
                           f"{engine}_seed{case_seed}[{opt.lstrip('-')}]")
                flags = [*_M32_COMPILE_BASE, *extra_flags, opt, "-c"]
                rc, _, err = run_command(
                    [ref_cc, *flags, str(src), "-o", str(Path(td) / "ref.o")],
                    timeout=60, cwd=Path(td))
                if rc != 0:
                    results.append(DiffResult(
                        test_id, "SKIP",
                        f"reference {ref_cc} -m32 compile failed: {err.strip()[:200]}",
                        opt_level=opt))
                    continue
                rc, _, err = run_command(
                    [lccc_path, *flags, str(src), "-o", str(Path(td) / "lccc.o")],
                    timeout=60, cwd=Path(td))
                if rc != 0:
                    repro_file = repro_dir / f"crash_{engine}_{case_seed}_{opt.lstrip('-')}.c"
                    repro_dir.mkdir(parents=True, exist_ok=True)
                    repro_file.write_text(source)
                    results.append(DiffResult(
                        test_id, "LCCC_CRASH",
                        f"lccc -m32 compile error (compile-only mode): {err.strip()[:200]}",
                        str(repro_file), opt_level=opt))
                else:
                    results.append(DiffResult(
                        test_id, "PASS",
                        "compile-only (int80 execution oracle unavailable on this host)"))
    if not results:
        sys.exit(f"error: compile-only fallback produced no cases for {engine}")
    return results


def run_standalone_engine(engine: str, lccc_path: str, ref_compilers: list[str],
                           opt_levels: list[str], seed: int, count: int,
                           repro_dir: Path | None = None) -> list[DiffResult]:
    """Forward to a tests/fuzz standalone differential tester.

    The standalone script keeps its own comparison pipeline and exit code
    semantics (0 = all agree); flag translation: --lccc -> --ccc, first
    reference -> --gcc, --seed/--count -> --seeds LO:HI, --opts -> --levels.
    The case directory is retained only when the engine fails, so failing
    sources stay inspectable; successful runs are cleaned up.

    Host gates: aarch64 needs a cross toolchain (SKIP otherwise); the m32
    engines need a working ELF32 build+int80 oracle - when the oracle is
    blocked (common in containers) they fall back to compile-only
    validation instead of reporting vacuous passes.
    """
    spec = _STANDALONE_FUZZ_ENGINES[engine]
    script = FUZZ_DIR / spec["script"]
    takes_seeds = spec["seeds"]
    if not script.is_file():
        sys.exit(f"error: legacy fuzz engine script not found: {script}")
    levels = [o.lstrip("-") for o in opt_levels]
    if not takes_seeds and count != 1:
        print(f"note: --engine {engine} runs one fixed probe per level; "
              f"--count has no effect")

    if engine == "aarch64":
        cross_gcc = shutil.which("aarch64-linux-gnu-gcc")
        qemu = shutil.which("qemu-aarch64")
        missing = [name for name, path in (("aarch64-linux-gnu-gcc", cross_gcc),
                                           ("qemu-aarch64", qemu)) if not path]
        if missing:
            return [DiffResult(
                engine, "SKIP",
                "cross toolchain unavailable on this host: " + ", ".join(missing)
                + "; run on a cross-equipped host or invoke the script directly")]
        ref_cc = cross_gcc
    else:
        ref_cc = ref_compilers[0]
        can_build, oracle = _m32_host_capability(ref_cc)
        if not can_build:
            return [DiffResult(
                engine, "SKIP",
                f"host gcc cannot build ELF32 ({ref_cc} -m32 multilib missing); "
                f"the m32 engines need a 32-bit-capable host")]
        if not oracle:
            print(f"note: {ref_cc} builds ELF32 but the int $0x80 oracle dies on "
                  f"this host (sandboxed syscall policy?); falling back to "
                  f"compile-only validation for --engine {engine}", file=sys.stderr)
            return _m32_compile_only(engine, lccc_path, ref_cc, opt_levels,
                                     seed, count, repro_dir or DEFAULT_REPRO_DIR)

    out_dir = Path(tempfile.mkdtemp(prefix=f"fuzz_diff_{engine}_"))
    cmd = [sys.executable, str(script), "--ccc", lccc_path, "--gcc", ref_cc,
           "--levels", ",".join(levels), "--out", str(out_dir)]
    if takes_seeds:
        cmd += ["--seeds", f"{seed}:{seed + max(count, 1)}"]
    cases = max(1, count) * len(levels) if takes_seeds else len(levels)
    timeout = max(120.0, 12.0 * cases)
    span = f"{seed}:{seed + max(count, 1)}" if takes_seeds else "n/a"
    print(f" forwarding to {script.name}: seeds={span}"
          f" levels={','.join(levels)} (~{cases} case-arms, {timeout:.0f}s budget)")
    rc, out, err = run_command(cmd, timeout=timeout)
    tail = ((err or out) or "").strip()[-400:]
    if rc == 0:
        shutil.rmtree(out_dir, ignore_errors=True)
        return [DiffResult(engine, "PASS",
                           f"standalone engine passed ({cases} case-arms)")]
    if rc == 124:
        shutil.rmtree(out_dir, ignore_errors=True)
        return [DiffResult(engine, "SKIP", "standalone engine timed out")]
    return [DiffResult(
        engine, "MISCOMPILE",
        f"standalone engine failed (rc={rc}): {tail}\n"
        f"  case directory retained: {out_dir}")]


# Per-engine historical defaults preserved from the standalone CLIs: the
# seed-span length (count), the initial seed, the levels, and the reference
# oracle (the standalone testers were built against gcc alone).
_LEGACY_ENGINE_DEFAULTS: dict[str, dict[str, Any]] = {
    "differential": {"count": 100, "seed": 0, "opts": "-O0,-O3,-Os", "refs": "gcc"},
    "phi_cfg": {"count": 200, "seed": 0, "opts": "-O0,-O3,-Os", "refs": "gcc"},
    "intcmp_thread": {"count": 150, "seed": 20260829, "opts": "-O0,-O1,-O2", "refs": "gcc"},
    "m32": {"count": 100, "seed": 0, "opts": "-O0,-O2,-Os", "refs": "gcc"},
    "regparm": {"count": 150, "seed": 0, "opts": "-O0,-O2,-Os", "refs": "gcc"},
    "slot_rmw": {"count": 200, "seed": 0, "opts": "-O0,-O2,-Os", "refs": "gcc"},
    "alias_m32": {"count": 64, "seed": 0, "opts": "-O2,-Os", "refs": "gcc"},
    "alu_torture": {"count": 1, "seed": 0, "opts": "-O2,-Os", "refs": "gcc"},
    "aarch64": {"count": 40, "seed": 0, "opts": "-O2,-Os", "refs": "gcc"},
}


# Single source of truth for --check-engines: every advertised engine maps to
# its real runner (the standalone forwarders share one driver bound per name).
_ENGINE_RUNNERS = {
    "synthetic": run_synthetic_pool,
    "csmith": run_csmith_pool,
    "yarpgen": run_yarpgen_pool,
    "stress_suite": run_stress_suite,
    "differential": run_differential_pool,
    "phi_cfg": run_phi_cfg_pool,
    "intcmp_thread": run_intcmp_pool,
    "m32": functools.partial(run_standalone_engine, "m32"),
    "regparm": functools.partial(run_standalone_engine, "regparm"),
    "slot_rmw": functools.partial(run_standalone_engine, "slot_rmw"),
    "alias_m32": functools.partial(run_standalone_engine, "alias_m32"),
    "alu_torture": functools.partial(run_standalone_engine, "alu_torture"),
    "aarch64": functools.partial(run_standalone_engine, "aarch64"),
}

# Every legacy engine's backing file in tests/fuzz (verified by
# --check-engines so a moved corpus fails loudly instead of vanishing).
_LEGACY_ENGINE_SCRIPTS = {
    "differential": "differential_fuzz.py",
    "phi_cfg": "phi_cfg_fuzz.py",
    "intcmp_thread": "fuzz_intcmp_thread.py",
    **{engine: spec["script"] for engine, spec in _STANDALONE_FUZZ_ENGINES.items()},
}


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

IMPLEMENTED_ENGINES = tuple(_ENGINE_RUNNERS)


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
        default=None,
        help="Number of test iterations to generate and evaluate "
             "(default: 50 for generator engines, 8 for stress_suite; the "
             "tests/fuzz engines default to their historical seed spans, "
             "e.g. 200 for phi_cfg, 150 for intcmp_thread; 0 = infinite, "
             "only for synthetic/csmith/yarpgen)",
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
        default=None,
        help="Path to LCCC compiler binary (default: target/fastbuild/lccc; "
             "env LCCC; lccc-arm for --engine aarch64)",
    )
    ap.add_argument(
        "--refs",
        default=None,
        help="Comma-separated reference compilers (default: gcc,clang; the "
             "tests/fuzz engines default to their historical gcc oracle)",
    )
    ap.add_argument(
        "--opts",
        default=None,
        help="Comma-separated optimization levels to verify (default: "
             "-O0,-O1,-O2,-O3; the tests/fuzz engines default to their "
             "historical levels, e.g. -O0,-O3,-Os for differential)",
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
        default=None,
        help="Initial random seed (default: current time; the tests/fuzz "
             "engines default to their historical deterministic seeds, e.g. "
             "20260829 for intcmp_thread)",
    )
    ap.add_argument(
        "--config-env",
        dest="config_env",
        action="append",
        default=[],
        metavar="KEY=VALUE",
        help="stress_suite only, repeatable: declare a configuration axis. "
             "Every generated stress case is additionally evaluated under each "
             "cartesian-product combination of the declared axes (the empty "
             "combination is the default environment), with BOTH the reference "
             "and lccc compiling and running under it. Two axes "
             "K1=V --config-env K2=V yield: default, K1, K2, K1+K2 "
             "(the slot-stress 4-way layout matrix)",
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
    missing entry fails loudly.  The tests/fuzz engines additionally require
    their backing script under tests/fuzz/, so a moved or renamed corpus
    fails here instead of silently zero-testing.
    """
    dispatch = _ENGINE_RUNNERS
    missing = [e for e in IMPLEMENTED_ENGINES if e not in dispatch]
    if missing:
        print(f"error: engines advertised but not implemented: {', '.join(missing)}")
        return 1
    missing_scripts = [e for e, s in _LEGACY_ENGINE_SCRIPTS.items()
                       if not (FUZZ_DIR / s).is_file()]
    if missing_scripts:
        print("error: tests/fuzz engine scripts missing: "
              + ", ".join(f"{e} -> {FUZZ_DIR / _LEGACY_ENGINE_SCRIPTS[e]}"
                           for e in missing_scripts))
        return 1
    print(f"all {len(IMPLEMENTED_ENGINES)} engines implemented: "
          f"{', '.join(IMPLEMENTED_ENGINES)}")
    return 0


def main() -> int:
    args = parse_args()

    if args.check_engines:
        return check_engines()

    legacy = args.engine in _LEGACY_ENGINE_DEFAULTS

    # --count keeps its historical default of 50 for the generator engines;
    # the stress-suite sweep defaults to 8 seeds per generator; each
    # tests/fuzz engine keeps the seed span of its standalone CLI.
    count = args.count
    if count is None:
        count = _LEGACY_ENGINE_DEFAULTS.get(args.engine, {}).get(
            "count", 8 if args.engine == "stress_suite" else 50)
    if args.engine == "stress_suite" and count < 1:
        sys.exit("error: --engine stress_suite requires a bounded --count of at least 1 "
                 "(0/negative is only meaningful for the infinite generator engines)")
    if legacy and count < 1:
        sys.exit(f"error: --engine {args.engine} requires a bounded --count of at least 1 "
                 "(the historical testers sweep an explicit seed span)")
    if args.config_env and args.engine != "stress_suite":
        sys.exit("error: --config-env only applies to --engine stress_suite")
    if args.runner and args.engine in _STANDALONE_FUZZ_ENGINES:
        print(f"note: --runner is unused by --engine {args.engine} "
              f"(the standalone tester manages its own execution model)", file=sys.stderr)

    # Historical deterministic seeds for the tests/fuzz engines (e.g.
    # intcmp_thread's 20260829); everything else stays time-based.
    seed = args.seed
    if seed is None:
        seed = _LEGACY_ENGINE_DEFAULTS.get(args.engine, {}).get("seed", 0 if legacy else int(time.time()))

    # Resolve compiler paths ONCE, before any evaluation.  Test cases are
    # compiled from per-case temporary directories, so a relative --lccc or
    # --refs path that validated fine against the launcher's CWD would be
    # unresolvable from the tempdir (this is exactly how the CI smoke first
    # failed: `--lccc target/fastbuild/lccc` + cwd=tmpdir => ENOENT reported
    # as an LCCC_CRASH).  Everything downstream gets absolute paths.
    lccc_arg = args.lccc or os.environ.get("LCCC") or ""
    if not lccc_arg:
        # The aarch64 engine targets the arm driver; everything else drives
        # the x86-64 lccc (use the arm driver when it is built).
        if args.engine == "aarch64" and DEFAULT_LCCC_ARM.is_file():
            print(f"note: --engine aarch64 defaults to the arm driver: {DEFAULT_LCCC_ARM}")
            lccc_arg = str(DEFAULT_LCCC_ARM)
        else:
            lccc_arg = str(DEFAULT_LCCC)
    if not Path(lccc_arg).is_file():
        sys.exit(f"error: LCCC binary '{lccc_arg}' not found. Build first!")
    lccc_path = str(Path(lccc_arg).resolve())

    refs_raw = args.refs or _LEGACY_ENGINE_DEFAULTS.get(args.engine, {}).get("refs", "gcc,clang")
    ref_compilers_raw = [c.strip() for c in refs_raw.split(",") if c.strip()]
    available_refs: list[str] = []
    missing_refs: list[str] = []
    for ref in ref_compilers_raw:
        resolved = shutil.which(ref)
        if resolved:
            # Bare name (or PATH-resolvable path): which() returns the
            # absolute spelling, which is CWD-independent.
            available_refs.append(resolved)
        elif Path(ref).is_file():
            available_refs.append(str(Path(ref).resolve()))
        else:
            missing_refs.append(ref)
    if missing_refs:
        print(f"note: reference compilers not found (skipped): "
              f"{', '.join(missing_refs)}", file=sys.stderr)
    if not available_refs:
        sys.exit(f"error: none of the reference compilers ({refs_raw}) found in PATH")

    opts_raw = args.opts or _LEGACY_ENGINE_DEFAULTS.get(args.engine, {}).get("opts", "-O0,-O1,-O2,-O3")
    opt_levels = [o.strip() for o in opts_raw.split(",") if o.strip()]

    # i686 targets need -m32 on the host references for a like-for-like oracle.
    arch_flags: list[str] = []
    if args.arch == "i686":
        arch_flags = ["-m32"]

    print("=" * 70)
    print(" LCCC Differential Fuzzing & Testing Harness")
    print(f" Engine   : {args.engine}")
    print(f" Target   : {args.arch} | Runner: {args.runner or 'native'}")
    print(f" LCCC     : {lccc_path}")
    print(f" Reference: {', '.join(available_refs)}")
    print(f" Opt levels: {', '.join(opt_levels)}")
    print(f" Workers  : {args.workers} | Count: {count or 'infinite'} | Seed: {seed}")
    print("=" * 70)

    repro_dir = Path(args.repro_dir)

    # The differential/phi_cfg engines keep the historical Oz spelling for
    # lccc while the references get their closest level (gcc: -Os).
    ref_level_map = _OZ_REF_ALIAS if args.engine in ("differential", "phi_cfg") else None

    def evaluate(test_id: str, unit: CompileUnit) -> DiffResult:
        return evaluate_single_test(
            test_id,
            unit,
            lccc_path,
            available_refs,
            opt_levels,
            args.runner,
            arch_flags,
            repro_dir,
            args.compile_timeout,
            args.run_timeout,
            ref_level_map=ref_level_map,
        )

    t0 = time.time()
    results: list[DiffResult]

    if args.engine == "stress_suite":
        results = run_stress_suite(
            lccc_path, available_refs, opt_levels, args.runner, repro_dir,
            args.compile_timeout, args.run_timeout, seed=seed,
            count=count, config_env=args.config_env,
        )
    elif args.engine == "synthetic":
        results = run_synthetic_pool(count, seed, args.workers, evaluate)
    elif args.engine == "csmith":
        csmith_bin = args.csmith or os.environ.get("CSMITH", "csmith")
        results = run_csmith_pool(
            count, seed, args.workers, evaluate,
            csmith_bin, args.include_dirs, args.compile_timeout,
        )
    elif args.engine == "yarpgen":
        yarpgen_bin = args.yarpgen or os.environ.get("YARPGEN", "yarpgen")
        results = run_yarpgen_pool(
            count, seed, args.workers, evaluate,
            yarpgen_bin, args.compile_timeout,
        )
    elif args.engine == "differential":
        results = run_differential_pool(count, seed, args.workers, evaluate)
    elif args.engine == "phi_cfg":
        results = run_phi_cfg_pool(count, seed, args.workers, evaluate)
    elif args.engine == "intcmp_thread":
        results = run_intcmp_pool(count, seed, args.workers, evaluate)
    elif args.engine in _STANDALONE_FUZZ_ENGINES:
        results = run_standalone_engine(args.engine, lccc_path, available_refs,
                                        opt_levels, seed, count, repro_dir)
    else:  # pragma: no cover - argparse restricts choices
        sys.exit(f"error: engine '{args.engine}' is not implemented; "
                 f"implemented engines: {', '.join(IMPLEMENTED_ENGINES)}")

    elapsed = time.time() - t0
    # FAIL_STATUSES: any of these makes the harness exit 1.  GEN-BUG is a
    # failure (a generated case that dies by the same signal under the
    # reference and lccc is an invalid oracle, never a silent SKIP).
    fail_statuses = ("MISCOMPILE", "LCCC_CRASH", "GEN-BUG")
    pass_cnt = sum(1 for r in results if r.status == "PASS")
    fail_cnt = sum(1 for r in results if r.status in fail_statuses)
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
            if r.status in fail_statuses:
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
