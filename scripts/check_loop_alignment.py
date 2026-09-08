#!/usr/bin/env python3
"""Loop-alignment codegen audit for LCCC.

Programmatic verification of the hot-loop alignment policy decided by
``src/passes/loop_align.rs``.  The audit compiles a fixed corpus of loop
shapes with the local compiler, then checks the EMITTED ASSEMBLY (not the
IR) for the oracle-derived directive contract:

  * a loop whose body contains SIMD instructions is preceded by the bounded
    vector cascade ``.p2align 5,,15`` + ``.p2align 4`` (32 bytes when at
    most 15 padding bytes are needed, else 16) — GCC 16.2's 32-byte
    vector-loop alignment with the padding cost capped at half of GCC's
    worst case, degrading to the ICX/Clang 16-byte form;
  * a loop whose header exit comparison proves a constant trip count <= 4
    is left unaligned (GCC and Clang leave constant-trip-3 loops alone:
    the one-shot padding cannot pay back over at most four iterations);
  * a scalar loop header is preceded by the bounded cascade
    ``.p2align 4,,10`` + ``.p2align 3`` (16 bytes when <= 10 padding bytes
    are needed, otherwise 8) — GCC's ASM_OUTPUT_MAX_SKIP_ALIGN shape;
  * -O0/-Os/-Oz emit no loop padding at all;
  * -fno-align-loops suppresses every loop directive (a deliberate
    improvement over GCC, whose vector-loop alignment cannot be turned
    off), while function-entry alignment is governed separately by
    -falign-functions;
  * -falign-loops=N[:M] overrides the loop policy with a custom directive;
  * the CCC_NO_LOOP_ALIGN kill switch disables the pass for A/B runs.

The audit is self-verifying in both directions: a missing directive fails,
and so does an unexpected one (e.g. padding at -Os).  It never executes the
generated code — the GCC-differential regression suite owns semantics; this
file owns the code-shape contract.

Usage:
    scripts/check_loop_alignment.py [--lccc PATH] [-v]
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc"

# Each corpus entry: (name, source, expected SIMD classification of its
# single hot loop at -O2).  The scalar nest keeps an inner loop the
# vectorizer declines (constant trip < 2*width) so a genuinely scalar loop
# survives to codegen.
CORPUS = [
    (
        "vec_axpy",
        """
void vec_axpy(float *restrict y, const float *restrict x, float a, int n) {
    for (int i = 0; i < n; i++)
        y[i] = y[i] * a + x[i];
}
""",
        "vector",
    ),
    (
        "vec_isum",
        """
int vec_isum(const int *restrict a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += a[i];
    return s;
}
""",
        "vector",
    ),
    (
        "scalar_nest",
        """
void scalar_nest(int *restrict d, const int *restrict s, int n) {
    for (int i = 0; i < n; i++) {
        int acc = 0;
        for (int j = 0; j < 3; j++)
            acc += s[i * 3 + j] * (j + 1);
        d[i] = acc;
    }
}
""",
        "scalar",
    ),
    (
        "scalar_memcpyish",
        """
unsigned scalar_hash(const unsigned *restrict k, int n) {
    unsigned h = 2166136261u;
    for (int i = 0; i < n; i++)
        h = (h ^ k[i]) * 16777619u;
    return h;
}
""",
        "scalar",
    ),
    (
        # Constant trip 3 with an opaque call in the body: the call keeps
        # the unroller from dissolving the loop, so a genuinely tiny loop
        # reaches codegen and must stay UNPADDED (GCC/Clang parity).
        "tiny_trip_call",
        """
