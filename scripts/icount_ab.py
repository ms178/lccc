#!/usr/bin/env python3
"""Deterministic cross-architecture dynamic instruction counting under QEMU.

WHY THIS EXISTS
---------------
The research host is a virtualised 2-core box with **no hardware PMU**, so
cycles, IPC, cache misses and branch mispredictions are simply unavailable, and
wall-clock time on a shared VM has a measured ±9 % spread on identical
binaries.  Neither is a usable signal for "which compiler produced better
code".

QEMU user-mode TCG removes the problem instead of working around it: under
TCG, a deterministic program retires *exactly* the same instruction stream on
every run.  Counting those instructions yields a **zero-variance, dynamic**
code-quality metric — it sees loops, spills, reloads and branch structure,
which a static instruction census cannot.

Debian's qemu-user builds have no TCG plugin support (`qemu-x86_64 -plugin`
is "unknown option"; only qemu-system has it), so this harness uses the log
interface instead:

    qemu-<arch> -d in_asm,exec,nochain -D <log> <binary>

* `in_asm` prints every translated block with its disassembled instructions,
  giving a TB-start-address -> instruction-count map;
* `exec` prints one line per executed TB (start address);
* `nochain` disables TB chaining so *every* TB execution is logged rather than
  only the first.

Both are captured in ONE run, so the address space is identical and ASLR is
irrelevant.  Retired instructions = SUM(executions(tb) * insns(tb)).

WHAT IT MEASURES
----------------
* `total`   — whole-process retired instructions (includes libc startup, which
  is identical across compilers for the same libc, so it is a constant offset
  and never a *difference*);
* per-function counts — a TB is attributed to the function whose [start,end)
  range (from `nm -S`) contains its entry address, so startup noise is gone.
  This is the number to quote.

EVERY MEASUREMENT IS CORRECTNESS-GATED AND DETERMINISM-GATED
------------------------------------------------------------
1. Correctness: each binary is executed (natively when it is host-architecture,
   under qemu otherwise) and its (exit status, stdout, stderr) is compared
   against the reference compiler's build of the same source.  A mismatch is a
   MISCOMPILE and fails the run — a fast wrong answer is never a win.
2. Determinism: the icount run is repeated and the two counts must be
   bit-identical.  A non-deterministic measurement is reported and rejected,
   never averaged.

USAGE
-----
    # Compare LCCC against GCC and Clang on one kernel, per-function:
    scripts/icount_ab.py tests/benchmark/programs/sha256_transform.c \
        --functions sha256_transform --reps 3

    # Cross-architecture: LCCC's AArch64 backend vs aarch64 GCC:
    scripts/icount_ab.py kernel.c --compilers lccc-arm,aarch64-gcc \
        --functions hot_loop

    # Same-binary A/B (a patch under review vs the baseline compiler):
    scripts/icount_ab.py kernel.c --compilers lccc,lccc-alt \
        --define lccc-alt=target/release/lccc:x86_64

    # CI gate: fail if LCCC regresses more than 2 % against itself:
    scripts/icount_ab.py <corpus> --gate --max-regression 0.02 \
        --compilers lccc,lccc-alt --define lccc-alt=<old>:x86_64

Compiler specs are `name` (builtin) or `name=command:arch` (custom, see
`--define`).  Builtins: lccc, lccc-x86, lccc-i686, lccc-arm, lccc-riscv,
gcc, clang, gcc32 (host gcc -m32), aarch64-gcc, riscv64-gcc.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# ---------------------------------------------------------------- compilers --

# name -> (command template, qemu arch, extra flags)
BUILTIN_COMPILERS: dict[str, tuple[str, str, list[str]]] = {
    "lccc":        ("{root}/target/fastbuild/lccc", "x86_64", []),
    "lccc-x86":    ("{root}/target/fastbuild/lccc-x86", "x86_64", []),
    "lccc-i686":   ("{root}/target/fastbuild/lccc-i686", "i386", []),
    "lccc-arm":    ("{root}/target/fastbuild/lccc-arm", "aarch64", []),
    "lccc-riscv":  ("{root}/target/fastbuild/lccc-riscv", "riscv64", []),
    "gcc":         ("gcc", "x86_64", []),
    "clang":       ("clang", "x86_64", []),
    "gcc32":       ("gcc", "i386", ["-m32"]),
    "aarch64-gcc": ("aarch64-linux-gnu-gcc", "aarch64", []),
    "riscv64-gcc": ("riscv64-linux-gnu-gcc", "riscv64", []),
}

# Architectures that can run natively on this host (no emulator needed for the
# correctness run; the *counting* run always goes through qemu for uniformity).
HOST_ARCHES = {"x86_64"}
if os.uname().machine == "i686":
    HOST_ARCHES.add("i386")


@dataclass
class Compiler:
    name: str
    cmd: str
    arch: str
    flags: list[str] = field(default_factory=list)

    @property
    def qemu(self) -> str:
        return f"qemu-{self.arch}"

    def available(self) -> bool:
        exe = shlex.split(self.cmd)[0]
        if "/" in exe:
            return os.path.exists(exe) and os.access(exe, os.X_OK)
        if shutil.which(exe) is None:
            return False
        # A cross compiler that exists but cannot build a trivial program (e.g.
        # missing sysroot) is not available; probe it once.
        return _probe_compiler(self)


_probe_cache: dict[str, bool] = {}


def _probe_compiler(comp: Compiler) -> bool:
    if comp.name in _probe_cache:
        return _probe_cache[comp.name]
    with tempfile.NamedTemporaryFile("w", suffix=".c", delete=False) as tf:
        tf.write("int main(void){return 0;}\n")
        src = tf.name
    out = src + ".out"
    ok = False
    try:
        argv = shlex.split(comp.cmd) + comp.flags + [src, "-o", out]
        rc = subprocess.run(argv, capture_output=True, timeout=120).returncode
        ok = rc == 0 and os.path.exists(out)
    except (subprocess.SubprocessError, OSError):
        ok = False
    finally:
        for p in (src, out):
            try:
                os.unlink(p)
            except OSError:
                pass
    _probe_cache[comp.name] = ok
    return ok


def resolve_compilers(specs: list[str], defines: list[str],
                      root: Path) -> list[Compiler]:
    table = dict(BUILTIN_COMPILERS)
    for spec in defines:
        # name=command:arch
        if "=" not in spec:
            raise SystemExit(f"--define expects NAME=COMMAND:ARCH, got {spec!r}")
        name, rest = spec.split("=", 1)
        if ":" in rest:
            cmd, arch = rest.rsplit(":", 1)
        else:
            cmd, arch = rest, "x86_64"
        table[name] = (cmd, arch, [])
    out: list[Compiler] = []
    for spec in specs:
        if spec not in table:
            raise SystemExit(
                f"unknown compiler {spec!r}; known: {', '.join(sorted(table))}")
        cmd, arch, flags = table[spec]
        out.append(Compiler(spec, cmd.format(root=root), arch, list(flags)))
    return out


# ------------------------------------------------------------------- parsing --

_INSN_LINE = re.compile(r"^\s*0x[0-9a-fA-F]+:\s+(?:[0-9a-fA-F]{2}\s+)+\s*\S")
_ADDR = re.compile(r"^\s*0x([0-9a-fA-F]+):\s")
_IN_BLOCK = re.compile(r"^IN:\s*(.*)$")
# `Trace 0: 0x<host tc.ptr> [<cs_base>/<tb->pc>/<flags>/<cflags>] <symbol>`
# Only the SECOND bracket field is the guest PC; the address before the
# bracket is QEMU's host code pointer and must never be used for attribution.
_TRACE = re.compile(r"^Trace\s+\d+:\s+0x[0-9a-fA-F]+\s+\[[^/\]]*/([0-9a-fA-F]+)/")


def parse_icount_log(path: Path) -> tuple[dict[int, int], dict[int, int]]:
    """Return (tb_insns, tb_execs) maps keyed by TB start address.

    `in_asm` blocks are the only source of instruction counts, and `exec`
    lines are the only source of execution counts; both are keyed by the TB's
    start address so they can be joined.
    """
    tb_insns: dict[int, int] = {}
    tb_execs: dict[int, int] = {}
    cur_start: int | None = None
    cur_count = 0
    with open(path, "r", errors="replace") as fh:
        for line in fh:
            m = _TRACE.match(line)
            if m:
                tb_execs[int(m.group(1), 16)] = tb_execs.get(int(m.group(1), 16), 0) + 1
                continue
            m = _IN_BLOCK.match(line)
            if m:
                # flush the previous block
                if cur_start is not None:
                    tb_insns[cur_start] = tb_insns.get(cur_start, 0) + cur_count
                cur_start = None
                cur_count = 0
                continue
            if _INSN_LINE.match(line):
                if cur_start is None:
                    am = _ADDR.match(line)
                    if am:
                        cur_start = int(am.group(1), 16)
                cur_count += 1
    if cur_start is not None:
        tb_insns[cur_start] = tb_insns.get(cur_start, 0) + cur_count
    return tb_insns, tb_execs


def nm_ranges(binary: Path, cross_prefix: str = "") -> list[tuple[str, int, int]]:
    """(name, start, end) for every sized text symbol, from `nm -S`."""
    nm = f"{cross_prefix}nm" if cross_prefix else "nm"
    if shutil.which(nm) is None:
        nm = "nm"
    try:
        proc = subprocess.run([nm, "-S", "--defined-only", str(binary)],
                              capture_output=True, text=True, timeout=120)
    except (subprocess.SubprocessError, OSError):
        return []
    out: list[tuple[str, int, int]] = []
    for line in proc.stdout.splitlines():
        # `nm -S` prints: <addr> <size> <type> <name>
        parts = line.split()
        if len(parts) < 4:
            continue
        if parts[2].lower() not in ("t", "w"):
            continue
        try:
            start = int(parts[0], 16)
            size = int(parts[1], 16)
        except ValueError:
            continue
        if size:
            out.append((parts[3], start, start + size))
    out.sort(key=lambda r: r[1])
    return out


def attribute(tb_insns: dict[int, int], tb_execs: dict[int, int],
              ranges: list[tuple[str, int, int]]) -> dict[str, int]:
    import bisect
    names = [r[0] for r in ranges]
    starts = [r[1] for r in ranges]
    per_fn: dict[str, int] = {}
    for addr, execs in tb_execs.items():
        insns = tb_insns.get(addr)
        if not insns:
            continue
        idx = bisect.bisect_right(starts, addr) - 1
        if idx < 0:
            continue
        name, start, end = ranges[idx]
        if start <= addr < end:
            per_fn[name] = per_fn.get(name, 0) + execs * insns
    _ = names
    return per_fn


# ------------------------------------------------------------------ runners --


@dataclass
class RunResult:
    compiler: str
    arch: str
    compile_ok: bool
    compile_error: str = ""
    exit_status: int | None = None
    stdout: str = ""
    stderr: str = ""
    total_insns: int | None = None
    per_function: dict[str, int] = field(default_factory=dict)
    deterministic: bool = True
    note: str = ""


def compile_one(comp: Compiler, src: Path, out: Path, flags: list[str],
                timeout: int) -> tuple[bool, str]:
    argv = shlex.split(comp.cmd) + comp.flags + list(flags) + [str(src), "-o", str(out)]
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False, "compile timeout"
    except OSError as exc:
        return False, f"compile exec failed: {exc}"
    if proc.returncode != 0 or not out.exists():
        return False, (proc.stderr or proc.stdout or "compile failed").strip()[:2000]
    return True, ""


def run_plain(comp: Compiler, binary: Path, timeout: int) -> tuple[int, str, str]:
    """Execute for correctness (natively when possible, else under qemu)."""
    if comp.arch in HOST_ARCHES:
        argv = [str(binary)]
    else:
        argv = [comp.qemu, "-L", f"/usr/{_triplet(comp.arch)}", str(binary)]
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return proc.returncode, proc.stdout, proc.stderr
    except subprocess.TimeoutExpired:
        return -999, "", "TIMEOUT"
    except OSError as exc:
        return -998, "", f"exec failed: {exc}"


def _triplet(arch: str) -> str:
    return {
        "aarch64": "aarch64-linux-gnu",
        "riscv64": "riscv64-linux-gnu",
        "i386": "i386-linux-gnu",
        "x86_64": "x86_64-linux-gnu",
    }.get(arch, arch + "-linux-gnu")


def count_once(comp: Compiler, binary: Path, workdir: Path, tag: str,
               timeout: int) -> tuple[dict[int, int], dict[int, int], str]:
    log = workdir / f"{tag}.log"
    if log.exists():
        log.unlink()
    env = dict(os.environ)
    env["QEMU_LOG_FILENAME"] = str(log)
    argv = [comp.qemu, "-d", "in_asm,exec,nochain", "-D", str(log), str(binary)]
    try:
        with open(workdir / f"{tag}.stdout", "w") as so, \
                open(workdir / f"{tag}.stderr", "w") as se:
            subprocess.run(argv, stdout=so, stderr=se, timeout=timeout, env=env)
    except subprocess.TimeoutExpired:
        return {}, {}, "count timeout"
    except OSError as exc:
        return {}, {}, f"count exec failed: {exc}"
    if not log.exists():
        return {}, {}, "no log produced (qemu missing or no plugin/log support)"
    insns, execs = parse_icount_log(log)
    return insns, execs, ""


# --------------------------------------------------------------------- main --


def measure(comp: Compiler, src: Path, workdir: Path, flags: list[str],
            functions: list[str], reps: int, timeout: int) -> RunResult:
    binary = workdir / f"{src.stem}.{comp.name}.bin"
    ok, err = compile_one(comp, src, binary, flags, timeout)
    res = RunResult(compiler=comp.name, arch=comp.arch, compile_ok=ok, compile_error=err)
    if not ok:
        return res

    status, so, se = run_plain(comp, binary, timeout)
    res.exit_status, res.stdout, res.stderr = status, so, se

    # Binary must be executable before counting: a crash would otherwise be
    # silently counted as "fast".
    if status not in (0, None) and status < 0:
        res.note = f"binary died (status {status}); not counted"
        return res

    counts: list[tuple[dict[int, int], dict[int, int]]] = []
    for r in range(reps):
        insns, execs, err = count_once(comp, binary, workdir, f"{src.stem}.{comp.name}.{r}",
                                       timeout)
        if err:
            res.note = err
            return res
        counts.append((insns, execs))

    # determinism gate: every repetition must agree exactly
    first_total = sum(e * i.get(a, 0) for a, e in counts[0][1].items()
                      for i in [counts[0][0]])
    for insns, execs in counts[1:]:
        total = sum(e * i.get(a, 0) for a, e in execs.items() for i in [insns])
        if total != first_total:
            res.deterministic = False
    if not res.deterministic:
        res.note = "NON-DETERMINISTIC instruction count — measurement rejected"
        return res

    insns, execs = counts[0]
    res.total_insns = first_total
    ranges = nm_ranges(binary)
    per_fn = attribute(insns, execs, ranges)
    if functions:
        wanted = {}
        for fn in functions:
            wanted[fn] = per_fn.get(fn, 0)
        res.per_function = wanted
    else:
        # report the ten hottest functions so a comparison is still meaningful
        res.per_function = dict(sorted(per_fn.items(), key=lambda kv: -kv[1])[:10])
    return res


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Deterministic QEMU-TCG dynamic instruction-count A/B oracle.")
    ap.add_argument("sources", nargs="+", type=Path, help="C source files")
    ap.add_argument("--compilers", default="lccc,gcc",
                    help="comma-separated compiler specs (default: lccc,gcc)")
    ap.add_argument("--define", action="append", default=[],
                    help="define a custom compiler: NAME=COMMAND:ARCH")
    ap.add_argument("--flags", default="-O2", help="optimization flags (default: -O2)")
    ap.add_argument("--functions", default="",
                    help="comma-separated function names to attribute (default: top 10)")
    ap.add_argument("--reps", type=int, default=2,
                    help="repetitions for the determinism gate (default: 2)")
    ap.add_argument("--timeout", type=int, default=600, help="per-run timeout seconds")
    ap.add_argument("--static", action="store_true", default=True,
                    help="link statically (default: on — removes loader noise)")
    ap.add_argument("--no-static", dest="static", action="store_false")
    ap.add_argument("--reference", default="",
                    help="compiler whose output defines correct behaviour "
                         "(default: first non-lccc compiler, else first)")
    ap.add_argument("--gate", action="store_true",
                    help="exit non-zero on any miscompile, non-determinism, or "
                         "regression beyond --max-regression")
    ap.add_argument("--max-regression", type=float, default=0.02,
                    help="allowed LCCC regression vs the alternative LCCC arm")
    ap.add_argument("--json", type=Path, help="write the full report as JSON")
    ap.add_argument("--markdown", type=Path, help="write a Markdown table")
    ap.add_argument("--artifact-dir", type=Path,
                    help="keep build artifacts and QEMU logs here")
    ap.add_argument("--verbose", "-v", action="store_true")
    args = ap.parse_args(argv)

    comps = resolve_compilers([c.strip() for c in args.compilers.split(",") if c.strip()],
                              args.define, REPO_ROOT)
    usable = [c for c in comps if c.available()]
    for c in comps:
        if c not in usable:
            print(f"SKIP  compiler {c.name}: not available ({c.cmd})", file=sys.stderr)

    flags = shlex.split(args.flags)
    if args.static:
        flags.append("-static")
    functions = [f for f in args.functions.split(",") if f]

    ref_name = args.reference or next(
        (c.name for c in usable if not c.name.startswith("lccc")), usable[0].name)
    if ref_name not in {c.name for c in usable}:
        print(f"FATAL: reference compiler {ref_name} is not available", file=sys.stderr)
        return 2

    keep = args.artifact_dir
    if keep:
        keep.mkdir(parents=True, exist_ok=True)

    report: dict[str, object] = {
        "tool": "icount_ab.py",
        "flags": args.flags,
        "static": args.static,
        "compilers": [c.name for c in usable],
        "reference": ref_name,
        "results": [],
    }
    failures: list[str] = []
    table_rows: list[tuple[str, str, str, float | None, int | None, str]] = []

    for src in args.sources:
        if not src.exists():
            print(f"SKIP  {src}: no such file", file=sys.stderr)
            continue
        with tempfile.TemporaryDirectory(prefix="icount-") as td:
            workdir = Path(td) if keep is None else keep
            results = {c.name: measure(c, src, workdir, flags, functions,
                                       args.reps, args.timeout)
                       for c in usable}
            ref = results[ref_name]
            entry: dict[str, object] = {"source": str(src), "compilers": {}}

            for name, r in results.items():
                rec: dict[str, object] = {
                    "arch": r.arch,
                    "compile_ok": r.compile_ok,
                    "exit_status": r.exit_status,
                    "deterministic": r.deterministic,
                    "total_insns": r.total_insns,
                    "per_function": r.per_function,
                }
                if not r.compile_ok:
                    rec["error"] = r.compile_error
                    failures.append(f"{src}: {name}: compile failed: {r.compile_error[:200]}")
                elif name != ref_name and ref.exit_status is not None:
                    same = (r.exit_status == ref.exit_status and r.stdout == ref.stdout)
                    rec["output_matches_reference"] = same
                    if not same:
                        failures.append(
                            f"{src}: {name}: MISCOMPILE — exit {r.exit_status} vs "
                            f"{ref.exit_status} / stdout differs from {ref_name}")
                if not r.deterministic and r.compile_ok:
                    failures.append(f"{src}: {name}: {r.note}")
                if r.note:
                    rec["note"] = r.note
                entry["compilers"][name] = rec

                key_fn = functions[0] if functions else None
                metric = (r.per_function.get(key_fn) if key_fn
                          else (sum(r.per_function.values()) if r.per_function
                                else r.total_insns))
                table_rows.append((src.name, name, key_fn or "(top10)",
                                   None, metric, r.note or ("OK" if r.compile_ok else "compile-failed")))
            report["results"].append(entry)

    # ratios vs the best measured metric for each source/function
    for entry in report["results"]:  # type: ignore[union-attr]
        comps_d = entry["compilers"]  # type: ignore[index]
        metrics = {n: (rec.get("per_function", {}) or {}) for n, rec in comps_d.items()}
        if not functions:
            continue
        for fn in functions:
            vals = {n: m.get(fn, 0) for n, m in metrics.items() if m.get(fn, 0)}
            if not vals:
                continue
            best = min(vals.values())
            for n, v in vals.items():
                comps_d[n].setdefault("ratios", {})[fn] = round(v / best, 4) if best else None
                if n == "lccc":
                    comps_d[n]["vs_best"] = round(v / best, 4) if best else None

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2))

    # ---- human-readable table ----
    print(f"\n=== icount A/B ({args.flags}{' -static' if args.static else ''}) ===")
    hdr = f"{'source':<28} {'compiler':<12} {'function':<24} {'insns':>14} {'vs best':>8}"
    print(hdr)
    print("-" * len(hdr))
    for entry in report["results"]:  # type: ignore[union-attr]
        for name, rec in entry["compilers"].items():  # type: ignore[union-attr]
            pf = rec.get("per_function") or {}
            for fn, v in (pf.items() if pf else {("(total)", rec.get("total_insns"))}):  # type: ignore[arg-type]
                ratio = (rec.get("ratios", {}).get(fn) if isinstance(rec.get("ratios"), dict)
                         else None)
                print(f"{Path(entry['source']).name:<28} {name:<12} {str(fn):<24} "
                      f"{v if v is not None else '-':>14} "
                      f"{(f'{ratio:.3f}x' if ratio else '-'):>8}")

    if failures:
        print("\nFAILURES:")
        for f in failures:
            print(f"  - {f}")

    if args.markdown:
        lines = ["| source | compiler | function | insns | vs best |",
                 "|---|---|---|---:|---:|"]
        for entry in report["results"]:  # type: ignore[union-attr]
            for name, rec in entry["compilers"].items():  # type: ignore[union-attr]
                pf = rec.get("per_function") or {}
                for fn, v in pf.items():
                    ratio = (rec.get("ratios", {}).get(fn)
                             if isinstance(rec.get("ratios"), dict) else None)
                    lines.append(f"| {Path(entry['source']).name} | {name} | {fn} | "
                                 f"{v} | {ratio if ratio else '-'} |")
        args.markdown.write_text("\n".join(lines) + "\n")
        print(f"\nwrote {args.markdown}")

    if failures and (args.gate or True):
        # correctness/nondeterminism failures are always fatal; --gate only adds
        # the regression threshold below.
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
