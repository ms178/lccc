#!/usr/bin/env python3
"""Hot-loop instruction density for elementwise kernels.

Why this exists
---------------
`codegen_oracle.py` reports a STATIC instruction count for the whole
function.  For a loop kernel that number is actively misleading: a compiler
that refuses to vectorize emits one tight scalar loop and "wins", while a
compiler that vectorizes pays for a prologue, a packed body, a scalar
remainder and often an alignment peel -- more static instructions for far
less work per element.  Ranking byte/word map kernels by static size
therefore rewards exactly the codegen we are trying to beat.

The meaningful figure is how many instructions the machine retires per
ELEMENT of input in steady state:

    density = instructions in the innermost loop body / bytes processed per trip

This tool recovers both terms directly from the assembly, with no source
annotations and no compiler cooperation (see :func:`bytes_per_trip` for the
step recovery and :func:`analyse` for the loop choice).

Loop identification is CONTROL FLOW, not text layout
----------------------------------------------------
The first implementation called every branch to a *textually earlier* label a
backedge.  That is wrong, and it is wrong in exactly the case that matters:
`jge .LBB7` from the remainder-loop guard of an LCCC kernel jumps backwards
in the listing but forward in the CFG (it exits the remainder loop into the
outer loop's latch).  The result was a "loop" whose body *contains* the real
inner loop, so its static instruction count was charged against a trip that
executes the inner loop VF times -- e.g. 24 instructions reported as
processing 2048 bytes for `matmul`, understating LCCC's true inner-loop cost
(15 insns / 128 B) by 20x and inverting the LCCC-vs-GCC verdict.

This version therefore builds a real CFG (basic blocks, successors,
iterative dominators) and defines:

* **backedge** -- an edge ``b -> h`` where ``h`` dominates ``b``;
* **natural loop** of that backedge -- ``h`` plus every block that reaches
  ``b`` without passing through ``h``;
* **nesting** -- loop A contains loop B when B's block set is a strict
  subset of A's;
* **innermost loop** -- a loop containing no other loop.  Only these have a
  static instruction count that means anything: a loop that encloses another
  loop executes its body a variable number of times per trip, so its
  "instructions per byte" is not a number this tool can compute and is
  reported as ``composite`` instead of being silently mis-measured.

Steady state is then the innermost loop with the largest bytes-per-trip (the
packed body; a scalar remainder is bounded by one vector width regardless of
n), ties broken toward the smaller body.

Usage:
    hot_loop_metric.py FILE.s [FILE.s ...] --symbol NAME [--json OUT]
                       [--all-loops] [--show-body] [--strict]
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# Directives and comments are not instructions.
_SKIP = re.compile(r"^\s*(?:[.#]|//|/\*|$)")
_LABEL = re.compile(r"^\s*(\.?[\w$.]+)\s*:")
_INSN = re.compile(r"^\s*([a-z][a-z0-9.]*)\s*(.*)$", re.I)
# Unconditional / conditional / terminating control flow.
_JMP = re.compile(r"^(jmp|jmpl|br)$", re.I)
_COND = re.compile(r"^j[a-z]+$", re.I)
_TERM = re.compile(r"^(ret|retq|retl|hlt|ud2|ud2a)$", re.I)
# `addq $32, %r11` / `add $0x20,%r11` / `subq $-128, %rax`
_ADD_IMM = re.compile(r"^\$(-?(?:0x)?[0-9a-fA-F]+)\s*,\s*%(\w+)$")
# `leaq 32(%rsi), %rsi`
_LEA_SELF = re.compile(r"^(-?\d+)\(%(\w+)\)\s*,\s*%(\w+)$")

_VEC_WIDTH = {"zmm": 64, "ymm": 32, "xmm": 16}


def _int(tok: str) -> int:
    tok = tok.strip()
    neg = tok.startswith("-")
    if neg:
        tok = tok[1:]
    v = int(tok, 16) if tok.lower().startswith("0x") else int(tok)
    return -v if neg else v


def extract_function(lines: list[str], symbol: str) -> list[str]:
    """Lines belonging to `symbol`, from its label to the next symbol/size."""
    out: list[str] = []
    inside = False
    for line in lines:
        m = _LABEL.match(line)
        if m and m.group(1) == symbol:
            inside = True
            continue
        if inside:
            if re.match(rf"^\s*\.size\s+{re.escape(symbol)}\b", line):
                break
            # A new global function label ends the region.
            if m and not m.group(1).startswith("."):
                break
            out.append(line)
    return out


@dataclass
class Block:
    index: int
    label: str | None
    insns: list[str] = field(default_factory=list)
    succ: list[int] = field(default_factory=list)
    pred: list[int] = field(default_factory=list)


def basic_blocks(body: list[str]) -> list[Block]:
    """Split a function body into basic blocks.

    A block starts at a label or after a terminator/branch and ends at the
    next label or at its own terminator.  Directives are dropped; labels are
    recorded on the block they open so a jump target can be resolved to a
    block index.
    """
    blocks: list[Block] = []
    cur = Block(0, None)
    blocks.append(cur)
    label_of: dict[str, int] = {}
    pending: str | None = None

    def new_block(label: str | None) -> Block:
        nxt = Block(len(blocks), label)
        blocks.append(nxt)
        return nxt

    for raw in body:
        m = _LABEL.match(raw)
        if m:
            pending = m.group(1)
            if cur.insns or cur.index == 0:
                cur = new_block(pending)
            else:
                cur.label = pending
            label_of.setdefault(pending, cur.index)
            continue
        text = raw.strip()
        if not text or _SKIP.match(text):
            continue
        im = _INSN.match(text)
        if not im:
            continue
        mnem = im.group(1).lower()
        cur.insns.append(text)
        if _TERM.match(mnem):
            cur = new_block(None)
        elif _JMP.match(mnem) or _COND.match(mnem):
            cur = new_block(None)
    # Resolve edges.  A block with no instructions (two labels in a row) is
    # not a control-flow node: it falls through, and a jump to its label
    # means a jump to the next block that actually holds code.
    for i, blk in enumerate(blocks):
        if not blk.insns:
            if i + 1 < len(blocks):
                blk.succ.append(i + 1)
            continue
        last = blk.insns[-1]
        im = _INSN.match(last)
        mnem = im.group(1).lower()
        operands = im.group(2).strip().split()[0].rstrip(",") if im.group(2) else ""
        if _TERM.match(mnem):
            continue
        if _JMP.match(mnem):
            if operands in label_of:
                blk.succ.append(_effective_block(blocks, label_of[operands]))
        elif _COND.match(mnem):
            if operands in label_of:
                blk.succ.append(_effective_block(blocks, label_of[operands]))
            if i + 1 < len(blocks):
                blk.succ.append(i + 1)
        elif i + 1 < len(blocks):
            blk.succ.append(i + 1)
    for blk in blocks:
        for s in blk.succ:
            if blk.index not in blocks[s].pred:
                blocks[s].pred.append(blk.index)
    return blocks


def _effective_block(blocks: list[Block], index: int) -> int:
    """Follow fallthroughs out of empty blocks to the block holding code."""
    seen = set()
    while not blocks[index].insns:
        if index in seen or not blocks[index].succ:
            break
        seen.add(index)
        index = blocks[index].succ[0]
    return index


def has_cycle(blocks: list[Block]) -> bool:
    """Is there any cycle in the CFG, including an irreducible one?"""
    state = [0] * len(blocks)  # 0 unseen, 1 on stack, 2 done

    def visit(node: int) -> bool:
        if state[node] == 1:
            return True
        if state[node] == 2:
            return False
        state[node] = 1
        for succ in blocks[node].succ:
            if visit(succ):
                return True
        state[node] = 2
        return False

    return any(state[i] == 0 and visit(i) for i in range(len(blocks)))


def dominators(blocks: list[Block], entry: int = 0) -> list[frozenset[int]]:
    """Iterative dominator sets (Cooper/Harvey/Kennedy without the idom tree)."""
    all_nodes = set(range(len(blocks)))
    dom: list[frozenset[int]] = [frozenset(all_nodes) for _ in blocks]
    dom[entry] = frozenset({entry})
    changed = True
    while changed:
        changed = False
        for i, blk in enumerate(blocks):
            if i == entry:
                continue
            if blk.pred:
                new = frozenset.intersection(*[dom[p] for p in blk.pred])
            else:
                new = frozenset()
            new = new | {i}
            if new != dom[i]:
                dom[i] = new
                changed = True
    return dom


@dataclass
class LoopInfo:
    header: int
    header_label: str
    blocks: list[int]
    insns: list[str]
    nested: list[str] = field(default_factory=list)

    @property
    def composite(self) -> bool:
        return bool(self.nested)


def natural_loops(blocks: list[Block]) -> list[LoopInfo]:
    """Every natural loop (backedge + dominated body), with nesting recorded.

    One loop per *header*: when several backedges share a header (an unrolled
    body split across two cycles, or a `continue` edge) their bodies are
    unioned, so a kernel is never reported twice under the same label.
    """
    dom = dominators(blocks)
    # header -> set of tail blocks with a backedge into it
    backedges: dict[int, set[int]] = {}
    for blk in blocks:
        for succ in blk.succ:
            # A backedge target must dominate the source: that is what makes
            # it a loop header rather than a jump to an earlier address.
            if succ in dom[blk.index]:
                backedges.setdefault(succ, set()).add(blk.index)
    loops: list[LoopInfo] = []
    for header, tails in sorted(backedges.items()):
        body_blocks: set[int] = {header} | set(tails)
        for tail in tails:
            stack = [tail]
            while stack:
                node = stack.pop()
                # "without passing through the header": the header's own
                # predecessors (the loop pre-header, other entries) are NOT
                # part of the loop.
                if node == header:
                    continue
                for pred in blocks[node].pred:
                    if pred not in body_blocks:
                        body_blocks.add(pred)
                        stack.append(pred)
        ordered = sorted(body_blocks)
        insns: list[str] = []
        for b in ordered:
            insns.extend(blocks[b].insns)
        loops.append(LoopInfo(
            header=header,
            header_label=blocks[header].label or f"bb{header}",
            blocks=ordered,
            insns=insns,
        ))
    # Nesting: A contains B when B's block set is a strict subset of A's.
    for loop in loops:
        for other in loops:
            if other is loop:
                continue
            if set(other.blocks) < set(loop.blocks):
                loop.nested.append(other.header_label)
    return loops


def count_instructions(block: list[str]) -> tuple[int, list[str]]:
    insns = []
    for line in block:
        s = line.strip()
        if _SKIP.match(s) or _LABEL.match(s):
            continue
        m = _INSN.match(s)
        if m:
            insns.append(s)
    return len(insns), insns


_MEM_OPERAND = re.compile(
    r"(?:-?(?:0x)?[0-9a-fA-F]+)?\((?P<base>%[\w]+)?"
    r"(?:\s*,\s*(?P<index>%[\w]+)(?:\s*,\s*(?P<scale>[1248]))?)?\)")
# `movslq %r8d, %r9` / `movq %rax, %rdx` / `leaq 8(%r10), %r10` /
# `leaq 0(,%r11,8), %r9`
_COPY = re.compile(r"^(?:movs[lb]q|movz[lb]q|mov[qblw]|movslq)\s+%(?P<src>\w+)\s*,\s*%(?P<dst>\w+)$")
_LEA_SCALED = re.compile(r"^leaq?\s+0\(\s*,\s*%(?P<src>\w+)\s*,\s*(?P<scale>[1248])\)\s*,\s*%(?P<dst>\w+)$")
_ADD_REG_IMM = re.compile(r"^(?P<mnem>add[qblw]|sub[qblw])\s+\$(?P<imm>-?(?:0x)?[0-9a-fA-F]+)\s*,\s*%(?P<dst>\w+)$")
_LEA_FROM_REG = re.compile(r"^leaq?\s+(?P<disp>-?\d+)\(%(?P<src>\w+)\)\s*,\s*%(?P<dst>\w+)$")


def _reg(name: str | None) -> str | None:
    return name[1:] if name else None


def register_advance(insns: list[str]) -> dict[str, int]:
    """Bytes each register advances per loop trip, following one definition.

    A vectorized kernel often keeps its trip counter separate from its address
    register: LCCC's packed reduction body is

        movslq %r8d, %r9
        vpaddd (%rbx,%r9), %ymm2, %ymm2
        addl $128, %r8d

    so the only register carrying the stride is `%r8d`, which appears in NO
    memory operand.  The pre-fix "advance an *addressed* register" rule
    recovered nothing, dropped the packed loop, and measured the scalar
    remainder instead -- reporting a vectorized LCCC kernel as scalar.  This
    function therefore follows the address computation: `add $imm` on a
    register, `lea disp(%self)` on itself, and one level of copy/scale
    (`movslq`, `movq`, `lea 0(,%src,scale)`).
    """
    direct: dict[str, int] = {}
    copies: dict[str, tuple[str, int]] = {}   # dst -> (src, scale)
    for s in insns:
        m = _ADD_REG_IMM.match(s)
        if m:
            imm = _int(m.group("imm"))
            if m.group("mnem").startswith("sub"):
                imm = -imm
            direct[m.group("dst")] = direct.get(m.group("dst"), 0) + imm
            continue
        m = _LEA_FROM_REG.match(s)
        if m and m.group("src") == m.group("dst"):
            direct[m.group("dst")] = direct.get(m.group("dst"), 0) + int(m.group("disp"))
            continue
        m = _LEA_SCALED.match(s)
        if m:
            copies[m.group("dst")] = (m.group("src"), int(m.group("scale")))
            continue
        m = _COPY.match(s)
        if m:
            copies[m.group("dst")] = (m.group("src"), 1)
    advance: dict[str, int] = dict(direct)

    def value(reg: str, seen: frozenset[str] = frozenset()) -> int:
        if reg in advance:
            return advance[reg]
        if reg in copies and reg not in seen:
            src, scale = copies[reg]
            got = value(src, seen | {reg}) * scale
            advance[reg] = got
            return got
        return 0

    for reg in list(direct) + list(copies):
        value(reg)
    return {k: v for k, v in advance.items() if v}


def bytes_per_trip(insns: list[str]) -> tuple[int, str]:
    """Bytes of input consumed per loop trip, with the evidence used.

    The step is the per-trip change of a memory operand's address:
    ``advance(base) + advance(index) * scale``, maximised over the loop's
    memory operands (the widest stream is the one the kernel is about).
    """
    adv = register_advance(insns)
    steps: list[tuple[int, str]] = []
    for s in insns:
        for m in _MEM_OPERAND.finditer(s):
            base, index, scale = (_reg(m.group("base")), _reg(m.group("index")),
                                  int(m.group("scale") or 1))
            step = (adv.get(base, 0) if base else 0) + (
                adv.get(index, 0) * scale if index else 0)
            if step > 0:
                steps.append((step, f"address advance {s.strip()[:40]}"))
    if steps:
        step, how = max(steps)
        return step, how

    # Fallback 1: an advance on a register that is used as a memory base at
    # all (older shape, no scale/index bookkeeping needed).
    addressed: set[str] = set()
    for s in insns:
        for reg in re.findall(r"\(%(\w+)(?:\s*,\s*%(\w+))?", s):
            addressed.update(r for r in reg if r)
    candidates = [v for k, v in adv.items() if k in addressed and v > 1]
    if candidates:
        return max(candidates), "pointer advance"

    # Fallback 2: the vector load width (a packed body with no recoverable
    # index, e.g. a fully unrolled pointer chain).
    width = 0
    loads = 0
    for s in insns:
        for tag, w in _VEC_WIDTH.items():
            if f"%{tag}" in s:
                width = max(width, w)
        if re.match(r"^\s*v?mov[a-z]*\s+[^,]*\(", s) or re.match(r"^\s*movz", s):
            loads += 1
    if width and loads:
        return width, "vector load width"
    if loads:
        return 1, "scalar load"
    return 0, "unknown"


def steady_state_loop(loops: list[LoopInfo]) -> tuple[LoopInfo | None, str | None]:
    """The innermost loop that dominates run time for a unit-stride kernel.

    NOT the tightest cycle: a vectorized kernel contains at least two
    backedges -- the packed body and the scalar remainder -- and the
    remainder is usually the SHORTER one, so "tightest" would report the
    vectorizing compiler as if it had stayed scalar.

    NOT any composite loop either: a loop that encloses another loop runs its
    body a variable number of times per trip, so a static instruction count
    divided by bytes-per-trip is not a density.  Those are reported as
    ``composite`` rather than measured (see :func:`analyse`).

    Among the innermost loops the steady-state one consumes the MOST input
    per trip: a remainder is bounded by one vector width regardless of n,
    while the packed body's trip count grows with n.  Ties go to the shorter
    body.
    """
    innermost = [loop for loop in loops if not loop.nested]
    if not innermost:
        return None, ("only composite (nested) loops found; no static density "
                      "is definable for them")
    measured: list[tuple[int, int, LoopInfo]] = []
    for loop in innermost:
        n, insns = count_instructions(loop.insns)
        step, _ = bytes_per_trip(insns)
        if step > 0:
            measured.append((step, n, loop))
    if not measured:
        return None, "no innermost loop with a recoverable per-trip step"
    # Largest step wins; fewest instructions breaks a tie.
    measured.sort(key=lambda item: (-item[0], item[1]))
    return measured[0][2], None


def _loop_row(loop: LoopInfo, file: str) -> dict:
    n, insns = count_instructions(loop.insns)
    vec = max(
        (w for s in insns for tag, w in _VEC_WIDTH.items() if f"%{tag}" in s),
        default=0,
    )
    if loop.composite:
        # A loop that encloses another loop runs its body a VARIABLE number of
        # times per trip, so "bytes per trip" is not a static quantity here and
        # no density is definable.  Reporting the pointer advances found in the
        # body would invent one: the composite `matmul` shells score 2177
        # "bytes per trip" (the outer 2048 plus the inner 128 the body also
        # contains), which is the pre-fix bug in a different unit.  Composite
        # rows therefore carry the raw instruction count and NOTHING else.
        return {
            "file": file,
            "loop_label": loop.header_label,
            "composite": True,
            "nested_loops": list(loop.nested),
            "insns": n,
            "bytes_per_trip": 0,
            "insns_per_byte": None,
            "widest_vector_bits": vec * 8,
            "step_evidence": "composite (encloses another loop): no static density",
            "body": insns,
        }
    step, how = bytes_per_trip(insns)
    return {
        "file": file,
        "loop_label": loop.header_label,
        "composite": False,
        "nested_loops": list(loop.nested),
        "insns": n,
        "bytes_per_trip": step,
        # Full precision: a measurement must not be rounded before it is
        # compared; the table formats it, the JSON keeps it.
        "insns_per_byte": (n / step) if step else None,
        "widest_vector_bits": vec * 8,
        "step_evidence": how,
        "body": insns,
    }


def analyse_lines(lines: list[str], symbol: str, file: str = "<lines>") -> dict:
    """Analyse an in-memory assembly listing (no temp file, no re-read).

    Split out of :func:`analyse` so batch tools (`scripts/loop_density_oracle.py`)
    can measure one compiler's output without round-tripping through the
    filesystem, which also keeps their numbers byte-identical to this tool's.
    """
    body = extract_function(lines, symbol)
    if not body:
        return {"file": file, "error": f"symbol {symbol} not found"}
    blocks = basic_blocks(body)
    loops = natural_loops(blocks)
    rows = [_loop_row(loop, file) for loop in loops]
    if not loops:
        if has_cycle(blocks):
            return {"file": file, "symbol": symbol,
                    "error": "irreducible control flow (a cycle with no "
                             "dominating header): no natural loop, no density",
                    "loops": rows}
        return {"file": file, "error": "no loop found", "loops": rows}
    chosen, reason = steady_state_loop(loops)
    if chosen is None:
        return {"file": file, "symbol": symbol, "error": reason, "loops": rows}
    result = _loop_row(chosen, file)
    result["symbol"] = symbol
    result["loops"] = rows
    return result


def analyse(path: Path, symbol: str) -> dict:
    return analyse_lines(path.read_text(errors="replace").splitlines(),
                         symbol, file=str(path))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--symbol", required=True)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--show-body", action="store_true")
    ap.add_argument("--all-loops", action="store_true",
                    help="also print every loop found, with its nesting class")
    ap.add_argument("--strict", action="store_true",
                    help="exit 1 if any given assembly has no measurable "
                         "innermost loop (instead of reporting 'no density')")
    args = ap.parse_args()

    results = [analyse(f, args.symbol) for f in args.files]
    width = max(len(Path(r["file"]).stem) for r in results)
    print(f"{'compiler':<{width}}  {'insns':>5} {'B/trip':>6} {'insn/B':>7} {'vec':>5}")
    print("-" * (width + 28))
    ranked = [r for r in results if r.get("insns_per_byte") is not None]
    best = min((r["insns_per_byte"] for r in ranked), default=None)
    failures = 0
    for r in results:
        name = Path(r["file"]).stem
        if "error" in r:
            print(f"{name:<{width}}  {r['error']}")
            failures += 1
            continue
        if r["insns_per_byte"] is None:
            # The step recovery failed: report the raw facts rather than
            # inventing a density (a wrong density is worse than none).
            print(f"{name:<{width}}  {r['insns']:>5} {'?':>6} {'n/a':>7} "
                  f"{r['widest_vector_bits'] or 0:>4}b  (step: {r['step_evidence']})")
            failures += 1
            continue
        mark = "  <-- best" if best is not None and r["insns_per_byte"] == best else ""
        print(f"{name:<{width}}  {r['insns']:>5} {r['bytes_per_trip']:>6} "
              f"{r['insns_per_byte']:>7.4f} {r['widest_vector_bits'] or 0:>4}b{mark}")
        if args.show_body:
            for s in r["body"]:
                print(f"      {s}")
        if args.all_loops:
            for row in r.get("loops", []):
                kind = "composite" if row["composite"] else "innermost"
                nested = f" nests {','.join(row['nested_loops'])}" if row["nested_loops"] else ""
                print(f"    loop {row['loop_label']:<10} {kind:<9} "
                      f"insns={row['insns']:>4} B/trip={row['bytes_per_trip']:>5}"
                      f"{nested}")
    if args.json:
        args.json.write_text(json.dumps(results, indent=2))
    return 1 if (args.strict and failures) else 0


if __name__ == "__main__":
    sys.exit(main())
