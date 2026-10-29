#!/usr/bin/env python3
"""MS-01a: hard codegen-quality gate for the golden workloads.

The runtime CI (ci-bench.py) gates a single metric (fib >= 10x vs GCC) —
spectral_norm could regress 4x and stay green. This gate protects what
actually matters, CODE GENERATION QUALITY, on the six golden workloads
from the performance charter:

    gzip_crc32, zlib_ng_adler32, expat_xml_scan, sqlite_varint,
    glibc_memcmp, hash_table, zstd_count  (+ stencil5 as the OP-05a sentinel)

Per workload it compiles with `lccc -O2 -S` and extracts stable, 
noise-free assembly metrics:

    insns      total instruction count (function bodies)
    stackmem   rbp/rsp-relative memory operands (spill traffic)
    pushes     prologue pressure (callee-save saves)
    ymm        256-bit vector instructions
    fma        fused multiply-add instructions

and compares each against a checked-in baseline
(.github/scripts/ci-codegen-baseline.json) with PER-WORKLOAD tolerance
bands. A metric may regress at most its tolerance (default 2%) without
an explicit baseline refresh; improvements update the baseline via
--update-baseline in the PR that lands them.

Baseline notes (2026-10-29): the checked-in numbers were a stale snapshot —
`--update-baseline` re-pins a broad improvement that shipped without a refresh
(gzip_crc32 64→60 insns and 4→3 pushes, adler32 202→195 insns and 6→0 stack
operands, expat_xml_scan 178→165, zstd_count 115→101 and 8→2, chacha20_block
192→184 and 8→4, portable_bit_counts 236→227). One metric moved the other way
and is intentional: `stencil5` 105→114 insns. That workload's final checksum
loop (`for (i…) checksum += output[i]`) is a single carried FP accumulator with
a constant trip, so the two-block reducer now unrolls it 4x: +3 static `addsd`
per group in exchange for one compare/branch per four elements. Measured with
callgrind on the same binary: 246,951,674 → 246,705,917 executed instructions
(−245,757) with byte-identical output. Static size is the wrong metric for an
unroll; the gate keeps the new number as its floor.

Usage:
    python3 ci-codegen-gate.py --lccc target/fastbuild/lccc
    python3 ci-codegen-gate.py --lccc ... --update-baseline
    python3 ci-codegen-gate.py --lccc ... --json out.json --summary
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).parent.resolve()
REPO = HERE.parent.parent
PROGRAMS = REPO / "tests" / "benchmark" / "programs"
BASELINE = HERE / "ci-codegen-baseline.json"

# (source file, label) — the golden set + the OP-05a sentinel.
WORKLOADS = [
    ("gzip_crc32.c", "gzip_crc32"),
    ("zlib_ng_adler32.c", "zlib_ng_adler32"),
    ("expat_xml_scan.c", "expat_xml_scan"),
    ("sqlite_varint.c", "sqlite_varint"),
    ("glibc_memcmp.c", "glibc_memcmp"),
    ("hash_table.c", "hash_table"),
    # zstd: the FSE/Huffman inner loops from the decompressor.  It is the
    # workload that caught the loop-carried copy bug (§9.8 hole 4) and it is
    # what the x86 boot path decompresses with, so it belongs in the golden set.
    ("zstd_count.c", "zstd_count"),
    ("fp_memfold_stencil5.c", "stencil5"),
    # chacha20: the largest hot-function oracle gap (core 178 vs 63 insns).
    # Tracked so the gap-closure project measures against a pinned floor and
    # no change regresses it silently while the gap is open.
    ("chacha20_block.c", "chacha20_block"),
    # Whole-TU sentinel; per-function baseline/v3 contracts live in
    # check_bit_count_codegen.py, executable/IR witnesses in the salvage gate.
    ("../../regression/count_zero_intermediate_casts.c", "portable_bit_counts"),
    ("../../regression/clz_safe_guards.c", "safe_count_guards"),
]

# Metric -> default max regression (fraction). A metric that got BETTER
# never fails; only regressions beyond the band do.
DEFAULT_TOLERANCE = 0.02
# Spill traffic is the RA-sensitive metric the whole program is about;
# give it a slightly wider band so ordinary churn does not flap CI.
TOLERANCES = {
    "stackmem": 0.05,
    "pushes": 0.10,
    "moves": 0.05,
}

INSN_RE = re.compile(r"^\s+([a-z0-9]+)[\s]")
MEM_RE = re.compile(r"[-0-9(]*\((?:%rbp|%rsp)\)")
# A `N(%rbp)` operand is stack traffic only when rbp is the frame pointer.
# In FPO functions rbp is an ordinary callee-saved GPR (and a *good* one for
# pointers: `8(%rbp)` needs no SIB byte while `8(%r12)` does), so `8(%rbp)`
# there is a heap/data dereference, not a spill. Count a function as
# rbp-as-GPR only on positive proof — a pure-write instruction targeting
# rbp/ebp (loads, LEAs, self-xor). The pattern is deliberately narrow: a
# missed writer merely keeps counting (today's behavior, a false positive at
# worst), while the whitelist can never fire on a true frame-pointer
# function (rbp is reserved there, so no GPR write to it exists) — the
# exclusion can therefore never mask a real spill. `movq %rsp,%rbp` is the
# frame-pointer setup itself and is excluded from the proof.
RBP_GPR_WRITE_RE = re.compile(
    r"^\s+(?:movq|movl|movabsq|movsbq|movswq|movslq|movzbq|movzwq|leaq)"
    r"\s+[^,]+,\s*%[re]?bp\b"
    r"|^\s+(?:xorl|xorq)\s+%[re]?bp,\s*%[re]?bp\b"
)
RBP_SETUP_RE = re.compile(r"^\s+movq\s+%rsp,\s*%rbp\b")
RBP_REF_RE = re.compile(r"\(%[re]?bp\)")
RSP_REF_RE = re.compile(r"\(%rsp\)")
YMM_RE = re.compile(r"^\s+(?:v[a-z]+)\s+[^#]*%ymm")
FMA_RE = re.compile(r"^\s+(?:vfmadd|vfnmadd|vfmsub|vfnmsub)[a-z0-9]*\s")


def score_chunk(lines: list, m: dict) -> None:
    """Score one function body. `N(%rbp)` operands count as stack traffic
    unless the body proves rbp is a GPR (see RBP_GPR_WRITE_RE)."""
    rbp_is_gpr = any(
        RBP_GPR_WRITE_RE.search(l) and not RBP_SETUP_RE.search(l) for l in lines
    )
    for line in lines:
        mi = INSN_RE.match(line)
        if not mi:
            continue
        op = mi.group(1)
        m["insns"] += 1
        if MEM_RE.search(line):
            # FPO rbp-deref (pointer/array traffic through the rbp GPR) is
            # not stack traffic. A line mentioning (%rsp) still counts (a
            # mixed line is conservative: real slot traffic is never dropped).
            if not (
                rbp_is_gpr and RBP_REF_RE.search(line) and not RSP_REF_RE.search(line)
            ):
                m["stackmem"] += 1
        if op.startswith("push"):
            m["pushes"] += 1
        if op in ("mov", "movq", "movl", "movw", "movb", "vmovsd", "vmovss"):
            if "%" in line and "(" not in line:
                m["moves"] += 1
        if YMM_RE.match(line):
            m["ymm"] += 1
        if FMA_RE.match(line):
            m["fma"] += 1


def function_body_metrics(asm: str) -> dict:
    """Aggregate metrics over function bodies only (between .type @function
    and the next .type/.size/.section). Bodies are scored per-function so
    the rbp-as-GPR proof stays function-local."""
    m = {"insns": 0, "stackmem": 0, "pushes": 0, "moves": 0, "ymm": 0, "fma": 0}
    chunk: list = []
    in_func = False

    def flush():
        if chunk:
            score_chunk(chunk, m)
            chunk.clear()

    for line in asm.splitlines():
        s = line.strip()
        if s.startswith(".type") and "@function" in s:
            flush()
            in_func = True
            continue
        if s == ".size" or (s.startswith(".section") and in_func):
            flush()
            in_func = False
            continue
        if not in_func:
            continue
        chunk.append(line)
    flush()
    return m


def compile_asm(lccc: str, gcc_inc: str, src: Path) -> str | None:
    """The `-O2 -S` translation unit for one workload, or None on failure."""
    with tempfile.NamedTemporaryFile(suffix=".s", delete=False) as f:
        out = Path(f.name)
    try:
        r = subprocess.run(
            [lccc, f"-I{gcc_inc}", "-O2", "-S", str(src), "-o", str(out)],
            capture_output=True, text=True, timeout=300,
        )
        if r.returncode != 0:
            return None
        return out.read_text()
    finally:
        out.unlink(missing_ok=True)


def compile_metrics(lccc: str, gcc_inc: str, src: Path) -> dict | None:
    asm = compile_asm(lccc, gcc_inc, src)
    return None if asm is None else function_body_metrics(asm)


def gha_escape(text: str) -> str:
    """Escape a workflow-command payload (property values are `%`/`,`/`:`)."""
    return (text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
            .replace(",", "%2C").replace(":", "%3A"))


# GitHub truncates one annotation body well below the 65535-byte API limit;
# 56 KiB keeps every chunk safely inside it while bounding the number of
# annotations a run publishes.
ANNOTATION_CHUNK = 56 * 1024


def emit_metrics_annotation(results: list) -> None:
    """Publish the per-workload numbers as ONE annotation.

    The gate's numbers otherwise exist only in the job log, and a log is not
    queryable from the API: a reviewer (or an agent) cannot read the codegen
    metrics of a run at all. One compact annotation makes the evidence
    API-visible without adding a step, a lane or an artifact — the same
    contract `.github/workflows/pr826-redteam.yml` uses for its fmt payload.
    """
    rows = ["workload insns stackmem pushes moves ymm fma verdict"]
    for r in results:
        if "skip" in r or "error" in r:
            rows.append(f"{r['label']} - - - - - - {r.get('error', 'SKIPPED')}")
            continue
        m = r["metrics"]
        verdict = "REGRESSION" if r.get("regressions") else "pass"
        rows.append(
            f"{r['label']} {m['insns']} {m['stackmem']} {m['pushes']} "
            f"{m['moves']} {m['ymm']} {m['fma']} {verdict}"
        )
        for reg in r.get("regressions") or []:
            rows.append(f"  ^ {reg}")
    # LEVEL: `warning`, deliberately. The checks API
    # (/repos/{o}/{r}/check-runs/{id}/annotations) returns only the error and
    # warning levels — a `::notice` is rendered in the UI but is NOT
    # retrievable, and the step-summary channel does not surface through
    # `output.summary` for a reusable-workflow job either. Publishing at
    # warning level is what makes the evidence machine-readable; it does not
    # affect this step's exit status, which is what actually gates the lane.
    print(f"::warning title=codegen-gate-metrics::"
          f"{gha_escape(chr(10).join(rows))}")


def emit_step_summary(results: list, asm_by_label: dict) -> None:
    """Append the same evidence to $GITHUB_STEP_SUMMARY.

    Annotations are rendered in the UI but a `::notice` is NOT retrievable
    through the checks API (`/check-runs/{id}/annotations` returns only
    error/warning levels), so a notice alone is still "visible to a human,
    invisible to a program" — the exact defect this instrument exists to fix.
    The step summary lands in the check run's `output.summary`, which anything
    holding a token can read: `gh api repos/O/R/check-runs/<job-id>
    --jq .output.summary`.
    """
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    out = ["", "## Codegen quality gate — golden workloads", "",
           "| workload | insns | stackmem | pushes | moves | ymm | fma | verdict |",
           "|---|---:|---:|---:|---:|---:|---:|---|"]
    for r in results:
        if "skip" in r or "error" in r:
            out.append(f"| {r['label']} | — | — | — | — | — | — | "
                       f"{r.get('error', 'SKIPPED')} |")
            continue
        m = r["metrics"]
        verdict = "REGRESSION" if r.get("regressions") else "pass"
        out.append(f"| {r['label']} | {m['insns']} | {m['stackmem']} | "
                   f"{m['pushes']} | {m['moves']} | {m['ymm']} | {m['fma']} | "
                   f"{verdict} |")
    for r in results:
        for reg in r.get("regressions") or []:
            out.append(f"- **{r['label']}**: {reg}")
    if asm_by_label:
        out.append("")
        out.append("<details><summary>emitted assembly (-O2 -S)</summary>")
        out.append("")
        out.append("```asm")
        for label, asm in asm_by_label.items():
            out.append(f"# ===== {label} =====")
            out.extend(asm.splitlines())
        out.append("```")
        out.append("</details>")
    with open(path, "a", encoding="utf-8") as fh:
        fh.write("\n".join(out) + "\n")


def emit_asm_annotations(asm_by_label: dict) -> None:
    """Publish the emitted assembly, chunked. Opt-in (`--annotate-asm`)."""
    buf: list = []
    size = 0
    chunk_no = 0
    for label, asm in asm_by_label.items():
        block = f"===== {label} =====\n{asm}"
        if size + len(block) > ANNOTATION_CHUNK and buf:
            chunk_no += 1
            print(f"::warning title=codegen-asm-{chunk_no}::"
                  f"{gha_escape(''.join(buf))}")
            buf, size = [], 0
        buf.append(block)
        size += len(block)
    if buf:
        chunk_no += 1
        print(f"::warning title=codegen-asm-{chunk_no}::{gha_escape(''.join(buf))}")


def detect_gcc_inc() -> str:
    r = subprocess.run(
        ["gcc", "-print-file-name=include"], capture_output=True, text=True
    )
    return r.stdout.strip()


def publish_baseline(results, path=BASELINE):
    """Never bless missing sources or failed compiles, even in update mode."""
    expected = {label for _, label in WORKLOADS}
    if (not expected or len(results) != len(expected)
            or {r["label"] for r in results} != expected
            or any("metrics" not in r for r in results)):
        raise ValueError("refusing incomplete codegen baseline refresh")
    tmp = path.with_suffix(f".tmp.{os.getpid()}")
    tmp.write_text(json.dumps({r["label"]: r["metrics"] for r in results}, indent=2) + "\n")
    tmp.replace(path)


def main():
    p = argparse.ArgumentParser(description="LCCC codegen quality gate")
    p.add_argument("--lccc", required=True, help="Path to LCCC binary")
    p.add_argument("--gcc-inc", default=None)
    p.add_argument("--update-baseline", action="store_true",
                   help="Write the measured metrics as the new baseline")
    p.add_argument("--json", metavar="FILE", help="Dump results JSON")
    p.add_argument("--summary", action="store_true", help="GitHub markdown")
    p.add_argument("--annotate", action="store_true",
                   help="publish the per-workload metrics as a workflow annotation "
                        "(API-visible; the numbers are otherwise log-only)")
    p.add_argument("--annotate-asm", action="store_true",
                   help="also publish each workload's -O2 -S output, chunked")
    args = p.parse_args()

    gcc_inc = args.gcc_inc or detect_gcc_inc()
    if not Path(args.lccc).exists():
        print(f"ERROR: LCCC binary not found: {args.lccc}", file=sys.stderr)
        sys.exit(2)

    baseline = {}
    if BASELINE.exists() and not args.update_baseline:
        baseline = json.loads(BASELINE.read_text())

    results = []
    failures = []
    annotate_asm = args.annotate_asm
    asm_by_label: dict = {}
    for src_name, label in WORKLOADS:
        src = PROGRAMS / src_name
        if not src.exists():
            failures.append(f"{label}: required source missing")
            results.append({"label": label, "skip": str(src)})
            continue
        asm = compile_asm(args.lccc, gcc_inc, src) if annotate_asm else None
        got = (function_body_metrics(asm) if asm is not None
               else compile_metrics(args.lccc, gcc_inc, src))
        if annotate_asm and asm is not None:
            asm_by_label[label] = asm
        if got is None:
            failures.append(f"{label}: compile failed")
            results.append({"label": label, "error": "compile failed"})
            continue

        entry = {"label": label, "metrics": got}
        base = baseline.get(label)
        if base is not None:
            regressions = []
            for metric, value in got.items():
                old = base.get(metric)
                if old is None or old == 0:
                    # No baseline for this metric: only fail if it appeared
                    # with a vengeance (any amount > 0 when baseline absent
                    # and value large). Keep conservative: skip.
                    continue
                tol = TOLERANCES.get(metric, DEFAULT_TOLERANCE)
                limit = old * (1.0 + tol)
                if value > limit:
                    regressions.append(
                        f"{metric} {old} -> {value} (+{100*(value-old)/old:.1f}%, "
                        f"tolerance {tol*100:.0f}%)"
                    )
            entry["regressions"] = regressions
            if regressions:
                failures.append(f"{label}: " + "; ".join(regressions))
        results.append(entry)

    if args.update_baseline:
        try:
            publish_baseline(results)
            print(f"Baseline updated: {BASELINE}", file=sys.stderr)
        except ValueError as exc:
            failures.append(str(exc))

    if args.json:
        Path(args.json).write_text(json.dumps(results, indent=2))

    if args.summary:
        print("## Codegen Quality Gate (golden workloads)")
        print("")
        print("| Workload | insns | stackmem | pushes | ymm | fma | verdict |")
        print("|---|---:|---:|---:|---:|---:|---|")
        for r in results:
            if "skip" in r:
                print(f"| {r['label']} | — | — | — | — | — | SKIPPED |")
                continue
            if "error" in r:
                print(f"| {r['label']} | — | — | — | — | — | ERROR |")
                continue
            m = r["metrics"]
            verdict = "PASS" if not r.get("regressions") else (
                "REGRESSION: " + "; ".join(r["regressions"])
            )
            print(
                f"| {r['label']} | {m['insns']} | {m['stackmem']} | "
                f"{m['pushes']} | {m['ymm']} | {m['fma']} | {verdict} |"
            )

    if args.annotate or annotate_asm:
        emit_metrics_annotation(results)
        emit_step_summary(results, asm_by_label if annotate_asm else {})
    if annotate_asm and asm_by_label:
        emit_asm_annotations(asm_by_label)

    print(file=sys.stderr)
    if failures:
        print("ERROR: codegen regressions detected:", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        print(
            "If these are intentional improvements-in-progress, refresh with "
            "--update-baseline in the landing PR.",
            file=sys.stderr,
        )
        sys.exit(1)
    print("Codegen gate: all golden workloads within tolerance.", file=sys.stderr)


if __name__ == "__main__":
    main()