extern int weigh(int v);
int tiny_trip_call(const int *restrict a) {
    int acc = 0;
    for (int j = 0; j < 3; j++)
        acc += weigh(a[j]);
    return acc;
}
""",
        "scalar",
    ),
]

VEC_MNEMONIC = re.compile(r"^\s*v[a-z]|^\s*movdq[au]|^\s*movdqa|^\s*p[a-z]")
LABEL_DEF = re.compile(r"^(\.L\S+):")
P2ALIGN = re.compile(r"^\s*\.p2align\s+(\d+)(?:\s*,\s*(\S+))?(?:\s*,\s*(\d+))?\s*$")
BRANCH = re.compile(r"^\s*(j\w+)\s+(\.L\S+)")

# Scalar SSE (movss/mulss/comiss/…, with or without a VEX `v` prefix) is NOT
# SIMD: the remainder loops the vectorizer emits are scalar in IR terms, and
# the alignment pass classifies them by IR intrinsics, not mnemonic prefixes.
_NON_PACKED_P = {"push", "pop", "pushf", "popf", "pause", "prefetch", "prefetchw"}


def is_simd_insn(line: str) -> bool:
    text = line.strip()
    if not text or text.startswith((".", "#")):
        return False
    # A ymm/zmm operand is unambiguous.
    if "%ymm" in text or "%zmm" in text:
        return True
    m = re.match(r"^(?:v)([a-z0-9]+)", text) or re.match(r"^([a-z0-9]+)", text)
    if not m:
        return False
    mn = m.group(1)
    if mn in _NON_PACKED_P:
        return False
    return (
        mn.startswith("p")  # packed integer SSE (paddb, pxor, punpck…)
        or mn.endswith("ps")  # packed single (vmovups, vaddps…)
        or mn.endswith("pd")  # packed double
        or mn.startswith(("movdq", "movup", "movap", "movlh", "movhl"))
    )


def compile_asm(lccc: Path, source: str, flags: list[str], wd: str) -> list[str]:
    src = Path(wd) / "t.c"
    src.write_text(source)
    out = Path(wd) / "t.s"
    r = subprocess.run(
        [str(lccc), *flags, "-S", str(src), "-o", str(out)],
        capture_output=True, text=True, timeout=120,
    )
    if r.returncode != 0:
        raise RuntimeError(f"compile failed: {r.stderr.strip()[:300]}")
    return out.read_text().splitlines()


def _build_cfg(lines: list[str]) -> tuple[dict[str, list[str]], dict[str, int], list[str]]:
    """Recover the basic-block CFG of one function's assembly.

    Returns (successors, label line index, block order).  Blocks are named
    by their label; the implicit block starting at the top is "‹entry›".
    Fallthrough happens when a block's last instruction is neither an
    unconditional jump nor a return.
    """
    labels: dict[str, int] = {}
    order: list[str] = []
    for i, line in enumerate(lines):
        m = LABEL_DEF.match(line)
        if m:
            labels.setdefault(m.group(1), i)
            order.append(m.group(1))
    succ: dict[str, list[str]] = {}
    # Determine the block each instruction belongs to and each block's last
    # instruction line.
    block_starts = [0] + [labels[l] for l in order]
    block_of_line: dict[int, str] = {}
    starts_sorted = sorted(set(block_starts))
    for bi, s in enumerate(starts_sorted):
        e = starts_sorted[bi + 1] if bi + 1 < len(starts_sorted) else len(lines)
        name = next(l for l in order if labels[l] == s) if s != 0 else "‹entry›"
        if s == 0 and order and labels[order[0]] == 0:
            name = order[0]
        for i in range(s, e):
            block_of_line[i] = name
    all_blocks = ["‹entry›"] + [l for l in order if labels[l] != 0]
    for b in all_blocks:
        succ.setdefault(b, [])
    for bi, s in enumerate(starts_sorted):
        e = starts_sorted[bi + 1] if bi + 1 < len(starts_sorted) else len(lines)
        name = next(l for l in order if labels[l] == s) if s != 0 else "‹entry›"
        if s == 0 and order and labels[order[0]] == 0:
            name = order[0]
        # Collect every branch target in the block up to (and including)
        # the first unconditional transfer. A conditional branch has a
        # fallthrough edge too — its target line keeps executing — and the
        # fallthrough may itself be a `jmp` (the classic `jl back; jmp out`
        # loop tail), so both targets must be recorded.
        terminated = False
        for i in range(s, e):
            m = BRANCH.match(lines[i])
            if m:
                succ.setdefault(name, []).append(m.group(2))
                if m.group(1) == "jmp":
                    terminated = True
                    break
            elif re.match(r"^\s*ret\b", lines[i]):
                terminated = True
                break
        if not terminated and bi + 1 < len(starts_sorted):
            nxt = starts_sorted[bi + 1]
            nname = next((l for l in order if labels[l] == nxt), None)
            if nname:
                succ.setdefault(name, []).append(nname)
    return succ, labels, all_blocks


def _dominators(succ: dict[str, list[str]], entry: str) -> dict[str, set[str]]:
    preds: dict[str, list[str]] = {}
    for b, ss in succ.items():
        for s in ss:
            preds.setdefault(s, []).append(b)
    blocks = list(succ.keys())
    dom = {b: set(blocks) for b in blocks}
    dom[entry] = {entry}
    changed = True
    while changed:
        changed = False
        for b in blocks:
            if b == entry:
                continue
            ps = preds.get(b, [])
            new = set(blocks) if not ps else set.intersection(*(dom[p] for p in ps))
            new.add(b)
            if new != dom[b]:
                dom[b] = new
                changed = True
    return dom


def loop_directives(lines: list[str], want_simd: str) -> dict[str, list[str]]:
    """Map each natural-loop header label to the .p2align directives that
    immediately precede it, for loops whose SIMD classification matches
    `want_simd` ('vector' or 'scalar').

    The loop model mirrors `passes::loop_align` exactly: a natural loop is
    identified by a CFG backedge whose target dominates its source — NOT by
    layout-order backward jumps (a loop-exit branch also jumps backward).
    """
    succ, labels, blocks = _build_cfg(lines)
    if not blocks:
        return {}
    entry = blocks[0]
    dom = _dominators(succ, entry)
    preds: dict[str, list[str]] = {}
    for b, ss in succ.items():
        for s in ss:
            preds.setdefault(s, []).append(b)

    headers: set[str] = set()
    backedges: list[tuple[str, str]] = []  # (tail, header)
    for src_block, ss in succ.items():
        for tgt in ss:
            if tgt in dom.get(src_block, set()) and tgt in labels:
                headers.add(tgt)
                backedges.append((src_block, tgt))

    def natural_body(h: str, t: str) -> set[str]:
        # Mirrors loop_analysis::compute_loop_body: the header is
        # pre-inserted so the predecessor walk stops there; a self-loop's
        # body is the header alone.
        body = {h}
        if h == t:
            return body
        body.add(t)
        work = [t]
        while work:
            b = work.pop()
            for p in preds.get(b, []):
                if p not in body:
                    body.add(p)
                    work.append(p)
        return body

    out: dict[str, list[str]] = {}
    for h in headers:
        head = labels[h]
        # Union of the bodies of every backedge targeting h (mirrors
        # merge_loops_by_header).
        body_blocks: set[str] = set()
        for t, hh in backedges:
            if hh == h:
                body_blocks |= natural_body(h, t)
        is_simd = False
        for b in body_blocks:
            if b == "‹entry›":
                continue
            i = labels[b]
            # to the next label
            nxt = min((labels[l] for l in labels if labels[l] > i), default=len(lines))
            if any(is_simd_insn(lines[k]) for k in range(i, min(nxt, len(lines)))):
                is_simd = True
                break
        kind = "vector" if is_simd else "scalar"
        if kind != want_simd:
            continue
        directives: list[str] = []
        j = head - 1
        while j >= 0:
            if P2ALIGN.match(lines[j]):
                directives.append(lines[j].strip())
                j -= 1
            else:
                break
        directives.reverse()
        out[h] = directives
    return out


def check(name: str, ok: bool, detail: str, failures: list[str], verbose: bool) -> None:
    if ok:
        if verbose:
            print(f"  ok    {name}")
    else:
        failures.append(f"{name}: {detail}")
        print(f"  FAIL  {name}: {detail}")


def run(lccc: Path, verbose: bool) -> int:
    failures: list[str] = []
    with tempfile.TemporaryDirectory() as wd:
        for fname, source, kind in CORPUS:
            print(f"[{fname}] expected {kind} loop")
            # ── -O2 default policy ─────────────────────────────────────
            lines = compile_asm(lccc, source, ["-O2"], wd)
            loops = loop_directives(lines, kind)
            check(
                f"{fname}/O2/has-loop",
                bool(loops),
                "no loop of the expected kind survived to codegen",
                failures, verbose,
            )
            for tgt, dirs in loops.items():
                if kind == "vector":
                    want = [".p2align 5,,15", ".p2align 4"]
                elif fname == "tiny_trip_call":
                    # A constant trip count <= 4 must suppress padding
                    # entirely (the exclusion is the CONTRACT, not an
                    # accident): assert the negative direction too.
                    want = []
                else:
                    want = [".p2align 4,,10", ".p2align 3"]
                check(
                    f"{fname}/O2/{tgt}",
                    dirs == want,
                    f"directives {dirs} != {want}",
                    failures, verbose,
                )

            # ── -O0 / -Os: no loop padding at all ──────────────────────
            for opt in (["-O0"], ["-Os"], ["-Oz"]):
                lines = compile_asm(lccc, source, opt, wd)
                both = {**loop_directives(lines, "vector"), **loop_directives(lines, "scalar")}
                padded = {t: d for t, d in both.items() if d}
                check(
                    f"{fname}/{' '.join(opt)}/no-pad",
                    not padded,
                    f"unexpected directives {padded}",
                    failures, verbose,
                )

            # ── -fno-align-loops: loop padding suppressed ──────────────
            lines = compile_asm(lccc, source, ["-O2", "-fno-align-loops"], wd)
            both = {**loop_directives(lines, "vector"), **loop_directives(lines, "scalar")}
            padded = {t: d for t, d in both.items() if d}
            check(
                f"{fname}/no-align-loops",
                not padded,
                f"unexpected directives {padded}",
                failures, verbose,
            )

            # ── custom policy ──────────────────────────────────────────
            lines = compile_asm(lccc, source, ["-O2", "-falign-loops=32:8"], wd)
            loops = {**loop_directives(lines, "vector"), **loop_directives(lines, "scalar")}
            for tgt, dirs in loops.items():
                check(
                    f"{fname}/custom/{tgt}",
                    dirs == [".p2align 5,,8"],
                    f"directives {dirs} != ['.p2align 5,,8']",
                    failures, verbose,
                )

            # ── kill switch ────────────────────────────────────────────
            env = dict(os.environ, CCC_NO_LOOP_ALIGN="1")
            src = Path(wd) / "t2.c"
            src.write_text(source)
            out = Path(wd) / "t2.s"
            subprocess.run(
                [str(lccc), "-O2", "-S", str(src), "-o", str(out)],
                env=env, capture_output=True, text=True, timeout=120, check=True,
            )
            lines = out.read_text().splitlines()
            both = {**loop_directives(lines, "vector"), **loop_directives(lines, "scalar")}
            padded = {t: d for t, d in both.items() if d}
            check(
                f"{fname}/kill-switch",
                not padded,
                f"unexpected directives {padded}",
                failures, verbose,
            )

    # ── function alignment is a separate knob ─────────────────────────
    with tempfile.TemporaryDirectory() as wd:
        lines = compile_asm(lccc, "int f(void){return 7;}\n", ["-O2"], wd)
        check(
            "function/O2/16",
            any(P2ALIGN.match(l) and l.strip() == ".p2align 4" for l in lines),
            "function entry not 16-byte aligned at -O2",
            failures, verbose,
        )
        lines = compile_asm(lccc, "int f(void){return 7;}\n", ["-O2", "-fno-align-functions"], wd)
        check(
            "function/no-align",
            not any(P2ALIGN.match(l) for l in lines),
            "function alignment not suppressed by -fno-align-functions",
            failures, verbose,
        )
        lines = compile_asm(lccc, "int f(void){return 7;}\n", ["-O2", "-falign-functions=64"], wd)
        check(
            "function/64",
            any(l.strip() == ".p2align 6" for l in lines),
            "-falign-functions=64 did not produce .p2align 6",
            failures, verbose,
        )
        lines = compile_asm(lccc, "int f(void){return 7;}\n", ["-Os"], wd)
        check(
            "function/Os",
            not any(P2ALIGN.match(l) for l in lines),
            "-Os must not align function entries",
            failures, verbose,
        )

    # ── invalid flag values are rejected ───────────────────────────────
    src = Path(tempfile.mkstemp(suffix=".c")[1])
    src.write_text("int f(void){return 7;}\n")
    for bad in ("12", "0", "abc", "32:xyz"):
        r = subprocess.run(
            [str(lccc), f"-falign-loops={bad}", "-c", str(src), "-o", "/dev/null"],
            capture_output=True, text=True, timeout=60,
        )
        check(
            f"flag-reject/{bad}",
            r.returncode != 0,
            "compiler accepted an invalid -falign-loops value",
            failures, verbose,
        )
    src.unlink(missing_ok=True)

    if failures:
        print(f"\nloop-alignment audit: {len(failures)} FAILURES")
        return 1
    print("\nloop-alignment audit: PASS")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC), type=Path)
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    if not args.lccc.exists():
        print(f"error: lccc not built at '{args.lccc}'", file=sys.stderr)
        return 2
    return run(args.lccc, args.verbose)


if __name__ == "__main__":
    sys.exit(main())
