#!/usr/bin/env python3
"""Per-function allocator/code-shape scoreboard for the RA corpus (RAP-01).

This deliberately reports proxies rather than declaring a winner: stack
operands include legitimate frame objects and code size is not runtime.  It
compiles each translation unit to assembly, records the compiler invocation
and elapsed wall time, and emits stable JSON/CSV suitable for revision-to-
revision comparison. Timing values are informational and must not be gated on a
shared VM. Use repeated, order-balanced executable runs on target hardware for
performance conclusions.

Examples:
  python3 tests/ra/spill_compare.py --lccc target/fastbuild/lccc --gcc gcc
  python3 tests/ra/spill_compare.py --files a.c b.c --json result.json
  python3 tests/ra/spill_compare.py --check --opt-levels 1,2,3
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import platform
import re
import shlex
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GENERATOR = ROOT / "tests/ra/gen_rastress.py"
NAME_RE = r"[A-Za-z_$][\w.$@]*"
TYPE_RE = re.compile(rf"^\s*\.type\s+({NAME_RE})\s*,\s*@function\b")
SIZE_RE = re.compile(rf"^\s*\.size\s+({NAME_RE})\s*,")
LABEL_RE = re.compile(rf"^({NAME_RE}|\.{NAME_RE}):")
STACK_RE = re.compile(r"\(%(?:rsp|rbp|esp|ebp)(?:\s*,|\))")
INSN_RE = re.compile(r"^\s*([A-Za-z][A-Za-z0-9_.]*)\s*(.*?)\s*$")
INSTRUCTION_PREFIXES = {"lock", "rep", "repe", "repz", "repne", "repnz", "bnd", "notrack", "xacquire", "xrelease", "addr16", "addr32", "data16", "data32"}


def assembly_functions(asm: str) -> dict[str, list[str]]:
    functions: dict[str, list[str]] = {}
    pending: str | None = None
    current: str | None = None
    section = ".text"
    previous_section: tuple[str, str | None] | None = None
    section_stack: list[tuple[str, str | None]] = []
    in_text = True
    for raw in asm.splitlines():
        line = raw.split("#", 1)[0].rstrip()
        text = line.strip()
        if text.startswith((".section", ".pushsection")):
            parts = text.split(None, 1)
            target = parts[1].split(",", 1)[0].strip('"') if len(parts) > 1 else ""
            if text.startswith(".pushsection"):
                section_stack.append((section, current))
            else:
                previous_section = (section, current)
            section = target
            in_text = section.startswith(".text")
            if not in_text or not current or not section.startswith(".text"):
                current = None
            continue
        if text == ".popsection":
            if section_stack:
                section, current = section_stack.pop()
                in_text = section.startswith(".text")
            continue
        if text == ".previous":
            if previous_section is not None:
                old_section, old_current = section, current
                section, current = previous_section
                previous_section = (old_section, old_current)
                in_text = section.startswith(".text")
            continue
        if text in {".data", ".bss", ".rodata", ".rdata", ".tdata", ".tbss", ".text"}:
            previous_section = (section, current)
            section = text
            in_text = section == ".text"
            if not in_text:
                current = None
            continue
        if match := TYPE_RE.match(line):
            pending = match.group(1)
            continue
        if match := SIZE_RE.match(line):
            if current == match.group(1):
                current = None
            continue
        if match := LABEL_RE.match(line):
            label = match.group(1)
            if in_text and (label == pending or (pending is None and not label.startswith(".L"))):
                current = label
                functions.setdefault(label, [])
                if label == pending:
                    pending = None
            elif current is not None and in_text:
                # Preserve local labels so the loop-temporary push/pop census
                # can recognize direct backward branches.
                functions[current].append(line)
            continue
        if current is not None and in_text:
            functions[current].append(line)
    return functions


def split_operands(text: str) -> list[str]:
    """Split AT&T operands at top-level commas (not commas inside SIB tuples)."""
    result: list[str] = []
    depth = 0
    start = 0
    for idx, char in enumerate(text):
        if char == "(":
            depth += 1
        elif char == ")":
            depth = max(0, depth - 1)
        elif char == "," and depth == 0:
            result.append(text[start:idx].strip())
            start = idx + 1
    tail = text[start:].strip()
    if tail or result:
        result.append(tail)
    return result


def is_memory_operand(operand: str) -> bool:
    # x87 register operands are spelled `%st(0)` and are not memory accesses.
    return "(" in operand and not re.fullmatch(r"%st\(\d+\)", operand)


def instruction_counts(lines: list[str]) -> dict[str, int]:
    counts = {k: 0 for k in ("instructions", "stack_operands", "loads", "stores", "push", "pop", "calls", "loop_push_pop")}
    labels: dict[str, int] = {}
    parsed: list[tuple[str, str] | None] = []
    for idx, line in enumerate(lines):
        label = LABEL_RE.match(line)
        if label:
            labels[label.group(1)] = idx
            parsed.append(None)
            continue
        match = INSN_RE.match(line)
        if not match:
            parsed.append(None)
            continue
        mnemonic, operands = match.group(1).lower(), match.group(2)
        while mnemonic in INSTRUCTION_PREFIXES and operands:
            prefixed_instruction = INSN_RE.match(operands)
            if prefixed_instruction is None:
                break
            mnemonic, operands = prefixed_instruction.group(1).lower(), prefixed_instruction.group(2)
        parsed.append((mnemonic, operands))
        if mnemonic.startswith((".", "#")):
            continue
        counts["instructions"] += 1
        if not mnemonic.startswith(("push", "pop", "call", "lea")) and STACK_RE.search(operands):
            counts["stack_operands"] += 1
        args = split_operands(operands)
        explicit_memory = [arg for arg in args if is_memory_operand(arg)]
        if explicit_memory and not mnemonic.startswith(("lea", "call", "jmp")):
            destination_is_memory = bool(args and is_memory_operand(args[-1]))
            if mnemonic.startswith("pop") and destination_is_memory:
                counts["stores"] += 1
            elif mnemonic.startswith(("mov", "vmov")):
                # AT&T source,destination order; the SIB-aware splitter keeps
                # `(%base,%index,scale)` in a single operand.
                counts["stores" if destination_is_memory else "loads"] += 1
            elif mnemonic.startswith("set") and destination_is_memory:
                counts["stores"] += 1
            elif mnemonic.startswith(("fst", "fist", "st")) and destination_is_memory:
                counts["stores"] += 1
            else:
                # Most x86 arithmetic memory operands are reads. Instructions
                # with a memory destination in this set are read-modify-write.
                counts["loads"] += 1
                if destination_is_memory and mnemonic.startswith((
                    "add", "sub", "adc", "sbb", "and", "or", "xor", "inc", "dec",
                    "neg", "not", "shl", "sal", "shr", "sar", "rol", "ror",
                    "rcl", "rcr", "bts", "btr", "btc", "xchg", "xadd", "cmpxchg",
                )):
                    counts["stores"] += 1
        if mnemonic.startswith("push"):
            counts["push"] += 1
        if mnemonic.startswith("pop"):
            counts["pop"] += 1
        if mnemonic.startswith("call"):
            counts["calls"] += 1
    # Lexical back-edge interval is a conservative screening proxy for loop
    # push/pop temporaries. It is not a CFG proof for irreducible/multi-exit
    # loops; the report labels this as a census, never a correctness verdict.
    loop_positions: set[int] = set()
    for idx, item in enumerate(parsed):
        if item is None:
            continue
        mnemonic, operands = item
        if mnemonic.startswith("j") or mnemonic.startswith("loop"):
            target = operands.split(",")[-1].strip().lstrip("*")
            target = target.split()[0] if target else ""
            start = labels.get(target)
            if start is not None and start < idx:
                loop_positions.update(range(start, idx + 1))
    counts["loop_push_pop"] = sum(
        1 for idx in loop_positions if parsed[idx] is not None and parsed[idx][0].startswith(("push", "pop"))
    )
    return counts


def compile_assembly(cc: str, src: Path, opt: str, extra: list[str]) -> tuple[dict[str, list[str]], float]:
    with tempfile.TemporaryDirectory(prefix="lccc-ra-asm-") as td:
        out = Path(td) / "unit.s"
        cmd = [*shlex.split(cc), f"-O{opt}", "-S", *extra, str(src), "-o", str(out)]
        start = time.perf_counter()
        proc = subprocess.run(cmd, text=True, capture_output=True, timeout=180)
        elapsed = time.perf_counter() - start
        if proc.returncode:
            raise RuntimeError(f"compile failed ({' '.join(cmd)}):\n{proc.stderr[-4000:]}")
        functions = assembly_functions(out.read_text(errors="replace"))
        if not functions:
            raise RuntimeError(f"no function bodies recognized in {src} from {cc} -O{opt}")
        return functions, elapsed


def check_correctness(cc: str, src: Path, opt: str, extra: list[str], reference: str | None = None) -> str:
    with tempfile.TemporaryDirectory(prefix="lccc-ra-run-") as td:
        exe = Path(td) / "kernel"
        cmd = [*shlex.split(cc), f"-O{opt}", *extra, str(src), "-o", str(exe)]
        proc = subprocess.run(cmd, text=True, capture_output=True, timeout=180)
        if proc.returncode:
            raise RuntimeError(
                f"link failed ({' '.join(cmd)}); --check requires a standalone executable translation unit or a source-specific runner:\n"
                f"{proc.stderr[-4000:]}"
            )
        run = subprocess.run([str(exe)], text=True, capture_output=True, timeout=20)
        if run.returncode:
            raise RuntimeError(f"program failed ({cc} -O{opt}): {run.stderr[-1000:]}")
        output = run.stdout.strip()
        if reference is not None and output != reference:
            raise RuntimeError(f"checksum mismatch ({cc} -O{opt}): expected {reference!r}, got {output!r}")
        return output


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lccc", default=os.environ.get("LCCC", str(ROOT / "target/fastbuild/lccc")))
    p.add_argument("--gcc", default=os.environ.get("GCC", "gcc"))
    p.add_argument("--clang", default=os.environ.get("CLANG"), help="optional Clang path")
    p.add_argument("--files", nargs="+", help="explicit C files instead of a named suite")
    p.add_argument("--suite", choices=("holdout", "tuning", "both"), default="holdout",
                   help="corpus to measure; holdout is never used to tune heuristics")
    p.add_argument("--opt-levels", default="2", help="comma-separated levels, e.g. 1,2,3")
    p.add_argument("--extra-flag", action="append", default=[], help="pass through to all compilers")
    p.add_argument("--check", action="store_true", help="run checksum comparison against GCC -O0")
    p.add_argument("--compile-budget-seconds", type=float, help="optional per-translation-unit compile-time ratchet (host-specific)")
    p.add_argument("--json", type=Path, help="write machine-readable report")
    p.add_argument("--csv", type=Path, help="write flat CSV report")
    p.add_argument("--text", action="store_true", help="print tabular report (default)")
    args = p.parse_args()
    levels = [x.strip().removeprefix("O") for x in args.opt_levels.split(",") if x.strip()]
    if not levels or any(x not in {"0", "1", "2", "3", "s", "fast"} for x in levels):
        p.error("--opt-levels must contain O0/O1/O2/O3/Os/Ofast levels")
    if args.compile_budget_seconds is not None and args.compile_budget_seconds <= 0:
        p.error("--compile-budget-seconds must be positive")

    tmp = tempfile.TemporaryDirectory(prefix="lccc-ra-corpus-")
    try:
        source_suites: list[tuple[str, Path]]
        if args.files:
            source_suites = [("explicit", Path(f).resolve()) for f in args.files]
        else:
            source_suites = []
            if args.suite in {"holdout", "both"}:
                generated_dir = Path(tmp.name) / "holdout"
                subprocess.run([sys.executable, str(GENERATOR), "--output-dir", str(generated_dir)], check=True)
                source_suites.extend(("holdout", f) for f in sorted(generated_dir.glob("*.c")))
            if args.suite in {"tuning", "both"}:
                for directory in (ROOT / "tests/benchmark/programs", ROOT / "tests/benchmark/kernel_corpus"):
                    source_suites.extend(("tuning", f) for f in sorted(directory.glob("*.c")))
        compilers = [("lccc", args.lccc), ("gcc", args.gcc)]
        if args.clang:
            compilers.append(("clang", args.clang))
        compiler_versions: dict[str, str] = {}
        for name, cc in compilers:
            try:
                version = subprocess.run([*shlex.split(cc), "--version"], text=True, capture_output=True, timeout=10)
                compiler_versions[name] = (version.stdout or version.stderr).splitlines()[0].strip() if version.returncode == 0 else "unavailable"
            except (OSError, subprocess.TimeoutExpired):
                compiler_versions[name] = "unavailable"
        rows: list[dict[str, object]] = []
        for suite, src in source_suites:
            baseline = None
            if args.check:
                baseline = check_correctness(args.gcc, src, "0", args.extra_flag)
            for name, cc in compilers:
                for opt in levels:
                    funcs, elapsed = compile_assembly(cc, src, opt, args.extra_flag)
                    if args.compile_budget_seconds is not None and elapsed > args.compile_budget_seconds:
                        raise RuntimeError(f"compile budget exceeded: {src.stem} {name} -O{opt}: {elapsed:.3f}s > {args.compile_budget_seconds:.3f}s")
                    if args.check:
                        got = check_correctness(cc, src, opt, args.extra_flag, baseline)
                    else:
                        got = "not-checked"
                    for fn, body in sorted(funcs.items()):
                        counts = instruction_counts(body)
                        rows.append({"suite": suite, "kernel": src.stem, "source": src.name,
                                     "source_sha256": hashlib.sha256(src.read_bytes()).hexdigest(),
                                     "function": fn, "compiler": name, "compiler_command": cc,
                                     "opt": f"O{opt}", "compile_wall_seconds": round(elapsed, 6),
                                     "checksum": got, **counts})
        fields = ["suite", "kernel", "source", "source_sha256", "function", "compiler", "compiler_command", "opt", "compile_wall_seconds", "checksum",
                  "instructions", "stack_operands", "loads", "stores", "push", "pop", "calls", "loop_push_pop"]
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps({"schema": 1, "suite": args.suite if not args.files else "explicit",
                                             "opt_levels": [f"O{x}" for x in levels],
                                             "compiler_versions": compiler_versions,
                                             "extra_flags": args.extra_flag,
                                             "host_platform": platform.platform(),
                                             "generator_sha256": hashlib.sha256(GENERATOR.read_bytes()).hexdigest(),
                                             "notes": "Assembly proxies; timing is informational, not hardware evidence.",
                                             "rows": rows}, indent=2) + "\n", encoding="utf-8")
        if args.csv:
            args.csv.parent.mkdir(parents=True, exist_ok=True)
            with args.csv.open("w", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=fields)
                writer.writeheader(); writer.writerows(rows)
        if args.text or not args.json and not args.csv:
            print("suite\tsource\tcompiler\topt\tfunction\tinsns\tstack\tloads\tstores\tpush/pop\tloop-push/pop\tcompile_s\tchecksum")
            for row in rows:
                print("\t".join(str(row[k]) for k in ("suite", "source", "compiler", "opt", "function", "instructions", "stack_operands", "loads", "stores"))
                      + f"\t{row['push']}/{row['pop']}\t{row['loop_push_pop']}\t{row['compile_wall_seconds']}\t{row['checksum']}")
        return 0
    except (RuntimeError, OSError, subprocess.SubprocessError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    finally:
        tmp.cleanup()

if __name__ == "__main__":
    raise SystemExit(main())
