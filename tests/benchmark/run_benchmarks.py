#!/usr/bin/env python3
"""LCCC generated-code benchmark laboratory.

This runner deliberately treats a virtual machine as a *screening* environment:
it performs paired, randomized, CPU-pinned wall-clock experiments and records
all raw samples plus reproducibility metadata, but it never labels them as
hardware-counter evidence or bare-metal performance claims.

Examples:
  # Default: LCCC, GCC, plus Clang/ICX when installed; 15 paired rounds.
  python3 tests/benchmark/run_benchmarks.py

  # Narrow reproducible experiment with assembly artifacts and JSON evidence.
  python3 tests/benchmark/run_benchmarks.py \
      --only gzip_crc32,sqlite_varint --compilers lccc,gcc --reps 21 \
      --artifact-dir results/crc-varint --json results/crc-varint/results.json \
      --markdown results/crc-varint/report.md

  # Correctness/compilation screening only.
  python3 tests/benchmark/run_benchmarks.py --only linux_find_bit --skip-perf

No third-party Python packages are required.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import platform
import random
import re
import resource
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


HERE = Path(__file__).resolve().parent
PROGRAMS = HERE / "programs"
REPO_ROOT = HERE.parents[1]
DEFAULT_LCCC = REPO_ROOT / "target" / "fastbuild" / "lccc"
sys.path.insert(0, str(REPO_ROOT))
from tools.corpus import process


@dataclass(frozen=True)
class Benchmark:
    """A deterministic executable benchmark and its compilation requirements."""

    name: str
    description: str
    sources: tuple[str, ...]
    tags: tuple[str, ...]
    timeout_s: int = 90
    extra_flags: tuple[str, ...] = ()
    # CI's sharded workflow deliberately pins a bounded canonical subset;
    # exploratory workload-derived cases remain in the full/local registry.
    ci_sharded: bool = True


# Keep historical synthetic tests, then add workload-derived kernels.  Sources
# may live in the legacy numbered directory until they are migrated.
BENCHMARKS: tuple[Benchmark, ...] = (
    Benchmark("f32_sse1_map", "strict cache-resident F32 maps without SSE2",
              ("f32_sse1_map.c",), ("synthetic", "simd", "fp", "map", "sse1"),
              extra_flags=("-mno-sse2", "-msse", "-ffp-contract=off", "-DSSE1_MAP_REQUIRE_ONLY")),
    Benchmark("fp32_product_map", "strict cache-resident F32 multiply/add maps",
              ("fp_product_map.c",), ("synthetic", "simd", "fp", "map"),
              extra_flags=("-ffp-contract=off",)),
    Benchmark("fp64_product_map", "strict cache-resident F64 multiply/add maps",
              ("fp_product_map.c",), ("synthetic", "simd", "fp", "map"),
              extra_flags=("-DFP_MAP_DOUBLE", "-ffp-contract=off")),
    Benchmark("i32_product_map", "cache-resident pointwise modular I32 products",
              ("i32_product_map.c",), ("synthetic", "simd", "integer", "map")),
    Benchmark("volatile_reduce", "volatile TLS sum and product / scalar ILP",
              ("volatile_reduce.c",), ("integer", "loop", "tls")),
    # Required source semantics, not optional tuning: these signed recurrences
    # intentionally overflow. Keeping the requirement here covers every native
    # and static caller, including correctness-only/default invocations.
    Benchmark("llvm_word_counts", "scalar zero-count unions and constant caps / synthetic call bundle",
              ("llvm_word_counts.c",), ("synthetic", "count", "integer")),
    Benchmark("llvm_scalar_facts", "conditional scalar facts / synthetic call bundle",
              ("llvm_scalar_facts.c",), ("synthetic", "scalar", "bit", "calls")),
    Benchmark("hot_web_loop", "call-spanning hot webs versus cold callee-saved holders",
              ("hot_web_loop.c",), ("synthetic", "regalloc", "calls", "integer")),
    Benchmark("arith_loop", "32-variable arithmetic loop / register pressure",
              ("arith_loop.c", "01_arith_loop.c"), ("synthetic", "regalloc", "integer"),
              extra_flags=("-fwrapv",)),
    Benchmark("fib", "recursive Fibonacci / recurrence recognition",
              ("fib.c", "02_fib.c"), ("synthetic", "calls", "recursion")),
    Benchmark("matmul", "dense matrix multiply / FP and cache",
              ("matmul.c", "03_matmul.c"), ("synthetic", "fp", "loop")),
    Benchmark("qsort", "quicksort via libc / branches",
              ("qsort.c", "04_qsort.c"), ("synthetic", "branch", "library")),
    Benchmark("sieve", "sieve of Eratosthenes / stores",
              ("sieve.c", "05_sieve.c"), ("synthetic", "memory", "loop")),
    Benchmark("tce_sum", "tail-recursive accumulator / TCE",
              ("tce_sum.c", "06_tce_sum.c"), ("synthetic", "calls", "tce")),
    Benchmark("nbody", "N-body simulation / FP structs",
              ("nbody.c",), ("synthetic", "fp", "struct"), 120, ("-lm",)),
    Benchmark("binary_trees", "binary trees / allocation and recursion",
              ("binary_trees.c",), ("synthetic", "allocation", "recursion"), 120),
    Benchmark("spectral_norm", "spectral norm / dense floating point",
              ("spectral_norm.c",), ("synthetic", "fp", "loop"), 120, ("-lm",)),
    # Contraction changes the iteration count: a fused FMA rounds once where
    # the source rounds twice, so an unbounded `-ffp-contract=fast` makes each
    # compiler's own choice of fused forms decide the escape test. Pinning it
    # off keeps every arm's output comparable while the timed code, the
    # arithmetic itself, remains whatever that compiler chooses to emit.
    Benchmark("mandelbrot", "Mandelbrot / FP branch-heavy inner loop",
              ("mandelbrot.c",), ("synthetic", "fp", "branch"), 120, ("-ffp-contract=off",)),
    Benchmark("hash_table", "hash table / pointer chasing",
              ("hash_table.c",), ("synthetic", "memory", "branch")),
    Benchmark("strlen_bench", "string operations / byte loops",
              ("strlen_bench.c",), ("synthetic", "string", "memory")),
    Benchmark("switch_dispatch", "switch lowering / dispatch",
              ("switch_dispatch.c",), ("synthetic", "switch", "branch"),
              extra_flags=("-fwrapv",)),
    Benchmark("struct_copy", "struct copy / ABI and memory",
              ("struct_copy.c",), ("synthetic", "abi", "memory")),
    Benchmark("loop_patterns", "scalar loop transforms",
              ("loop_patterns.c",), ("synthetic", "loop", "integer"),
              extra_flags=("-fwrapv",)),
    Benchmark("fannkuch", "Fannkuch-Redux / permutations",
              ("fannkuch.c",), ("synthetic", "integer", "branch"), 180),
    Benchmark("ackermann", "Ackermann / deep recursion",
              ("ackermann.c",), ("synthetic", "recursion", "calls"), 120),
    Benchmark("constant_recursion", "constant recursive specialization",
              ("constant_recursion.c",), ("synthetic", "recursion", "specialization")),
    Benchmark("bitops", "bit manipulation / integer selection",
              ("bitops.c",), ("synthetic", "bit", "integer")),
    Benchmark("double_reduction", "two independent accumulators per loop (multi-reduction)",
              ("double_reduction.c",), ("synthetic", "vectorize", "integer", "loop")),
    Benchmark("ascii_case_fold", "ASCII parser case-fold byte loop / branch selection",
              ("ascii_case_fold.c",), ("synthetic", "parser", "branch", "memory")),
    Benchmark("binary_search", "sorted-table binary search / branch-heavy lookup",
              ("binary_search.c",), ("synthetic", "search", "branch")),
    Benchmark("ring_fifo", "masked ring FIFO enqueue/dequeue / dependent loads",
              ("ring_fifo.c",), ("synthetic", "queue", "memory", "branch")),
    Benchmark("aarch64_select_patterns", "conditional increment, narrow compare, and select pressure",
              ("aarch64_select_patterns.c",), ("synthetic", "aarch64", "select", "integer")),
    Benchmark("histogram", "256-bin histogram / indexed increment and reduction",
              ("histogram.c",), ("synthetic", "database", "memory", "integer")),
    # Workload-derived corpus.  Licenses and exact extraction records are in
    # WORKLOAD_PROVENANCE.md rather than being hidden in an opaque generator.
    Benchmark("gzip_crc32", "GNU gzip CRC-32 scalar table loop",
              ("gzip_crc32.c",), ("workload", "gzip", "checksum", "memory")),
    Benchmark("libm_round_family", "glibc libm scalar rounding entry points (vroundsd inline)",
              ("libm_round_family.c",), ("workload", "glibc", "fp", "libm")),
    Benchmark("tls_seg_access", "glibc TLS access shapes (THREAD_SELF/SETMEM, %fs segment)",
              ("tls_seg_access.c",), ("workload", "glibc", "tls", "memory")),
    Benchmark("percpu_ptr_walk", "computed GS-base per-CPU pointer walk",
              ("percpu_ptr_walk.c",), ("synthetic", "kernel", "percpu", "memory")),
    Benchmark("byte_load_reassembly", "portable little-endian byte assembly (Linux get_unaligned_le shape)",
              ("byte_load_reassembly.c",), ("synthetic", "endian", "memory", "integer")),
    Benchmark("zlib_ng_adler32", "zlib-ng Adler-32 NMAX accumulator",
              ("zlib_ng_adler32.c",), ("workload", "zlib-ng", "checksum", "loop")),
    Benchmark("zlib_ng_adler32_combine", "zlib-ng checksum composition / modular arithmetic",
              ("zlib_ng_adler32_combine.c",), ("workload", "zlib-ng", "checksum", "integer")),
    Benchmark("zlib_ng_compare256", "zlib-ng 256-byte match extension / XOR and bit scan",
              ("zlib_ng_compare256.c",), ("workload", "zlib-ng", "compression", "memory", "bit")),
    Benchmark("expat_xml_scan", "Expat UTF-8 XML name-token scan",
              ("expat_xml_scan.c",), ("workload", "expat", "parser", "branch")),
    Benchmark("expat_siphash24", "Expat SipHash-2-4 keyed name hashing / short variable messages",
              ("expat_siphash24.c",), ("workload", "expat", "hash", "arx", "integer")),
    Benchmark("sqlite_varint", "SQLite 1–9 byte varint decoder",
              ("sqlite_varint.c",), ("workload", "sqlite", "branch", "integer")),
    Benchmark("linux_find_bit", "Linux sparse find_next_andnot_bit",
              ("linux_find_bit.c",), ("workload", "linux", "bitmap", "bit")),
    # The original ~15 ms runtime is dominated by process-launch/scheduler
    # noise on constrained hosts. Preserve it for continuity, and keep a
    # separately output-checked long-running arm for performance decisions.
    Benchmark("linux_find_bit_scaled", "Linux sparse find_next_andnot_bit / scaled",
              ("linux_find_bit.c",), ("workload", "linux", "bitmap", "bit"),
              120, ("-DPASSES=65536",)),
    Benchmark("glibc_memcmp", "glibc aligned-word memcmp path",
              ("glibc_memcmp.c",), ("workload", "glibc", "memory", "branch")),
    Benchmark("chacha20_block", "ChaCha20 20-round ARX block cipher / register pressure",
              ("chacha20_block.c",), ("workload", "crypto", "arx", "integer")),
    Benchmark("sha256_transform", "SHA-256 64-step block transform / rotate and schedule",
              ("sha256_transform.c",), ("workload", "crypto", "sha256", "integer")),
    Benchmark("linux_rbtree", "Linux kernel intrusive Red-Black tree insertion and search",
              ("linux_rbtree.c",), ("workload", "linux", "rbtree", "tree", "memory")),
    Benchmark("zstd_count", "Zstandard fast unaligned match counting (ZSTD_count / ctz)",
              ("zstd_count.c",), ("workload", "zstd", "compression", "bit")),
    Benchmark("lz4_compress", "LZ4 fast block compression / 4-byte hash matching",
              ("lz4_compress.c",), ("workload", "lz4", "compression", "memory")),
    # The default LZ4 input reaches no four-byte matches: its byte-compare
    # extension loop cannot be evaluated by timing that arm. This separately
    # output-checked input runs long matches for >200 ms on the screening VM.
    Benchmark("lz4_match_extend", "LZ4 repeat-rich input / byte match extension",
              ("lz4_compress.c",), ("workload", "lz4", "compression", "byte-compare"),
              120, ("-DMATCH_RICH=1", "-DPASSES=2048")),
    Benchmark("glibc_strstr", "glibc two-way string search / needle shift table",
              ("glibc_strstr.c",), ("workload", "glibc", "string", "search")),
    # Second extension round. Each of these is a *different* pressure profile
    # from the kernels already present: gzip's match finder is a data-dependent
    # chain walk with a widened unaligned load; list_sort is pointer chasing
    # behind a non-devirtualizable comparison callback; double-fast is two
    # independent hash probes per iteration whose results feed address
    # arithmetic. All three self-check against a naive reference in the same
    # binary, so a faster-but-wrong rewrite fails the output comparison rather
    # than winning the timing column.
    Benchmark("gzip_longest_match", "GNU gzip longest_match / hash-chain match finder",
              ("gzip_longest_match.c",), ("workload", "gzip", "compression", "branch"),
              ci_sharded=False),
    Benchmark("linux_list_sort", "Linux kernel list_sort / pointer-chasing merge sort",
              ("linux_list_sort.c",), ("workload", "linux", "sort", "memory", "calls"),
              ci_sharded=False),
    Benchmark("zstd_double_fast", "Zstandard double-fast match search / dual hash tables",
              ("zstd_double_fast.c",), ("workload", "zstd", "compression", "memory"),
              ci_sharded=False),
    # Third extension round (archpkgbuilds workload kernels: gzip, zlib-ng,
    # expat, sqlite, linux-cachymod-6.18, glibc). Each exercises a distinct
    # codegen regime (8-table slice-by-8 CRC, saturating u16 hash-table slide +
    # rolling insert, UTF-8 byte-class state machine, 12-way switch + big-endian
    # record decoder, 4-lane 64-bit rotate-multiply hash, SWAR word scan +
    # GNU_HASH djb2) and self-checks against an independent reference in-binary.
    Benchmark("gzip_crc32_slice8", "GNU gzip slice-by-8 CRC-32 / 8-way table lookup and endian word xor",
              ("gzip_crc32_slice8.c",), ("workload", "gzip", "checksum", "memory"),
              ci_sharded=False),
    Benchmark("zlib_ng_slide_hash", "zlib-ng slide_hash_c + rolling insert_string / saturating u16 array map",
              ("zlib_ng_slide_hash.c",), ("workload", "zlib-ng", "compression", "vectorize", "loop"),
              ci_sharded=False),
    Benchmark("expat_utf8_content", "Expat UTF-8 XML content scanner / byte-class dispatch and entity loops",
              ("expat_utf8_content.c",), ("workload", "expat", "parser", "switch", "branch"),
              ci_sharded=False),
    Benchmark("sqlite_vdbe_serial", "SQLite VDBE sqlite3VdbeSerialGet / switch jump-table and big-endian decode",
              ("sqlite_vdbe_serial.c",), ("workload", "sqlite", "database", "switch", "endian"),
              ci_sharded=False),
    Benchmark("linux_xxhash64", "Linux kernel xxhash64 / 4-lane 64-bit rotate-multiply streaming hash",
              ("linux_xxhash64.c",), ("workload", "linux", "hash", "integer", "loop"),
              ci_sharded=False),
    Benchmark("glibc_dl_hash", "glibc _dl_new_hash + SWAR strchrnul / dynamic linker symbol table lookup",
              ("glibc_dl_hash.c",), ("workload", "glibc", "string", "hash", "bit"),
              ci_sharded=False),
)


@dataclass(frozen=True)
class CompilerSpec:
    key: str
    label: str
    executable: str
    flags: tuple[str, ...]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def stable_seed(*parts: str) -> int:
    """A process-independent seed (unlike Python's randomized hash())."""
    material = "\x1f".join(parts).encode("utf-8")
    return int.from_bytes(hashlib.sha256(material).digest()[:8], "big")


def balanced_compiler_order(items: list, rng: random.Random, round_number: int, previous: list | None) -> list:
    """Randomize two-arm order in balanced two-round blocks; shuffle larger sets.

    Reversal applies only if the two active compilers are unchanged; after a
    failure changes the set, never reintroduce a failed compiler.
    """
    order = list(items)
    if (len(order) == 2 and round_number % 2 == 1 and previous is not None
            and len(previous) == 2 and all(item in order for item in previous)):
        return list(reversed(previous))
    rng.shuffle(order)
    return order


def first_nonempty_line(text: str) -> str:
    for line in text.splitlines():
        if line.strip():
            return line.strip()
    return ""


def command_version(executable: str) -> str:
    """Best-effort version capture; version failures are evidence too."""
    try:
        result = subprocess.run(
            [executable, "--version"], capture_output=True, text=True,
            timeout=10, check=False,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired) as error:
        return f"unavailable: {error}"
    line = first_nonempty_line(result.stdout) or first_nonempty_line(result.stderr)
    return line or f"exit={result.returncode}"


def child_cpu_seconds() -> float:
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime + usage.ru_stime


def invoke(command: list[str], *, timeout_s: int, cwd: Path | None = None,
           env: dict[str, str] | None = None) -> dict[str, Any]:
    """Bounded byte-preserving execution; wall time and child CPU stay distinct."""
    cpu_before = child_cpu_seconds()
    start_ns = time.perf_counter_ns()
    try:
        result = process.run(command, timeout_s, cwd=cwd, env=env)
    except OSError as exc:
        result = dict(returncode=None, stdout=b'', stderr=b'', failure=str(exc))
    end_ns = time.perf_counter_ns()
    cpu_after = child_cpu_seconds()
    return {
        "ok": result['failure'] is None and result['returncode'] == 0,
        "returncode": result['returncode'],
        "error": result['failure'] or '',
        "wall_s": (end_ns - start_ns) / 1_000_000_000.0,
        "child_cpu_s": max(0.0, cpu_after - cpu_before),
        "stdout": result['stdout'].decode('utf-8', errors='backslashreplace'),
        "stdout_sha256": hashlib.sha256(result['stdout']).hexdigest(),
        "stderr": result['stderr'].decode('utf-8', errors='backslashreplace'),
        "command": command,
    }


def source_for(benchmark: Benchmark) -> Path | None:
    for candidate in benchmark.sources:
        path = PROGRAMS / candidate
        if path.is_file():
            return path
    return None


def parse_selection(raw: list[str] | None) -> set[str]:
    if not raw:
        return set()
    selected: set[str] = set()
    for group in raw:
        selected.update(item.strip() for item in group.split(",") if item.strip())
    return selected


def compiler_path(candidate: str) -> str | None:
    path = Path(candidate)
    if path.is_file() and os.access(path, os.X_OK):
        return str(path.resolve())
    found = shutil.which(candidate)
    return found


def detect_gcc_include(gcc: str | None) -> str | None:
    if not gcc:
        return None
    try:
        result = subprocess.run(
            [gcc, "-print-file-name=include"], capture_output=True, text=True,
            timeout=10, check=False,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    include = result.stdout.strip()
    return include if result.returncode == 0 and Path(include).is_dir() else None


def discover_compilers(args: argparse.Namespace) -> tuple[list[CompilerSpec], dict[str, str]]:
    requested = [item.strip().lower() for item in args.compilers.split(",") if item.strip()]
    aliases = {"icc": "icx"}
    requested = [aliases.get(item, item) for item in requested]
    known = {"lccc", "lev", "ccc", "gcc", "clang", "icx"}
    additional: dict[str, str] = {}
    for item in getattr(args, "candidate", []):
        key, separator, executable = item.partition("=")
        if (not separator or not executable or not re.fullmatch(r"[a-z][a-z0-9_-]*", key)
                or key in known or key in additional):
            raise ValueError("--candidate requires a unique non-reserved label=executable: " + item)
        additional[key] = executable
    requested = list(dict.fromkeys([*requested, *additional]))
    unknown = [item for item in requested if item not in known | additional.keys()]
    if unknown:
        raise ValueError(f"unknown compiler key(s): {', '.join(unknown)}")

    candidates = {
        "lccc": ("LCCC", args.lccc),
        "lev": ("Lev-LCCC", args.lev),
        "ccc": ("CCC", args.ccc),
        "gcc": ("GCC", args.gcc),
        "clang": ("Clang", args.clang),
        "icx": ("ICX", args.icx),
    }
    candidates.update((key, ("LCCC candidate: " + key, executable))
                      for key, executable in additional.items())
    resolved = {key: compiler_path(value) for key, (_, value) in candidates.items()}
    gcc_include = detect_gcc_include(resolved.get("gcc"))
    unavailable: dict[str, str] = {}
    compilers: list[CompilerSpec] = []

    for key in requested:
        label, supplied = candidates[key]
        executable = resolved[key]
        if not executable:
            unavailable[key] = f"not found: {supplied}"
            continue
        if key in ("lccc", "lev", "ccc") or key in additional:
            if not gcc_include:
                unavailable[key] = "GCC builtin include directory unavailable (needed for compiler headers)"
                continue
            flags = (f"-I{gcc_include}", args.opt, *args.cflag)
        else:
            flags = (args.opt, *args.cflag)
        compilers.append(CompilerSpec(key, label, executable, tuple(flags)))

    return compilers, unavailable


def text_size(binary: Path) -> int:
    size_tool = shutil.which("size")
    if not size_tool:
        return 0
    result = invoke([size_tool, str(binary)], timeout_s=10)
    if not result["ok"]:
        return 0
    lines = result["stdout"].splitlines()
    if len(lines) < 2:
        return 0
    fields = lines[1].split()
    try:
        return int(fields[0])
    except (IndexError, ValueError):
        return 0


def write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def save_compile_artifacts(artifact_dir: Path | None, benchmark: Benchmark,
                           compiler: CompilerSpec, source: Path, binary: Path,
                           result: dict[str, Any]) -> None:
    if artifact_dir is None:
        return
    destination = artifact_dir / benchmark.name / compiler.key
    destination.mkdir(parents=True, exist_ok=True)
    write_text(destination / "compile-command.txt", " ".join(result["command"]) + "\n")
    write_text(destination / "compile.stderr.txt", result.get("stderr", "") + "\n")
    if source.is_file():
        shutil.copy2(source, destination / "source.c")
    if not result.get("ok") or not binary.is_file():
        return

    for tool, suffix, command in (
        (shutil.which("objdump"), "assembly.txt", ["-drwC"]),
        (shutil.which("readelf"), "sections.txt", ["-SW"]),
        (shutil.which("size"), "size.txt", []),
    ):
        if not tool:
            continue
        tool_result = invoke([tool, *command, str(binary)], timeout_s=30)
        write_text(destination / suffix, tool_result["stdout"] + "\n" + tool_result["stderr"])


def compile_one(compiler: CompilerSpec, benchmark: Benchmark, source: Path,
                output: Path, artifact_dir: Path | None) -> dict[str, Any]:
    output.parent.mkdir(parents=True, exist_ok=True)
    # Link libraries must follow the translation unit for one-pass Unix linkers
    # (for example libm's sqrt in spectral_norm/nbody).  Keep them separate
    # from compiler flags so this ordering is invariant across compilers.
    command = [compiler.executable, *compiler.flags, "-o", str(output),
               str(source), *benchmark.extra_flags]
    # A zero-exit/no-output compiler must never reuse a previous executable.
    output.unlink(missing_ok=True)
    result = invoke(command, timeout_s=180, cwd=REPO_ROOT)
    if result["ok"] and (not output.is_file() or not os.access(output, os.X_OK)):
        result["ok"] = False
        result["error"] = "compiler reported success without an executable artifact"
    result["binary_sha256"] = sha256_file(output) if result["ok"] else None
    result["binary_bytes"] = output.stat().st_size if result["ok"] and output.is_file() else 0
    result["text_bytes"] = text_size(output) if result["ok"] and output.is_file() else 0
    result["source_sha256"] = sha256_file(source)
    save_compile_artifacts(artifact_dir, benchmark, compiler, source, output, result)
    return result


def bootstrap_median_ci(values: list[float], seed: int, resamples: int = 2000) -> list[float] | None:
    if not values:
        return None
    if len(values) == 1:
        return [values[0], values[0]]
    rng = random.Random(seed)
    count = len(values)
    medians = []
    for _ in range(resamples):
        medians.append(statistics.median(values[rng.randrange(count)] for _ in range(count)))
    medians.sort()
    lower = medians[int(0.025 * (resamples - 1))]
    upper = medians[int(0.975 * (resamples - 1))]
    return [lower, upper]


def outlier_count(values: list[float]) -> int:
    """Count robust MAD outliers without excluding any sample from statistics."""
    if len(values) < 3:
        return 0
    middle = statistics.median(values)
    deviations = [abs(value - middle) for value in values]
    mad = statistics.median(deviations)
    if mad == 0.0:
        return 0
    return sum(1 for value in values if abs(0.67448975 * (value - middle) / mad) > 3.5)


def summarize(values: list[float], seed: int) -> dict[str, Any] | None:
    if not values:
        return None
    average = statistics.fmean(values)
    stdev = statistics.stdev(values) if len(values) > 1 else 0.0
    median = statistics.median(values)
    return {
        "n": len(values),
        "min": min(values),
        "median": median,
        "mean": average,
        "max": max(values),
        "stdev": stdev,
        "cv": (stdev / average) if average else 0.0,
        "mad": statistics.median(abs(value - median) for value in values),
        "outlier_count_mad": outlier_count(values),
        "median_ci95_bootstrap": bootstrap_median_ci(values, seed),
    }


def read_first(path: str) -> str | None:
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def git_metadata() -> dict[str, Any]:
    metadata: dict[str, Any] = {"repo": str(REPO_ROOT)}
    for name, command in (
        ("revision", ["git", "rev-parse", "HEAD"]),
        ("status", ["git", "status", "--short"]),
    ):
        result = invoke(command, timeout_s=10, cwd=REPO_ROOT)
        metadata[name] = result["stdout"] if result["ok"] else f"unavailable: {result['error'] or result['stderr']}"
    return metadata


def _cpu_list_contains(text: str, cpu: int) -> bool:
    """Read kernel CPU-list syntax without accepting a malformed topology."""
    if not text.strip():
        raise ValueError("empty PMU CPU list")
    present = False
    for part in text.strip().split(","):
        fields = part.split("-")
        if len(fields) not in (1, 2) or any(not f.isascii() or not f.isdigit() for f in fields):
            raise ValueError(f"invalid PMU CPU list: {text!r}")
        lo, hi = int(fields[0]), int(fields[-1])
        if lo > hi:
            raise ValueError(f"invalid PMU CPU range: {part!r}")
        present |= lo <= cpu <= hi
    return present


def probe_pmu(enabled: bool, affinity_prefix: list[str] | None = None,
              cpu: int | None = None,
              sysfs: Path = Path("/sys/bus/event_source/devices")) -> dict[str, Any]:
    """Probe the selected task's PMU, not the PMU of an arbitrary core.

    On Linux hybrid systems generic hardware events expand into both PMUs;
    select the PMU belonging to the *pinned* CPU instead of combining events
    or declaring the other core's <not counted> row a failure.
    """
    status = {
        "probed": enabled,
        "perf_path": shutil.which("perf"),
        "available": False,
        "reason": "not probed",
        "perf_event_paranoid": read_first("/proc/sys/kernel/perf_event_paranoid"),
        "probe_cpu": cpu,
        "probe_pmu": None,
    }
    if not enabled:
        return status
    if not status["perf_path"]:
        status["reason"] = "perf is not installed"
        return status
    paths = {name: sysfs / name for name in ("cpu_core", "cpu_atom")}
    if any(p.exists() for p in paths.values()):
        if cpu is None or not affinity_prefix or not all(p.is_dir() for p in paths.values()):
            status["reason"] = "hybrid PMU class requires an identified pinned CPU and complete topology"
            return status
        try:
            matches = [name for name, p in paths.items()
                       if _cpu_list_contains((p / "cpus").read_text(), cpu)]
        except (OSError, ValueError) as exc:
            status["reason"] = f"hybrid PMU topology unavailable: {exc}"
            return status
        if len(matches) != 1:
            status["reason"] = f"CPU {cpu} belongs to {len(matches)} hybrid PMUs; cannot select one"
            return status
        status["probe_pmu"] = matches[0]
    pmu = status["probe_pmu"]
    def event(name: str) -> str:
        return f"{pmu}/{name}/" if pmu else name
    command = [*(affinity_prefix or []), status["perf_path"], "stat", "--all-user",
               "--no-big-num", "-x", ",", "-e", f"{{{event('cycles')},{event('instructions')}}}",
               "--", "/bin/true"]
    result = invoke(command, timeout_s=15, env=controlled_environment())
    status["probe_command"] = command
    status["probe_returncode"] = result["returncode"]
    status["probe_stderr"] = result["stderr"]
    counts: dict[str, float] = {}
    invalid: set[str] = set()
    required = {event("cycles"), event("instructions")}
    for row in csv.reader(result["stderr"].splitlines()):
        if len(row) < 3:
            continue
        label = row[2].strip()
        label = label.removesuffix(":u").removesuffix(":k")
        if label.endswith(("/u", "/k")):
            label = label[:-1]
        if label not in required:
            continue
        if label in counts:
            invalid.add(label)
        try:
            count = float(row[0].strip())
            running = float(row[4].strip())
        except (ValueError, IndexError):
            invalid.add(label)
            continue
        if not math.isfinite(count) or count <= 0 or not math.isfinite(running) or not 95 <= running <= 100.01:
            invalid.add(label)
            continue
        counts[label] = count
    status["observed_counts"] = counts
    if result["ok"] and counts.keys() >= required and not invalid:
        status["available"] = True
        status["reason"] = ("perf returned usable cycles and instructions counters for the selected CPU/PMU"
                            if cpu is not None else
                            "perf returned usable generic cycles and instructions counters; probe was unpinned")
    elif result["ok"]:
        status["reason"] = "perf exited zero without usable cycles/instructions counters (missing, duplicated, invalid, or running below 95%)"
    else:
        detail = first_nonempty_line(result["stderr"]) or result["error"] or f"exit={result['returncode']}"
        status["reason"] = detail
    return status


def select_work_root(user_path: str | None) -> tuple[Path, dict[str, Any]]:
    """Choose the writable candidate with the most free space, never /tmp by habit."""
    if user_path:
        chosen = Path(user_path).expanduser().resolve()
        chosen.mkdir(parents=True, exist_ok=True)
        usage = shutil.disk_usage(chosen)
        return chosen, {"selected": str(chosen), "reason": "--work-dir", "free_bytes": usage.free}

    candidates = [REPO_ROOT, Path("/var/tmp"), Path("/tmp")]
    usable: list[tuple[int, Path]] = []
    for candidate in candidates:
        try:
            candidate.mkdir(parents=True, exist_ok=True)
            if os.access(candidate, os.W_OK | os.X_OK):
                usable.append((shutil.disk_usage(candidate).free, candidate.resolve()))
        except OSError:
            continue
    if not usable:
        raise RuntimeError("no writable benchmark work directory found")
    free_bytes, chosen = max(usable, key=lambda item: item[0])
    return chosen, {
        "selected": str(chosen),
        "reason": "largest writable candidate among repository, /var/tmp, /tmp",
        "free_bytes": free_bytes,
        "candidates": [{"path": str(path), "free_bytes": free} for free, path in usable],
    }


def choose_affinity(cpu_option: str) -> tuple[list[str], dict[str, Any], list[str]]:
    """Pin benchmark children when taskset and the current cpuset allow it."""
    warnings: list[str] = []
    try:
        allowed = sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        allowed = []
    metadata: dict[str, Any] = {"requested": cpu_option, "allowed_cpus": allowed, "applied": False}
    if cpu_option.lower() in ("none", "off", "no"):
        metadata["reason"] = "affinity disabled by user"
        return [], metadata, warnings
    if not allowed:
        metadata["reason"] = "sched_getaffinity unavailable"
        warnings.append("CPU affinity could not be determined; samples are not pinned.")
        return [], metadata, warnings
    if cpu_option.lower() == "auto":
        cpu = allowed[0]
    else:
        try:
            cpu = int(cpu_option)
        except ValueError:
            metadata["reason"] = f"invalid CPU: {cpu_option}"
            warnings.append(metadata["reason"])
            return [], metadata, warnings
        if cpu not in allowed:
            metadata["reason"] = f"CPU {cpu} is outside current affinity set {allowed}"
            warnings.append(metadata["reason"])
            return [], metadata, warnings
    taskset = shutil.which("taskset")
    if not taskset:
        metadata["reason"] = "taskset not installed"
        warnings.append("taskset is unavailable; samples are not pinned.")
        return [], metadata, warnings
    probe = invoke([taskset, "-c", str(cpu), "/bin/true"], timeout_s=10)
    if not probe["ok"]:
        metadata["reason"] = first_nonempty_line(probe["stderr"]) or probe["error"] or "taskset probe failed"
        warnings.append(f"CPU pinning unavailable: {metadata['reason']}")
        return [], metadata, warnings
    metadata.update({"applied": True, "cpu": cpu, "reason": "taskset pinning"})
    return [taskset, "-c", str(cpu)], metadata, warnings


def collect_environment(pinning: dict[str, Any], work_root: dict[str, Any], pmu: dict[str, Any],
                        compilers: list[CompilerSpec]) -> dict[str, Any]:
    cpu_models: list[str] = []
    try:
        for line in Path("/proc/cpuinfo").read_text(errors="replace").splitlines():
            if line.lower().startswith("model name") and ":" in line:
                model = line.split(":", 1)[1].strip()
                if model not in cpu_models:
                    cpu_models.append(model)
    except OSError:
        pass
    governors: dict[str, str] = {}
    for governor in sorted(Path("/sys/devices/system/cpu").glob("cpu*/cpufreq/scaling_governor")):
        value = read_first(str(governor))
        if value is not None:
            governors[str(governor.parent.parent.name)] = value
    hypervisor = any("hypervisor" in line.lower()
                     for line in read_first("/proc/cpuinfo").splitlines()) if read_first("/proc/cpuinfo") else False
    return {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "python": sys.version.replace("\n", " "),
        "platform": platform.platform(),
        "uname": list(os.uname()),
        "cpu_models": cpu_models,
        "hypervisor_detected": hypervisor,
        "cpu_governors": governors,
        "pinning": pinning,
        "work_root": work_root,
        "swap": read_first("/proc/swaps"),
        "meminfo": read_first("/proc/meminfo"),
        "pmu": pmu,
        "git": git_metadata(),
        "compiler_versions": {compiler.key: command_version(compiler.executable) for compiler in compilers},
    }


def controlled_environment() -> dict[str, str]:
    environment = dict(os.environ)
    # The benchmark programs are single-threaded, but make inherited BLAS/OpenMP
    # environments unable to accidentally alter future benchmark additions.
    environment.update({
        "LC_ALL": "C",
        "LANG": "C",
        "TZ": "UTC",
        "OMP_NUM_THREADS": "1",
        "OPENBLAS_NUM_THREADS": "1",
        "MKL_NUM_THREADS": "1",
        "VECLIB_MAXIMUM_THREADS": "1",
    })
    return environment


def run_binary(binary: Path, affinity_prefix: list[str], timeout_s: int,
               environment: dict[str, str]) -> dict[str, Any]:
    return invoke([*affinity_prefix, str(binary)], timeout_s=timeout_s,
                  cwd=REPO_ROOT, env=environment)


def output_path(work: Path, artifacts: Path | None, benchmark: Benchmark,
                compiler: CompilerSpec) -> Path:
    if artifacts is not None:
        location = artifacts / benchmark.name / compiler.key
        location.mkdir(parents=True, exist_ok=True)
        return location / "benchmark.bin"
    return work / f"{benchmark.name}_{compiler.key}"


REFERENCE_COMPILERS = ("gcc", "clang", "icx")


def evaluate_correctness(entry: dict[str, Any], compiler_keys: list[str]) -> None:
    """Mark output stability and cross-compiler equivalence without hiding errors."""
    successful: dict[str, list[str]] = {}
    for key in compiler_keys:
        timed = entry["runs"].get(key, [])
        # Warm-ups are excluded from timing, never from correctness. A wrong
        # first execution or unstable control cannot certify later samples.
        observed = [*entry.get("warmups", {}).get(key, []), *timed]
        # Exact stdout bytes (hash), never lossy decoded/stripped text.
        outputs = [run.get("stdout_sha256", run["stdout"]) for run in observed if run.get("ok")]
        complete = bool(timed) and len(outputs) == len(observed)
        stable = len(set(outputs)) == 1 if outputs else None
        if complete and stable:
            successful[key] = outputs
        entry["correctness"][key] = {
            "stable_within_compiler": stable,
            "all_runs_successful": complete,
            "output": next((run["stdout"] for run in observed if run.get("ok")), None),
            "matches_baseline": False if observed and not (complete and stable) else None,
        }
    # The oracle is an independent reference compiler: GCC when requested,
    # else the first requested of Clang/ICX. A failed oracle is never replaced
    # by a later one, and an LCCC build (lccc/lev/ccc/--candidate) never
    # becomes the oracle: a candidate would certify its own wrong output.
    baseline = next((key for key in REFERENCE_COMPILERS if key in compiler_keys), None)
    entry["baseline_compiler"] = baseline
    if baseline is None or baseline not in successful:
        return
    baseline_check = entry["correctness"][baseline]
    if not baseline_check["stable_within_compiler"] or not baseline_check["all_runs_successful"]:
        return
    baseline_output = successful[baseline][0]
    for key, outputs in successful.items():
        check = entry["correctness"][key]
        check["matches_baseline"] = (check["stable_within_compiler"]
                                     and check["all_runs_successful"]
                                     and outputs[0] == baseline_output)


def pair_ratios(rounds: list[dict[str, Any]], numerator: str, denominator: str,
                seed: int) -> dict[str, Any] | None:
    values: list[float] = []
    raw: list[dict[str, Any]] = []
    for round_info in rounds:
        numerator_run = round_info["runs"].get(numerator)
        denominator_run = round_info["runs"].get(denominator)
        if not numerator_run or not denominator_run:
            continue
        if not numerator_run.get("ok") or not denominator_run.get("ok"):
            continue
        denominator_time = denominator_run["wall_s"]
        if denominator_time <= 0.0:
            continue
        ratio = numerator_run["wall_s"] / denominator_time
        values.append(ratio)
        raw.append({"round": round_info["round"], "ratio": ratio})
    stats = summarize(values, seed)
    if stats is None:
        return None
    stats["raw"] = raw
    stats["interpretation"] = "numerator/denominator; <1 means numerator faster"
    return stats


def quality_warnings(stats: dict[str, Any] | None) -> list[str]:
    if stats is None:
        return ["no successful timing samples"]
    warnings: list[str] = []
    if stats["median"] < 0.020:
        warnings.append("median runtime below 20 ms; process-launch and scheduler noise may dominate")
    if stats["n"] < 7:
        warnings.append("fewer than seven timed paired rounds")
    if stats["cv"] > 0.05:
        warnings.append(f"wall-clock coefficient of variation is {stats['cv']:.1%} (>5%)")
    if stats["outlier_count_mad"]:
        warnings.append(f"{stats['outlier_count_mad']} MAD outlier(s) retained (never silently discarded)")
    return warnings


def run_benchmark(benchmark: Benchmark, compilers: list[CompilerSpec], args: argparse.Namespace,
                  work: Path, artifacts: Path | None, affinity_prefix: list[str],
                  environment: dict[str, str], seed: int) -> dict[str, Any]:
    source = source_for(benchmark)
    entry: dict[str, Any] = {
        "name": benchmark.name,
        "description": benchmark.description,
        "tags": list(benchmark.tags),
        "source": str(source) if source else None,
        "source_sha256": sha256_file(source) if source else None,
        "compile": {},
        "warmups": {},
        "rounds": [],
        "runs": {},
        "runtime": {},
        "correctness": {},
        "ratios_to_gcc": {},
        "ratios_from_lccc": {},
        "warnings": [],
    }
    if source is None:
        entry["warnings"].append("source not found")
        return entry

    binaries: dict[str, Path] = {}
    runnable: list[CompilerSpec] = []
    for compiler in compilers:
        binary = output_path(work, artifacts, benchmark, compiler)
        compiled = compile_one(compiler, benchmark, source, binary, artifacts)
        entry["compile"][compiler.key] = compiled
        if compiled["ok"]:
            binaries[compiler.key] = binary
            runnable.append(compiler)
        else:
            entry["warnings"].append(f"{compiler.label} compile failure: {compiled['error'] or first_nonempty_line(compiled['stderr'])}")

    # Warm-ups are intentionally outside the sample distribution.  Their only
    # job is loader/code-path stabilization and early crash discovery.
    active = list(runnable)
    warmup_count = 0 if args.skip_perf else args.warmup
    rng = random.Random(seed)
    for warmup in range(warmup_count):
        order = list(active)
        rng.shuffle(order)
        for compiler in order:
            run = run_binary(binaries[compiler.key], affinity_prefix, benchmark.timeout_s, environment)
            entry["warmups"].setdefault(compiler.key, []).append(run)
            if not run["ok"]:
                entry["warnings"].append(
                    f"{compiler.label} warm-up failure: {run['error'] or first_nonempty_line(run['stderr']) or 'non-zero exit'}")
                active = [item for item in active if item.key != compiler.key]

    timed_rounds = 1 if args.skip_perf else args.reps
    previous_order = None
    for round_number in range(timed_rounds):
        order = balanced_compiler_order(active, rng, round_number, previous_order)
        previous_order = order
        round_info: dict[str, Any] = {"round": round_number, "order": [item.key for item in order], "runs": {}}
        for compiler in order:
            run = run_binary(binaries[compiler.key], affinity_prefix, benchmark.timeout_s, environment)
            round_info["runs"][compiler.key] = run
            entry["runs"].setdefault(compiler.key, []).append(run)
        entry["rounds"].append(round_info)
        failed = [key for key, run in round_info["runs"].items() if not run["ok"]]
        if failed:
            for key in failed:
                entry["warnings"].append(f"runtime failure in timed round {round_number}: {key}")
            active = [item for item in active if item.key not in failed]

    compiler_keys = [compiler.key for compiler in compilers]
    evaluate_correctness(entry, compiler_keys)
    if args.skip_perf:
        # A single execution is retained solely for output/crash evidence.  Do
        # not turn one timing observation into a performance statistic.
        for compiler in compilers:
            entry["runtime"][compiler.key] = {
                "wall": None,
                "child_cpu": None,
                "raw_wall_s": [],
                "quality_warnings": ["timing deliberately skipped"],
            }
        return entry

    for compiler in compilers:
        wall_values = [run["wall_s"] for run in entry["runs"].get(compiler.key, []) if run["ok"]]
        cpu_values = [run["child_cpu_s"] for run in entry["runs"].get(compiler.key, []) if run["ok"]]
        stats = summarize(wall_values, seed + stable_seed(benchmark.name, compiler.key) % 1_000_000)
        entry["runtime"][compiler.key] = {
            "wall": stats,
            "child_cpu": summarize(cpu_values, seed + 17 + stable_seed(compiler.key, benchmark.name) % 1_000_000),
            "raw_wall_s": wall_values,
            "quality_warnings": quality_warnings(stats),
        }

    if "gcc" in compiler_keys:
        for compiler in compilers:
            if compiler.key == "gcc":
                continue
            if not all(entry["correctness"][key]["matches_baseline"] is True
                       for key in (compiler.key, "gcc")):
                continue
            ratio = pair_ratios(entry["rounds"], compiler.key, "gcc",
                                seed + stable_seed(benchmark.name, compiler.key, "gcc") % 1_000_000)
            entry["ratios_to_gcc"][compiler.key] = ratio

    # Keep direct paired comparisons against every installed reference.  A
    # reference compiler with the lowest independent median is selected later
    # for the LCCC-vs-best matrix; the raw pairs remain available for audit.
    if "lccc" in compiler_keys:
        for compiler in compilers:
            if compiler.key == "lccc":
                continue
            if not all(entry["correctness"][key]["matches_baseline"] is True
                       for key in ("lccc", compiler.key)):
                continue
            entry["ratios_from_lccc"][compiler.key] = pair_ratios(
                entry["rounds"], "lccc", compiler.key,
                seed + stable_seed(benchmark.name, "lccc", compiler.key) % 1_000_000,
            )
    return entry


def best_reference_for_entry(entry: dict[str, Any]) -> tuple[str, float] | None:
    """Return the lowest-median available reference compiler for one workload."""
    candidates: list[tuple[str, float]] = []
    for key, runtime in entry.get("runtime", {}).items():
        if key == "lccc" or entry.get("correctness", {}).get(key, {}).get("matches_baseline") is not True:
            continue
        wall = runtime.get("wall") if runtime else None
        if wall and wall.get("median") is not None:
            candidates.append((key, wall["median"]))
    return min(candidates, key=lambda item: item[1]) if candidates else None


def aggregate_lccc_vs_gcc(entries: list[dict[str, Any]]) -> dict[str, Any] | None:
    ratios: list[tuple[str, float]] = []
    for entry in entries:
        ratio = entry.get("ratios_to_gcc", {}).get("lccc")
        lccc_correct = entry.get("correctness", {}).get("lccc", {}).get("matches_baseline")
        gcc_correct = entry.get("correctness", {}).get("gcc", {}).get("matches_baseline")
        if ratio and ratio.get("median") and lccc_correct is True and gcc_correct is True:
            ratios.append((entry["name"], ratio["median"]))
    if not ratios:
        return None
    values = [ratio for _, ratio in ratios]
    geometric = math.exp(statistics.fmean(math.log(value) for value in values))
    arithmetic = statistics.fmean(values)
    best_name, best_ratio = min(ratios, key=lambda item: item[1])
    worst_name, worst_ratio = max(ratios, key=lambda item: item[1])
    return {
        "n_correct_benchmarks": len(ratios),
        "geometric_mean_ratio": geometric,
        "arithmetic_mean_ratio": arithmetic,
        "best": {"benchmark": best_name, "ratio": best_ratio},
        "worst": {"benchmark": worst_name, "ratio": worst_ratio},
        "interpretation": "LCCC/GCC paired median ratios; <1 means LCCC faster",
    }


def duration_sensitivity(entries: list[dict[str, Any]], threshold_s: float = 0.020) -> dict[str, Any]:
    """Secondary, explicitly selected view; never erase short rows or change the main aggregate."""
    selected, excluded = [], []
    for entry in entries:
        ratio = entry.get("ratios_to_gcc", {}).get("lccc")
        if not ratio or not ratio.get("median"):
            continue
        if any(entry.get("correctness", {}).get(key, {}).get("matches_baseline") is not True
               for key in ("lccc", "gcc")):
            continue
        walls = [entry.get("runtime", {}).get(key, {}).get("wall") for key in ("lccc", "gcc")]
        if any(not wall or wall.get("median") is None or wall["median"] < threshold_s for wall in walls):
            excluded.append(entry["name"])
        else:
            selected.append(entry)
    return {"minimum_median_seconds_each_compiler": threshold_s,
            "excluded_short_or_unknown": excluded,
            "included": [entry["name"] for entry in selected],
            "aggregate": aggregate_lccc_vs_gcc(selected),
            "interpretation": "diagnostic sensitivity only; nonrepresentative selection, not a replacement score"}


def aggregate_lccc_vs_best_reference(entries: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Aggregate LCCC against the fastest available non-LCCC compiler per row."""
    ratios: list[tuple[str, str, float]] = []
    for entry in entries:
        reference = best_reference_for_entry(entry)
        if not reference:
            continue
        reference_key, _ = reference
        ratio = entry.get("ratios_from_lccc", {}).get(reference_key)
        lccc_correct = entry.get("correctness", {}).get("lccc", {}).get("matches_baseline")
        reference_correct = entry.get("correctness", {}).get(reference_key, {}).get("matches_baseline")
        if ratio and ratio.get("median") and lccc_correct is True and reference_correct is True:
            ratios.append((entry["name"], reference_key, ratio["median"]))
    if not ratios:
        return None
    values = [ratio for _, _, ratio in ratios]
    best_name, best_ref, best_ratio = min(ratios, key=lambda item: item[2])
    worst_name, worst_ref, worst_ratio = max(ratios, key=lambda item: item[2])
    return {
        "n_correct_benchmarks": len(ratios),
        "geometric_mean_ratio": math.exp(statistics.fmean(math.log(value) for value in values)),
        "arithmetic_mean_ratio": statistics.fmean(values),
        "best": {"benchmark": best_name, "reference": best_ref, "ratio": best_ratio},
        "worst": {"benchmark": worst_name, "reference": worst_ref, "ratio": worst_ratio},
        "interpretation": "LCCC / fastest available reference paired median; <1 means LCCC faster",
    }


def format_seconds(value: float | None) -> str:
    if value is None:
        return "—"
    if value < 1.0:
        return f"{value * 1000.0:.2f} ms"
    return f"{value:.4f} s"


def format_ratio(value: float | None) -> str:
    if value is None:
        return "—"
    if value < 1.0:
        return f"{value:.3f} ({1.0 / value:.2f}× faster)"
    return f"{value:.3f} ({value:.2f}× slower)"


def render_terminal(report: dict[str, Any]) -> str:
    lines: list[str] = []
    metadata = report["metadata"]
    measurement_class = ("VM screening" if metadata["hypervisor_detected"] else
                         "bare-metal candidate" if metadata["pmu"]["available"] else
                         "PMU-unverified screening")
    lines.append("=" * 108)
    lines.append("LCCC generated-code benchmark report")
    lines.append(f"Measurement class: {measurement_class}; PMU: {metadata['pmu']['reason']}")
    lines.append(f"Pinning: {metadata['pinning']}; rounds: {report['settings']['reps']}; warm-ups: {report['settings']['warmup']}")
    if report["settings"]["skip_perf"]:
        lines.append("Correctness-only mode: no performance statistic is emitted.")
    else:
        lines.append("Primary statistic is paired median wall time.  All samples are retained; min is diagnostic only.")
    lines.append("=" * 108)
    lines.append(f"{'Benchmark':<22} {'LCCC median':>14} {'GCC median':>14} {'LCCC/GCC paired':>29} {'Correct':>10}")
    lines.append("-" * 108)
    for entry in report["benchmarks"]:
        lccc = entry.get("runtime", {}).get("lccc", {}).get("wall")
        gcc = entry.get("runtime", {}).get("gcc", {}).get("wall")
        ratio = entry.get("ratios_to_gcc", {}).get("lccc")
        ratio_text = format_ratio(ratio["median"]) if ratio else "—"
        if ratio and ratio.get("median_ci95_bootstrap"):
            low, high = ratio["median_ci95_bootstrap"]
            ratio_text += f" [{low:.3f}, {high:.3f}]"
        correct = entry.get("correctness", {}).get("lccc", {}).get("matches_baseline")
        correct_text = "pass" if correct is True else ("FAIL" if correct is False else "—")
        lines.append(f"{entry['name']:<22} {format_seconds(lccc['median'] if lccc else None):>14} "
                     f"{format_seconds(gcc['median'] if gcc else None):>14} {ratio_text:>29} {correct_text:>10}")
        for reference_key, reference_ratio in entry.get("ratios_from_lccc", {}).items():
            if reference_key == "gcc" or not reference_ratio:
                continue
            reference_wall = entry.get("runtime", {}).get(reference_key, {}).get("wall")
            reference_text = format_ratio(reference_ratio.get("median"))
            ci = reference_ratio.get("median_ci95_bootstrap")
            if ci:
                reference_text += f" [{ci[0]:.3f}, {ci[1]:.3f}]"
            lines.append(f"  LCCC/{reference_key.upper()} paired: {reference_text}; "
                         f"{reference_key.upper()} median={format_seconds(reference_wall['median'] if reference_wall else None)}")
        for warning in entry.get("warnings", []):
            lines.append(f"  ! {warning}")
        for compiler_key, runtime in entry.get("runtime", {}).items():
            for warning in runtime.get("quality_warnings", []):
                lines.append(f"  ! {compiler_key}: {warning}")
    aggregate = report.get("aggregate_lccc_vs_gcc")
    if aggregate:
        lines.append("-" * 108)
        lines.append("LCCC/GCC aggregate over correct benchmark pairs only:")
        lines.append(f"  n={aggregate['n_correct_benchmarks']}; geometric mean={aggregate['geometric_mean_ratio']:.4f}; "
                     f"arithmetic mean={aggregate['arithmetic_mean_ratio']:.4f}")
        lines.append(f"  best={aggregate['best']['benchmark']} ({format_ratio(aggregate['best']['ratio'])}); "
                     f"worst={aggregate['worst']['benchmark']} ({format_ratio(aggregate['worst']['ratio'])})")
    sensitivity = duration_sensitivity(report["benchmarks"])
    if aggregate:
        lines.append("  WARNING: workload mix and extreme specialized/short tests can dominate; not a whole-application score.")
        if sensitivity["aggregate"]:
            filtered = sensitivity["aggregate"]
            lines.append(f"  sensitivity only (both medians >=20 ms): n={filtered['n_correct_benchmarks']}; "
                         f"geomean={filtered['geometric_mean_ratio']:.4f}; "
                         f"excluded={len(sensitivity['excluded_short_or_unknown'])}; selection is not representative")
    best_aggregate = report.get("aggregate_lccc_vs_best_reference")
    if best_aggregate:
        lines.append("LCCC / fastest available reference aggregate over correct benchmark pairs only:")
        lines.append(f"  n={best_aggregate['n_correct_benchmarks']}; geometric mean={best_aggregate['geometric_mean_ratio']:.4f}; "
                     f"arithmetic mean={best_aggregate['arithmetic_mean_ratio']:.4f}")
        lines.append(f"  best={best_aggregate['best']['benchmark']} vs {best_aggregate['best']['reference']} "
                     f"({format_ratio(best_aggregate['best']['ratio'])}); worst={best_aggregate['worst']['benchmark']} "
                     f"vs {best_aggregate['worst']['reference']} ({format_ratio(best_aggregate['worst']['ratio'])})")
    if report["unavailable_compilers"]:
        lines.append("Unavailable reference compilers:")
        for key, reason in report["unavailable_compilers"].items():
            lines.append(f"  {key}: {reason}")
    lines.append("Raw per-round samples, commands, metadata, and optional disassembly are retained in JSON/artifacts.")
    return "\n".join(lines)


def render_markdown(report: dict[str, Any]) -> str:
    metadata = report["metadata"]
    method = ("correctness-only execution; no timing statistic emitted"
              if report["settings"]["skip_perf"]
              else "randomized balanced two-round blocks for two compilers; independent shuffle otherwise; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal")
    compiler_labels = {compiler["key"]: compiler["label"]
                       for compiler in report.get("compilers", [])}
    compiler_metadata = {compiler["key"]: compiler for compiler in report.get("compilers", [])}
    reference_keys = [compiler["key"] for compiler in report.get("compilers", [])
                      if compiler["key"] != "lccc"]
    shard_cpus = {shard["report"]: "; ".join(shard.get("metadata", {}).get("cpu_models", []))
                  for shard in report.get("shards", [])}
    header = ["Benchmark", "LCCC median"]
    header.extend(f"{compiler_labels.get(key, key).upper()} median" for key in reference_keys)
    header.extend(["Best reference", "LCCC/best paired (95% bootstrap CI)", "Correct"])
    separator = ["---", "---:"] + ["---:" for _ in reference_keys] + ["---", "---:", ":---:"]
    if shard_cpus:
        header.append("Measurement CPU / shard")
        separator.append("---")

    lines = [
        "# LCCC benchmark report",
        "",
        f"- **UTC:** `{metadata['timestamp_utc']}`",
        f"- **CPU model(s):** `{'; '.join(metadata['cpu_models']) or 'unknown'}`",
        f"- **Hypervisor detected:** `{metadata['hypervisor_detected']}`",
        f"- **CPU pinning:** `{metadata['pinning']}`",
        f"- **PMU:** `{metadata['pmu']['reason']}`",
        f"- **LCCC revision:** `{metadata['git'].get('revision', 'unknown')}`",
        f"- **LCCC binary SHA-256:** `{compiler_metadata.get('lccc', {}).get('binary_sha256', 'unknown')}`",
        f"- **Optimization:** `{report['settings']['opt']}`; common flags: `{report['settings'].get('common_cflags', [])}`",
        f"- **Rounds / excluded warm-ups:** `{report['settings']['reps']} / {report['settings']['warmup']}`",
        f"- **Method:** {method}.",
        "",
        "| " + " | ".join(header) + " |",
        "| " + " | ".join(separator) + " |",
    ]
    for entry in report["benchmarks"]:
        lccc = entry.get("runtime", {}).get("lccc", {}).get("wall")
        row = [f"`{entry['name']}`", format_seconds(lccc["median"] if lccc else None)]
        for reference_key in reference_keys:
            wall = entry.get("runtime", {}).get(reference_key, {}).get("wall")
            row.append(format_seconds(wall["median"] if wall else None))
        best_reference = best_reference_for_entry(entry)
        if best_reference:
            reference_key, _ = best_reference
            ratio = entry.get("ratios_from_lccc", {}).get(reference_key)
            if ratio:
                ci = ratio.get("median_ci95_bootstrap")
                ratio_text = f"{ratio['median']:.4f}" + (f" [{ci[0]:.4f}, {ci[1]:.4f}]" if ci else "")
            else:
                ratio_text = "—"
            best_text = compiler_labels.get(reference_key, reference_key)
        else:
            best_text = "—"
            ratio_text = "—"
        correct = entry.get("correctness", {}).get("lccc", {}).get("matches_baseline")
        correct_text = "pass" if correct is True else ("FAIL" if correct is False else "—")
        row.extend([best_text, ratio_text, correct_text])
        if shard_cpus:
            shard = entry.get("measurement_shard", "unknown")
            row.append(f"{shard_cpus.get(shard, 'unknown')} / `{shard}`")
        lines.append("| " + " | ".join(row) + " |")

    if report.get("shards"):
        lines.extend(["", "Sharded measurements: rows retain their runner identity; aggregates describe workload/host pairs, not one CPU."])
    aggregate = report.get("aggregate_lccc_vs_gcc")
    if aggregate:
        lines.extend([
            "",
            "## Aggregate LCCC/GCC (correct pairs only)",
            "",
            "Workload mix and extreme specialized or short tests can dominate; this is not a whole-application score.",
            "",
            f"- Geometric mean ratio: `{aggregate['geometric_mean_ratio']:.4f}`",
            f"- Arithmetic mean ratio: `{aggregate['arithmetic_mean_ratio']:.4f}`",
            f"- Best individual ratio: `{aggregate['best']['benchmark']}` = `{aggregate['best']['ratio']:.4f}`",
            f"- Worst individual ratio: `{aggregate['worst']['benchmark']}` = `{aggregate['worst']['ratio']:.4f}`",
        ])
    best_aggregate = report.get("aggregate_lccc_vs_best_reference")
    if best_aggregate:
        lines.extend([
            "",
            "## Aggregate LCCC / fastest available reference (correct pairs only)",
            "",
            f"- Geometric mean ratio: `{best_aggregate['geometric_mean_ratio']:.4f}`",
            f"- Arithmetic mean ratio: `{best_aggregate['arithmetic_mean_ratio']:.4f}`",
            f"- Best individual ratio: `{best_aggregate['best']['benchmark']}` vs `{best_aggregate['best']['reference']}` = `{best_aggregate['best']['ratio']:.4f}`",
            f"- Worst individual ratio: `{best_aggregate['worst']['benchmark']}` vs `{best_aggregate['worst']['reference']}` = `{best_aggregate['worst']['ratio']:.4f}`",
        ])
    lines.extend([
        "",
        "A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.",
    ])
    return "\n".join(lines) + "\n"

SCHEMA = "lccc-bench-v2"


def merge_reports(paths: list[Path]) -> dict[str, Any]:
    """Combine the shard reports of one run into the single canonical report.

    The corpus is split across runners so its wall clock is not the sum of
    every kernel, and the split must not change what the evidence IS: the
    published report is still one `lccc-bench-v2` document with one row per
    benchmark, the same aggregates, and the same rendering.  This function is
    the single owner of that combination, so a shard cannot publish a partial
    aggregate and a consumer cannot see a half corpus.

    Refusals are deliberate: a shard that did not finish (`complete` false), a
    duplicated benchmark, a schema from another version, or shards measured
    with different settings are all errors.  Merging them would produce a
    document that looks complete and is not.
    """
    reports = []
    for path in paths:
        report = json.loads(path.read_text())
        if not isinstance(report, dict) or "benchmarks" not in report:
            raise SystemExit(f"{path}: not a benchmark report")
        if report.get("schema") != SCHEMA:
            raise SystemExit(f"{path}: schema {report.get('schema')!r}, expected {SCHEMA!r}")
        if not report.get("complete"):
            raise SystemExit(f"{path}: shard did not complete; refusing to publish a "
                             "partial corpus as complete")
        reports.append((path, report))
    if not reports:
        raise SystemExit("no shard reports to merge")

    base_path, base = reports[0]
    settings = base.get("settings")
    for path, report in reports[1:]:
        if report.get("settings") != settings:
            raise SystemExit(f"{path}: settings differ from {base_path}; shards must "
                             "measure the same protocol")
    compilers = base.get("compilers")
    for path, report in reports[1:]:
        if report.get("compilers") != compilers:
            raise SystemExit(f"{path}: compiler identities differ from {base_path}")

    entries: list[dict[str, Any]] = []
    seen: dict[str, str] = {}
    shards: list[dict[str, Any]] = []
    for path, report in reports:
        names: list[str] = []
        for entry in report["benchmarks"]:
            name = entry.get("name")
            if name in seen:
                raise SystemExit(f"{path}: benchmark {name!r} also appears in {seen[name]}")
            seen[name] = path.name
            names.append(name)
            entries.append(dict(entry, measurement_shard=path.name))
        shards.append({"report": path.name, "benchmarks": names,
                       "settings": report["settings"], "metadata": report["metadata"]})

    merged = json.loads(json.dumps(base))
    merged["benchmarks"] = entries
    merged["complete"] = True
    merged["shards"] = shards
    merged["metadata"] = dict(base["metadata"], merged_from=[p.name for p in paths])
    merged["metadata"]["cpu_models"] = sorted({
        model for _, report in reports for model in report["metadata"]["cpu_models"]
    })
    merged["metadata"]["hypervisor_detected"] = any(
        report["metadata"]["hypervisor_detected"] for _, report in reports
    )
    merged["metadata"]["pinning"] = {
        "per_shard": {path.name: report["metadata"]["pinning"] for path, report in reports}
    }
    merged["metadata"]["pmu"] = {
        "available": all(report["metadata"]["pmu"]["available"] for _, report in reports),
        "reason": "per-shard PMU evidence; see shard metadata",
    }
    merged["runner_warnings"] = [warning for _, report in reports
                                 for warning in report.get("runner_warnings", [])]
    if len({tuple(report["metadata"]["cpu_models"]) for _, report in reports}) > 1:
        merged["runner_warnings"].append(
            "Mixed-CPU shards: aggregate describes workload/host pairs, not performance on one CPU.")
    merged["aggregate_lccc_vs_gcc"] = aggregate_lccc_vs_gcc(entries)
    merged["duration_sensitivity_lccc_vs_gcc"] = duration_sensitivity(entries)
    merged["aggregate_lccc_vs_best_reference"] = aggregate_lccc_vs_best_reference(entries)
    return merged


def list_benchmarks() -> None:
    print("Available benchmarks:")
    for benchmark in BENCHMARKS:
        source = source_for(benchmark)
        status = str(source.relative_to(REPO_ROOT)) if source else "MISSING"
        print(f"  {benchmark.name:<20} {status:<54} {benchmark.description}")


# Flags whose value is itself a compiler flag and therefore always begins with
# a hyphen: `-O2`, `-Os`, `-march=...`, `-fno-...`.
_FLAG_VALUED_OPTIONS = ("--opt", "--cflag")


def normalize_flag_valued_options(argv):
    """Rewrite `--opt -O2` as `--opt=-O2` so both spellings parse.

    argparse decides whether a token is an option or a value purely from its
    leading hyphen, so a space-separated value that begins with `-` is always
    read as the next option and the flag dies with "expected one argument".
    Every legal value of `--opt`/`--cflag` begins with a hyphen, which makes
    the natural spelling unusable rather than merely ambiguous.  Joining the
    pair before parsing keeps it working without weakening any other flag: a
    following token that starts with `--` is left alone, so `--opt --strict`
    still reports the missing value instead of silently swallowing `--strict`.

    Exactly one token is joined per occurrence, matching what argparse would
    have consumed.
    """
    normalized = []
    index = 0
    while index < len(argv):
        token = argv[index]
        if (
            token in _FLAG_VALUED_OPTIONS
            and index + 1 < len(argv)
            and not argv[index + 1].startswith("--")
        ):
            normalized.append(f"{token}={argv[index + 1]}")
            index += 2
            continue
        normalized.append(token)
        index += 1
    return normalized


def build_arg_parser():
    """The runner's command line, as a parser, so it can be tested directly."""
    parser = argparse.ArgumentParser(description="Paired, reproducible LCCC code-generation benchmark runner")
    parser.add_argument("--list", action="store_true", help="list benchmark IDs and exit")
    parser.add_argument("--only", action="append", help="comma-separated benchmark IDs; may be repeated")
    parser.add_argument("--compilers", default="lccc,gcc,clang,icx",
                        help="ordered compiler keys from lccc,ccc,gcc,clang,icx (default: %(default)s)")
    parser.add_argument("--lccc", default=str(DEFAULT_LCCC), help="path to LCCC executable")
    parser.add_argument("--gcc", default="gcc", help="GCC executable")
    parser.add_argument("--clang", default="clang", help="Clang executable")
    parser.add_argument("--icx", default="icx", help="ICX executable")
    parser.add_argument("--ccc", default="", help="path to the original Claude's C Compiler executable (Anthropic upstream)")
    parser.add_argument("--lev", default="", help="path to Lev Kropp's original lccc executable (levkropp/lccc)")
    parser.add_argument("--candidate", action="append", default=[], metavar="LABEL=EXECUTABLE",
                        help="additional, explicitly labelled LCCC-compatible arm; repeatable, never impersonates a vendor")
    parser.add_argument("--opt", default="-O2", help="optimization flag passed uniformly to every compiler")
    parser.add_argument("--cflag", action="append", default=[], help="additional common compiler flag; may be repeated")
    parser.add_argument("--reps", type=int, default=15, help="timed paired rounds per benchmark (default: %(default)s)")
    parser.add_argument("--warmup", type=int, default=2, help="excluded warm-up rounds per compiler (default: %(default)s)")
    parser.add_argument("--skip-perf", action="store_true", help="compile and run one correctness sample, no timing claim")
    parser.add_argument("--cpu", default="auto", help="CPU to pin with taskset, auto, or none (default: %(default)s)")
    parser.add_argument("--work-dir", help="temporary compile/run directory; default chooses largest writable filesystem")
    parser.add_argument("--artifact-dir", type=Path,
                        help="retain binaries, compile commands, objdump disassembly, sections, and sources here")
    parser.add_argument("--json", type=Path, help="write raw evidence/report JSON")
    parser.add_argument("--markdown", type=Path, help="write compact Markdown report")
    parser.add_argument("--seed", type=int, default=20260810, help="randomized-order/bootstrap seed")
    parser.add_argument("--no-pmu-probe", action="store_true", help="do not make the one perf availability probe")
    parser.add_argument("--strict", action="store_true",
                        help="non-zero exit on a compile/runtime failure, missing requested compiler, or non-match")
    parser.add_argument("--merge", type=Path, nargs="+", metavar="REPORT",
                        help="combine the shard reports of one run into the canonical "
                             "report and exit (used by the sharded benchmark lane)")
    return parser


def main(argv=None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = build_arg_parser()
    args = parser.parse_args(normalize_flag_valued_options(argv))

    if args.list:
        list_benchmarks()
        return 0
    if args.merge:
        merged = merge_reports(args.merge)
        terminal = render_terminal(merged)
        print(terminal)
        if args.markdown:
            args.markdown.parent.mkdir(parents=True, exist_ok=True)
            args.markdown.write_text(render_markdown(merged), encoding="utf-8")
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(merged, indent=2, sort_keys=True) + "\n")
        # A shard that failed is refused by merge_reports, so a merged report
        # is complete by construction; the exit status still reflects the
        # per-entry failures the shards recorded.
        failures = [(entry["name"], key)
                    for entry in merged["benchmarks"]
                    for key, result in entry.get("compile", {}).items()
                    if not result.get("ok")]
        if failures:
            print("\nBenchmark failures:", file=sys.stderr)
            for name, key in sorted(failures):
                print(f"  - {name}: {key} compile failure", file=sys.stderr)
            return 1
        return 0
    if args.reps < 1 or args.warmup < 0:
        parser.error("--reps must be >= 1 and --warmup must be >= 0")

    selected = parse_selection(args.only)
    all_names = {benchmark.name for benchmark in BENCHMARKS}
    unknown = sorted(selected - all_names)
    if unknown:
        parser.error(f"unknown benchmark ID(s): {', '.join(unknown)}; use --list")
    benchmarks = [benchmark for benchmark in BENCHMARKS if not selected or benchmark.name in selected]
    if not benchmarks:
        parser.error("selection is empty")

    try:
        compilers, unavailable = discover_compilers(args)
    except ValueError as error:
        parser.error(str(error))
    if not compilers:
        parser.error("no requested compiler is available")
    requested = [item.strip().lower() for item in args.compilers.split(",") if item.strip()]
    if "lccc" in requested and not any(compiler.key == "lccc" for compiler in compilers):
        parser.error(f"LCCC is unavailable: {unavailable.get('lccc', 'unknown reason')}; build it first")

    artifacts = args.artifact_dir.resolve() if args.artifact_dir else None
    if artifacts:
        artifacts.mkdir(parents=True, exist_ok=True)
    work_root, work_metadata = select_work_root(args.work_dir)
    affinity_prefix, pinning, affinity_warnings = choose_affinity(args.cpu)
    pmu = probe_pmu(not args.no_pmu_probe, affinity_prefix, pinning["cpu"] if pinning["applied"] else None)
    metadata = collect_environment(pinning, work_metadata, pmu, compilers)
    environment = controlled_environment()

    report: dict[str, Any] = {
        "complete": False,
        "schema": SCHEMA,
        "metadata": metadata,
        "settings": {
            "opt": args.opt,
            "common_cflags": args.cflag,
            "reps": 1 if args.skip_perf else args.reps,
            "warmup": 0 if args.skip_perf else args.warmup,
            "skip_perf": args.skip_perf,
            "seed": args.seed,
            "compiler_order_policy": "randomized balanced two-round blocks for two compilers; independent shuffle otherwise",
            "timer": "time.perf_counter_ns wall-clock plus resource.RUSAGE_CHILDREN CPU time",
            "outlier_policy": "MAD outliers counted and reported; none are removed",
        },
        "compilers": [
            {"key": compiler.key, "label": compiler.label,
             "executable": compiler.executable, "flags": list(compiler.flags),
             "binary_sha256": sha256_file(Path(compiler.executable))
             if Path(compiler.executable).is_file() else None}
            for compiler in compilers
        ],
        "unavailable_compilers": unavailable,
        "benchmarks": [],
        "runner_warnings": affinity_warnings,
    }

    def publish_report() -> None:
        if args.json:
            # Reuse the repository's bounded, atomic publication contract.
            # Direct CLI execution does not otherwise import repository tools.
            if str(REPO_ROOT) not in sys.path:
                sys.path.insert(0, str(REPO_ROOT))
            from tools.corpus.publication import atomic_bytes
            args.json.parent.mkdir(parents=True, exist_ok=True)
            atomic_bytes(args.json, (json.dumps(report, indent=2, sort_keys=True) + "\n").encode())

    # Invalidate any old successful receipt before executing a new experiment.
    # Each completed case survives interruption, but cannot certify completion.
    publish_report()
    with tempfile.TemporaryDirectory(prefix="lccc_bench_", dir=str(work_root)) as temporary:
        work = Path(temporary)
        for index, benchmark in enumerate(benchmarks):
            print(f"▶ {benchmark.name}: {benchmark.description}", file=sys.stderr)
            entry = run_benchmark(benchmark, compilers, args, work, artifacts,
                                  affinity_prefix, environment, args.seed + index * 100003)
            report["benchmarks"].append(entry)
            publish_report()

    report["aggregate_lccc_vs_gcc"] = aggregate_lccc_vs_gcc(report["benchmarks"])
    report["duration_sensitivity_lccc_vs_gcc"] = duration_sensitivity(report["benchmarks"])
    report["aggregate_lccc_vs_best_reference"] = aggregate_lccc_vs_best_reference(report["benchmarks"])
    terminal = render_terminal(report)
    print(terminal)
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(render_markdown(report), encoding="utf-8")
    # Completion means all selected cases were attempted, not that their
    # outputs were correct. Preserve failures and the strict nonzero exit.
    report["complete"] = True
    publish_report()

    failures: list[str] = []
    for entry in report["benchmarks"]:
        for compiler in compilers:
            compile_result = entry["compile"].get(compiler.key, {})
            if not compile_result.get("ok"):
                failures.append(f"{entry['name']}: {compiler.key} compile failure")
                continue
            runs = entry.get("runs", {}).get(compiler.key, [])
            if not runs or any(not run.get("ok") for run in runs):
                failures.append(f"{entry['name']}: {compiler.key} runtime failure")
            matches = entry.get("correctness", {}).get(compiler.key, {}).get("matches_baseline")
            if matches is False:
                failures.append(f"{entry['name']}: {compiler.key} output mismatch")
        if args.strict:
            for warning in entry.get("warnings", []):
                if "failure" in warning:
                    failures.append(f"{entry['name']}: {warning}")
    if args.strict and unavailable:
        failures.extend(f"requested compiler unavailable: {key}: {reason}" for key, reason in unavailable.items())
    if failures:
        print("\nBenchmark failures:", file=sys.stderr)
        for failure in sorted(set(failures)):
            print(f"  - {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
