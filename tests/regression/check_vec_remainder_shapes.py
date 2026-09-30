#!/usr/bin/env python3
"""Per-function loop census for ZERO-REM-1 (dead vectorizer remainder loops).

The correctness corpus (`vec_dead_remainder.c`) proves the transform produces
the right answers; it cannot prove the transform FIRED.  A map vectorizer that
silently stops vectorizing still passes every output check, so this script
pins the shape of the emitted code per function:

    loops      number of loops = strongly connected components of the
               function's CFG that contain a cycle.  Backward branches are
               NOT used: lccc lays the vectorized loop, the exit block and
               the scalar mirror out in an order that makes some
               logically-forward edges point at lower addresses, and a
               rotated loop can put its back edge physically before its
               header.  A branch count therefore under-reports; an SCC is
               layout-independent, which is what "how many loops does this
               function contain" actually needs.  See `count_loops`.
    packed     instructions naming an %xmm/%ymm register

and asserts, for every shape in `vec_dead_remainder_shapes.c`:

  * exact-multiple trip count  -> 1 loop  (the dead scalar mirror is gone)
  * non-multiple trip count    -> 2 loops (packed loop + scalar tail)
  * may-alias stream           -> 2 loops (the mirror is the dependence
                                  guard's fallback and must be kept)

A shape that stops vectorizing is a FAIL, not a skip: `packed == 0` would make
"1 loop" true for the wrong reason -- a purely scalar loop is also one loop.
The two shapes the map vectorizer does not handle today (16-bit elements and
the `s[i + 1]` stream) are listed explicitly and are asserted to stay scalar,
so the gap is documented and any change to it is visible.

Usage:
    python3 tests/regression/check_vec_remainder_shapes.py ASM.s [ASM.s ...]
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

_SYMBOL = r"[A-Za-z_$][\w.$@]*"
# Local labels start with a dot (`.LBB2:`), which `_SYMBOL` deliberately does
# not admit: it is used for symbol names, where a leading dot means something
# else entirely.
_LSYMBOL = r"\.?[A-Za-z_$][\w.$@]*"
_TYPE_FUNCTION = re.compile(rf"^\s*\.type\s+({_SYMBOL})\s*,\s*@function\b")
_SIZE_FUNCTION = re.compile(rf"^\s*\.size\s+({_SYMBOL})\s*,")
_LABEL_DEF = re.compile(rf"^({_LSYMBOL}):")
_DIRECTIVE = re.compile(r"^\s*\.")
# lccc annotates each function with `# LCCC_RET_*` markers before the
# prologue; they are comments, not code, and must not inflate the census.
_COMMENT = re.compile(r"^\s*#")
# `jmp`, `jl`, `jge`, `jne`, ... — every control transfer with a label target.
_JUMP = re.compile(rf"^\s*jmp\s+({_LSYMBOL})\s*$")
_COND_JUMP = re.compile(rf"^\s*j[a-z]{{1,3}}\s+({_LSYMBOL})\s*$")
_RET = re.compile(r"^\s*(ret|retq|jmp\s+\*|hlt|ud2)\b")
_PACKED = re.compile(r"%[xy]mm\d+\b")


def _skippable(line: str) -> bool:
    """True for lines that carry no code.

    Local labels (`.LBB2:`) start with a dot like a directive but are the
    CFG's block names, so they are NOT skippable; `#` comments are.
    """
    if not line.strip() or _COMMENT.match(line):
        return True
    return bool(_DIRECTIVE.match(line)) and not _LABEL_DEF.match(line)

# function -> expected number of loops (CFG SCCs containing a cycle; see `count_loops`)
EXPECT_LOOPS: dict[str, int] = {
    # Exact multiples of the packed width: the mirror cannot iterate.
    "shape_u32_exact": 1,
    "shape_u8_exact": 1,
    "shape_u64_exact": 1,
    "shape_i32_exact": 1,
    "shape_f32_exact": 1,
    "shape_f64_exact": 1,
    "shape_two_src_exact": 1,
    "shape_escape_exact": 1,
    # Non-multiples: the scalar tail is live code.
    "shape_u32_tail": 2,
    "shape_u8_tail": 2,
    "shape_u64_tail": 2,
    "shape_f32_tail": 2,
    "shape_u16_tail": 2,
    # May-alias: the runtime dependence guard re-enters the mirror.
    "shape_alias_exact": 2,
}

# Shapes the map vectorizer does not handle (pre-existing, unrelated to this
# transform).  They are asserted to stay scalar so that the table cannot rot.
EXPECT_SCALAR = {
    # 16-bit elements: the map analyzer declines the 32-element shape (it is
    # left to the scalar unroller) while the 35-element one is vectorized with
    # a live tail.  The asymmetry predates this transform and is pinned here so
    # it cannot change unnoticed.
    "shape_u16_exact",
    # `s[i + 1]` is not a unit-stride stream, so the map pattern never matches.
    "shape_alias_fwd",
}

# The dead mirror costs a whole scalar loop: an exact-multiple shape must be
# materially smaller than its non-multiple twin (same body, one element more).
SMALLER_THAN_TWIN = [
    ("shape_u32_exact", "shape_u32_tail"),
    ("shape_u8_exact", "shape_u8_tail"),
    ("shape_u64_exact", "shape_u64_tail"),
    ("shape_f32_exact", "shape_f32_tail"),
]


def parse_blocks(lines: list[str]) -> tuple[list[str], dict[str, int], list[list[str]]]:
    """Split an assembly function body into basic blocks.

    Returns `(labels, label_to_block, bodies)` where `bodies[i]` is the list of
    instruction lines of block `i` (the terminator last).
    """
    labels: list[str] = []
    label_to_block: dict[str, int] = {}
    bodies: list[list[str]] = []
    need_new_block = False
    for line in lines:
        m = _LABEL_DEF.match(line)
        if m:
            name = m.group(1)
            if name in label_to_block:
                continue
            label_to_block[name] = len(labels)
            labels.append(name)
            bodies.append([])
            # The label names the block the previous control transfer fell
            # into; without clearing the flag its body would be pushed into a
            # SECOND, unnamed block and the branch to this label would land on
            # an empty one (observed: a loop whose back edge targeted an empty
            # phantom, so the loop vanished from the census).
            need_new_block = False
            continue
        if not line.strip() or _skippable(line):
            continue
        if not bodies or need_new_block:
            # Instruction before any label (entry block), or the line after a
            # control transfer with no label in between: start a fresh block.
            # The block is created LAZILY so that a label on the very next
            # line names this block instead of leaving an empty phantom in
            # front of it (a phantom would swallow the fall-through edge).
            labels.append("")
            bodies.append([])
            need_new_block = False
        bodies[-1].append(line)
        # lccc emits `jl .L...` / `jne .L...` and the following `jmp` with no
        # label between them, so a basic block must be closed after EVERY
        # control transfer — not only at the next label.  Without this the
        # `jmp` would be taken for the block's terminator and the conditional
        # edge (the loop's back edge) would vanish from the CFG.
        if _JUMP.match(line) or _COND_JUMP.match(line) or _RET.match(line):
            need_new_block = True
    while bodies and not bodies[-1]:
        bodies.pop()
        labels.pop()
    return labels, label_to_block, bodies


def _terminator_kind(line: str) -> tuple[str, str | None]:
    """Classify a block's last instruction.

    "uncond" — an unconditional `jmp`, "cond" — a conditional branch (target
    plus fall-through), "ret" — a real exit (`ret`/`hlt`/`ud2`/indirect jump),
    "fallthrough" — the block simply ends, so control runs into the next one
    (e.g. a prologue block whose last instruction is a `mov`).
    """
    jm = _JUMP.match(line)
    if jm:
        return ("uncond", jm.group(1))
    cm = _COND_JUMP.match(line)
    if cm:
        return ("cond", cm.group(1))
    if _RET.match(line):
        return ("ret", None)
    return ("fallthrough", None)


def count_loops(lines: list[str]) -> int:
    """Number of loops in the function = SCCs of its CFG that contain a cycle.

    Counting *backward branches* is wrong: lccc lays the vectorized loop, the
    exit block and the scalar mirror out in an order that makes some
    logically-forward edges point at lower addresses, and a rotated loop can
    put its back edge physically before its header.  A strongly connected
    component is layout-independent, which is exactly what the "how many loops
    does this function contain" question needs.

    The CFG is tiny (a vectorized loop is a handful of blocks), so the answer
    is computed with a transitive closure instead of a stack-explicit Tarjan:
    correctness by construction beats cleverness in a gate script.
    """
    _labels, label_to_block, bodies = parse_blocks(lines)
    n = len(bodies)
    if n == 0:
        return 0
    succ: list[set[int]] = [set() for _ in range(n)]
    for i, body in enumerate(bodies):
        if not body:
            continue
        kind, target = _terminator_kind(body[-1])
        if target is not None and target in label_to_block:
            succ[i].add(label_to_block[target])
        if (kind == "cond" or kind == "fallthrough") and i + 1 < n:
            succ[i].add(i + 1)  # fall-through to the physically next block
    # Reachability (Floyd-Warshall over a boolean matrix).
    reach = [[j in succ[i] for j in range(n)] for i in range(n)]
    for k in range(n):
        for i in range(n):
            if reach[i][k]:
                ri = reach[i]
                rk = reach[k]
                for j in range(n):
                    if rk[j]:
                        ri[j] = True
    # Group mutually reachable nodes; an SCC with a cycle is one loop.
    seen: set[int] = set()
    loops = 0
    for i in range(n):
        if i in seen:
            continue
        group = [j for j in range(n) if reach[i][j] and reach[j][i]]
        seen.update(group)
        if len(group) > 1 or reach[i][i]:
            loops += 1
    return loops


def analyze(path: Path) -> dict[str, dict[str, int]]:
    """Return {function: {"insns": n, "loops": n, "packed": n}}."""
    lines = path.read_text().splitlines()

    funcs: dict[str, dict[str, int]] = {}
    current: str | None = None
    body: list[str] = []
    for line in lines:
        m = _TYPE_FUNCTION.match(line)
        if m:
            if current is not None:
                stats = funcs[current]
                stats["loops"] = count_loops(body)
            current = m.group(1)
            funcs[current] = {"insns": 0, "loops": 0, "packed": 0}
            body = []
            continue
        if current is None:
            continue
        if _SIZE_FUNCTION.match(line):
            stats = funcs[current]
            stats["loops"] = count_loops(body)
            current = None
            body = []
            continue
        if not line.strip() or _skippable(line):
            continue
        stats = funcs[current]
        stats["insns"] += 1
        body.append(line)
        if _PACKED.search(line):
            stats["packed"] += 1
    if current is not None:
        funcs[current]["loops"] = count_loops(body)
    return funcs


def check(path: Path) -> list[str]:
    funcs = analyze(path)
    problems: list[str] = []
    print(f"── {path}")
    print(f"   {'function':<24s} {'insns':>6s} {'loops':>6s} {'packed':>7s}  verdict")
    for name in sorted(funcs):
        stats = funcs[name]
        insns, loops, packed = stats["insns"], stats["loops"], stats["packed"]
        if name in EXPECT_LOOPS:
            want = EXPECT_LOOPS[name]
            if packed == 0:
                problems.append(
                    f"{path.name}: {name} stopped vectorizing (packed=0); "
                    "'1 loop' would now be true of a scalar loop"
                )
                verdict = "FAIL scalar"
            elif loops != want:
                problems.append(
                    f"{path.name}: {name} has {loops} loop(s), expected {want}"
                )
                verdict = f"FAIL loops!={want}"
            else:
                verdict = f"ok ({want} loop(s))"
        elif name in EXPECT_SCALAR:
            if packed != 0:
                problems.append(
                    f"{path.name}: {name} now vectorizes (packed={packed}); "
                    "update EXPECT_SCALAR and give it a loop expectation"
                )
                verdict = "FAIL vectorized"
            else:
                verdict = "ok (scalar, documented)"
        else:
            verdict = "info"
        print(f"   {name:<24s} {insns:6d} {loops:6d} {packed:7d}  {verdict}")

    for small, big in SMALLER_THAN_TWIN:
        if small not in funcs or big not in funcs:
            problems.append(f"{path.name}: missing {small} or {big}")
            continue
        if funcs[small]["insns"] >= funcs[big]["insns"]:
            problems.append(
                f"{path.name}: {small} ({funcs[small]['insns']} insns) is not "
                f"smaller than {big} ({funcs[big]['insns']} insns)"
            )
        else:
            print(
                f"   {small} ({funcs[small]['insns']} insns) < "
                f"{big} ({funcs[big]['insns']} insns)  ok"
            )
    return problems


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    problems: list[str] = []
    for arg in argv[1:]:
        problems += check(Path(arg))
    if problems:
        print("\nFAILURES:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("\ncheck_vec_remainder_shapes: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
