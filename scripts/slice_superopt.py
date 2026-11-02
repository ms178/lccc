#!/usr/bin/env python3
"""Minotaur-lite: enumerative superoptimisation of small integer slices.

Second transplant from Regehr & Livinskii, "Looking for Missed Alarm Bugs in
a Formal Verification Tool" (https://blog.regehr.org/archives/2124).  Their
second missed-alarm hunter was Zhengyang Liu's Minotaur superoptimiser: for
each LLVM instruction, slice out its backward dependencies as a
*specification*, enumerate cheaper partially-symbolic candidates, ask Alive2
(an exists-forall SMT query) whether symbolic constants exist that make the
candidate refine the spec, and ask llvm-mca whether the candidate is cheaper.

LCCC has no LLVM IR and no Alive2, so this tool plays the same roles with
mechanisms that are sound at small scale and need no solver:

  * Specification: a closed integer expression over 1-2 uint8 variables
    (a "slice"), evaluated *exhaustively* over all 256 / 65536 inputs.
    For two variables the *search* keys candidates by a 208-point sampled
    signature (full tables per candidate are intractable), but acceptance
    is still only ever by the exhaustive check: a signature can lose a
    winner, it can never bless a loser.  Exhaustion is a decision procedure at this width -- it is our Alive2:
    a candidate refines the spec iff their full truth tables agree.  There
    is no sampling and therefore no missed alarm inside the checked width.
  * Symbolic constants: candidate constant slots are solved by enumeration
    over a curated constant pool (the concrete exists-step), while the
    forall-step is the exhaustive table check above.
  * Cost model: a transparent, documented op-weight proxy (mul=3, shifts and
    logic=1, select=2; division excluded from candidates) -- our llvm-mca
    stand-in.  A proxy win is a *hypothesis*: before any rewrite ships in the
    compiler it must be re-verified at the target width (this tool re-checks
    finalists at 16/32/64 bits on edge+random vectors and says so) and
    re-measured with scripts/bench_kernels.py.  Nothing here is auto-applied
    to codegen; the output is a reviewable candidate table for peephole /
    strength-reduction work, in the spirit of Minotaur feeding LLVM patches.
  * Missed-alarm self-test: synthesis is run against deliberately *wrong*
    specs (negative controls).  Every inequivalent candidate must be
    rejected and the rejection count is reported -- if the checker ever
    accepted one, the self-test fails.  This is the blog post's central
    lesson applied to the tool itself: a verifier that is only ever shown
    true facts has never been tested.

Synthesis is bottom-up enumerative search with observational-equivalence
pruning (one representative per truth table, cheapest first), bounded by
--max-size.  Typical run: tens of thousands of candidates per spec, seconds.

Usage:
    scripts/slice_superopt.py --self-test
    scripts/slice_superopt.py --spec 'x*7' --spec '(x&0xf)*3' --json out.json
    scripts/slice_superopt.py            # built-in spec suite
"""
from __future__ import annotations

import argparse
import json
import random
import re
import sys

WIDTH = 8
MASK = (1 << WIDTH) - 1
# Op weights: rough x86-64 latency/throughput class.  Documented proxy only.
COST = {"var": 0, "const": 0, "~": 1, "-u": 1, "+": 1, "-": 1, "*": 3,
        "&": 1, "|": 1, "^": 1, "<<": 1, ">>": 1, "sel": 2, "<": 1}
CONST_POOL = (0, 1, 2, 3, 4, 5, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 255)

# AST: ("var", i) | ("const", v) | ("un", op, a) | ("bin", op, a, b) |
#      ("sel", c, a, b)


def ev(node, env, width: int, mask: int) -> int:
    k = node[0]
    if k == "var":
        return env[node[1]] & mask
    if k == "const":
        return node[1] & mask
    if k == "un":
        a = ev(node[2], env, width, mask)
        return (~a) & mask if node[1] == "~" else (-a) & mask
    if k == "sel":
        return ev(node[2], env, width, mask) if ev(node[1], env, width, mask) \
            else ev(node[3], env, width, mask)
    _, op, la, lb = node
    a, b = ev(la, env, width, mask), ev(lb, env, width, mask)
    if op == "+":
        return (a + b) & mask
    if op == "-":
        return (a - b) & mask
    if op == "*":
        return (a * b) & mask
    if op == "&":
        return a & b
    if op == "|":
        return a | b
    if op == "^":
        return a ^ b
    if op == "<<":
        return (a << (b & (width - 1))) & mask
    if op == ">>":
        return a >> (b & (width - 1))
    return 1 if a < b else 0  # "<" (unsigned)


def to_c(node, names=("x", "y")) -> str:
    k = node[0]
    if k == "var":
        return names[node[1]]
    if k == "const":
        return str(node[1])
    if k == "un":
        op = "~" if node[1] == "~" else "-"
        return f"({op}{to_c(node[2], names)})"
    if k == "sel":
        return f"({to_c(node[1], names)} ? {to_c(node[2], names)} : {to_c(node[3], names)})"
    return f"({to_c(node[2], names)} {node[1]} {to_c(node[3], names)})"


def cost(node) -> int:
    k = node[0]
    if k in ("var", "const"):
        return 0
    if k == "un":
        return COST[node[1]] + cost(node[2])
    if k == "sel":
        return COST["sel"] + cost(node[1]) + cost(node[2]) + cost(node[3])
    return COST[node[1]] + cost(node[2]) + cost(node[3])


def size(node) -> int:
    k = node[0]
    if k in ("var", "const"):
        return 1
    if k == "un":
        return 1 + size(node[2])
    if k == "sel":
        return 1 + size(node[1]) + size(node[2]) + size(node[3])
    return 1 + size(node[2]) + size(node[3])


# ---------------------------------------------------------------------------
# Tiny spec parser: x, y, decimal/hex constants, + - * & | ^ << >> < ~ ( ) ?:
# ---------------------------------------------------------------------------

_TOKEN = re.compile(r"\s*(?:(0x[0-9a-fA-F]+|\d+)|([xy])|(<<|>>|[+\-*&|^<~?:()]))")


def parse_spec(text: str):
    toks: list[str] = []
    pos = 0
    while pos < len(text):
        m = _TOKEN.match(text, pos)
        if not m:
            raise ValueError(f"cannot tokenise spec at {text[pos:]!r}")
        toks.append(next(g for g in m.groups() if g is not None))
        pos = m.end()
    stream = iter(toks + [")_end"])
    state = {"tok": next(stream)}

    def peek():
        return state["tok"]

    def take(want=None):
        t = state["tok"]
        if want and t != want:
            raise ValueError(f"expected {want!r}, got {t!r} in spec {text!r}")
        state["tok"] = next(stream)
        return t

    def atom():
        t = peek()
        if t == "(":
            take("(")
            node = ternary()
            take(")")
            return node
        if t == "~":
            take()
            return ("un", "~", atom())
        if t == "-":
            take()
            return ("un", "-u", atom())
        take()
        if t in ("x", "y"):
            return ("var", 0 if t == "x" else 1)
        # Specs are uint8 semantics throughout: mask literals at parse time
        # so the AST, the printed form and the truth table all agree on the
        # constant (an unmasked 300 used to print as 300 but evaluate as 44).
        return ("const", int(t, 0) & MASK)

    def binary(sub, ops):
        node = sub()
        while peek() in ops:
            op = take()
            node = ("bin", op, node, sub())
        return node

    def ternary():
        # Proper C precedence tiers (tightest last in this nesting):
        #   *  >  + -  >  << >>  >  <  >  &  >  ^  >  |
        # The original parser put <<,>>,&,|,^,< in ONE tier, so an
        # unparenthesised spec like "x|y&3" silently meant ((x|y)&3) here
        # while C reads x|(y&3).  Output was always safe (to_c fully
        # parenthesises), but input specs meant something else than C.
        mul = lambda: binary(atom, ("*",))  # noqa: E731
        add = lambda: binary(mul, ("+", "-"))  # noqa: E731
        shift = lambda: binary(add, ("<<", ">>"))  # noqa: E731
        rel = lambda: binary(shift, ("<",))  # noqa: E731
        band = lambda: binary(rel, ("&",))  # noqa: E731
        bxor = lambda: binary(band, ("^",))  # noqa: E731
        node = binary(bxor, ("|",))
        if peek() == "?":
            take("?")
            a = ternary()
            take(":")
            node = ("sel", node, a, ternary())
        return node

    node = ternary()
    if peek() != ")_end":
        raise ValueError(f"trailing tokens in spec {text!r}")
    return node


def has_var_shift(node) -> bool:
    """True if any shift has a non-constant count.

    ev()/truth tables mask shift counts (``b & (width-1)``) -- hardware
    semantics.  C leaves shifts by >= width undefined, so for a spec with
    a variable shift count an 8-bit "proof" is a statement about the
    masked-shift model, not about C.  Candidate enumeration already
    restricts counts to constant slots; this flags specs whose own
    semantics need that caveat before anyone quotes a result as a C
    rewrite.  (Red-team honesty fix: the caveat used to be nowhere.)
    """
    k = node[0]
    if k == "un":
        return has_var_shift(node[2])
    if k == "sel":
        return any(has_var_shift(node[i]) for i in (1, 2, 3))
    if k != "bin":
        return False
    if node[1] in ("<<", ">>") and node[3][0] != "const":
        return True
    return has_var_shift(node[2]) or has_var_shift(node[3])


def nvars(node) -> int:
    k = node[0]
    if k == "var":
        return node[1] + 1
    if k == "const":
        return 0
    if k == "un":
        return nvars(node[2])
    if k == "sel":
        return max(nvars(node[1]), nvars(node[2]), nvars(node[3]))
    return max(nvars(node[2]), nvars(node[3]))


# ---------------------------------------------------------------------------
# Exhaustive truth tables + bottom-up synthesis
# ---------------------------------------------------------------------------

def truth_table(node, nv: int) -> bytes:
    if nv == 1:
        return bytes(ev(node, (x, 0), WIDTH, MASK) for x in range(256))
    out = bytearray()
    for x in range(256):
        for y in range(256):
            out.append(ev(node, (x, y), WIDTH, MASK))
    return bytes(out)


def variants(node):
    """Cheap theoretical rewrites for when a spec has no enumerated cheaper solution.

    Returns candidate expressions *equivalent by construction* for masked
    unsigned arithmetic.  They are used only as hints and still must pass the
    exhaustive truth-table check before they are reported.
    """
    k = node[0]
    if k != "bin":
        return
    op, a, b = node[1], node[2], node[3]

    def const(node):
        # Mask defensively: the identities below do arithmetic on the
        # literal itself (c - 2^k), which is meaningless for an unmasked
        # value even though ev() would mask at evaluation time.
        return node[1] & MASK if node[0] == "const" else None

    if op == "*":
        c, other = const(a), b
        if c is None:
            c, other = const(b), a
        if c is not None:
            # x * (2^k + r) == (x << k) + x*r (and the subtraction identity).
            # Remaining factor is limited to a tiny constant so the emitted
            # expression remains reviewable and cheap in the cost model.
            for k in range(WIDTH):
                p = 1 << k
                r = c - p
                if 0 <= r <= 4:
                    cand = ("bin", "<<", other, ("const", k))
                    if r:
                        cand = ("bin", "+", cand,
                                other if r == 1 else ("bin", "*", other, ("const", r)))
                    yield cand
                r = p - c
                if 0 < r <= 4:
                    cand = ("bin", "<<", other, ("const", k))
                    cand = ("bin", "-", cand,
                            other if r == 1 else ("bin", "*", other, ("const", r)))
                    yield cand
    # (No "+" hint: the old a-(-b) rewrite costs 2 against the spec's 1,
    # so it could never pass the cheaper-than gate -- dead weight.)


_TT_CACHE: dict = {}

_COMBINE = {
    "+": lambda a, b: (a + b) & MASK,
    "-": lambda a, b: (a - b) & MASK,
    "*": lambda a, b: (a * b) & MASK,
    "&": lambda a, b: a & b,
    "|": lambda a, b: a | b,
    "^": lambda a, b: a ^ b,
    "<<": lambda a, b: (a << (b & (WIDTH - 1))) & MASK,
    ">>": lambda a, b: a >> (b & (WIDTH - 1)),
    "<": lambda a, b: 1 if a < b else 0,
}


def tt_of(node, nv: int) -> bytes:
    """Truth table, built algebraically from cached child tables.

    Combining two 256-entry child tables with one zip-comprehension is far
    cheaper than re-evaluating the whole AST per input, which is what makes
    exhaustively-verified enumeration tractable in pure Python at all.
    """
    key = (nv, node)
    tt = _TT_CACHE.get(key)
    if tt is not None:
        return tt
    if len(_TT_CACHE) > 400_000:
        _TT_CACHE.clear()
    k = node[0]
    if k in ("var", "const"):
        tt = truth_table(node, nv)
    elif k == "un":
        ta = tt_of(node[2], nv)
        tt = bytes((~a) & MASK for a in ta) if node[1] == "~" \
            else bytes((-a) & MASK for a in ta)
    elif k == "sel":
        tc, ta, tb = tt_of(node[1], nv), tt_of(node[2], nv), tt_of(node[3], nv)
        tt = bytes(x if c else z for c, x, z in zip(tc, ta, tb))
    else:
        f = _COMBINE[node[1]]
        ta, tb = tt_of(node[2], nv), tt_of(node[3], nv)
        tt = bytes(f(a, b) for a, b in zip(ta, tb))
    _TT_CACHE[key] = tt
    return tt


# --- Sampled signatures for 2-variable synthesis (F-3) -------------------
# Full 2-var truth tables have 65536 entries; building one per candidate is
# what made 2-var enumeration intractable.  Instead, candidates are keyed
# by a SIGNATURE: their outputs on a fixed, deterministic point set (full
# cross-product of 12 edge values + 64 seeded random pairs = 208 points).
# Signatures are combined algebraically from child signatures exactly like
# truth tables, at 1/315 the cost.  Soundness is untouched: a signature
# match against the spec is only ever a *candidate* -- acceptance still
# requires the exhaustive 65536-point truth-table proof.  The price is
# completeness, not correctness: two functions that differ only outside
# the point set share a signature, so observational pruning may discard a
# representative that would have led to a winner.  That trade (Minotaur
# makes the analogous one with its sampled "interesting values") is
# reported in stats, never hidden.
def _make_fp_points() -> tuple:
    edge = (0, 1, 2, 3, 7, 15, 16, 31, 63, 127, 128, 255)
    pts = [(x, y) for x in edge for y in edge]
    rng = random.Random(0xF12E)
    pts += [(rng.randrange(256), rng.randrange(256)) for _ in range(64)]
    return tuple(pts)


_FP_POINTS = _make_fp_points()
_FP_CACHE: dict = {}


def fp_of(node) -> bytes:
    """Sampled signature of a 2-variable expression (see block comment)."""
    tt = _FP_CACHE.get(node)
    if tt is not None:
        return tt
    if len(_FP_CACHE) > 400_000:
        _FP_CACHE.clear()
    k = node[0]
    if k == "var":
        idx = node[1]
        tt = bytes(p[idx] for p in _FP_POINTS)
    elif k == "const":
        tt = bytes((node[1] & MASK,)) * len(_FP_POINTS)
    elif k == "un":
        ta = fp_of(node[2])
        tt = bytes((~a) & MASK for a in ta) if node[1] == "~" \
            else bytes((-a) & MASK for a in ta)
    elif k == "sel":
        tc, ta, tb = fp_of(node[1]), fp_of(node[2]), fp_of(node[3])
        tt = bytes(x if c else z for c, x, z in zip(tc, ta, tb))
    else:
        f = _COMBINE[node[1]]
        ta, tb = fp_of(node[2]), fp_of(node[3])
        tt = bytes(f(a, b) for a, b in zip(ta, tb))
    _FP_CACHE[node] = tt
    return tt


def synthesise(spec, max_size: int = 9, max_considered: int = 400_000):
    """Return (best_cheaper_candidate | None, stats).

    Enumeration is bounded by COST, not raw size: only expressions strictly
    cheaper than the spec can ever win (Minotaur's "find a cheaper way"
    step), and every subexpression of a cheaper expression is itself
    cheaper, so cost-bounding loses no winner while keeping the search
    small.  Observational-equivalence pruning keeps one representative per
    truth table.  (`max_size` remains as a hard structural cap.)
    """
    nv = max(1, nvars(spec))
    sampled = nv > 1
    # 1-var: keys are full truth tables.  2-var: keys are sampled
    # signatures and spec_tt (computed once) is the exhaustive reference
    # every signature hit must still match before it counts as proven.
    spec_tt = tt_of(spec, nv) if sampled else None
    target = fp_of(spec) if sampled else tt_of(spec, nv)
    spec_cost = cost(spec)
    budget = spec_cost - 1
    reps: dict[bytes, object] = {}
    by_cost: dict[int, list] = {0: []}
    considered = rejected = fp_false_hits = 0

    def offer(node, node_cost: int) -> bool:
        """Register a candidate; return True iff it PROVABLY equals spec."""
        nonlocal considered, rejected, fp_false_hits
        considered += 1
        if size(node) > max_size:
            return False
        key = fp_of(node) if sampled else tt_of(node, nv)
        proven = False
        if key == target:
            if not sampled:
                proven = True
            elif tt_of(node, nv) == spec_tt:
                proven = True
            else:
                # Signature collision: same outputs on the 208 sampled
                # points, different function.  Counted, never accepted.
                fp_false_hits += 1
        if key in reps:
            if not proven:
                rejected += 1
            return proven
        reps[key] = node
        by_cost.setdefault(node_cost, []).append(node)
        return proven

    for i in range(nv):
        offer(("var", i), 0)
    for c in CONST_POOL:
        offer(("const", c), 0)

    best = None
    aborted = False
    binops = ("+", "-", "*", "&", "|", "^", "<<", ">>")
    # Two-variable search keys candidates by sampled signature (F-3; see
    # fp_of): enumeration is complete only up to signature collisions, but
    # every accepted candidate is exhaustively proven, so an answer here is
    # sound and a non-answer says "not found", never "does not exist".
    cost_levels = range(1, budget + 1)
    for total in cost_levels:
        # unary (weight 1)
        for child in list(by_cost.get(total - 1, [])):
            for op in ("~", "-u"):
                cand = ("un", op, child)
                if offer(cand, total) and best is None:
                    best = cand
        # binary
        for op in binops:
            rem = total - COST[op]
            if rem < 0:
                continue
            for ca in range(rem + 1):
                cb = rem - ca
                for a in by_cost.get(ca, []):
                    for b in by_cost.get(cb, []):
                        if op in ("<<", ">>") and b[0] != "const":
                            continue  # shift counts: symbolic-constant slots only
                        cand = ("bin", op, a, b)
                        if offer(cand, total) and best is None:
                            best = cand
        if best is not None:
            break  # cheapest-first enumeration: first hit at this cost wins
        if considered > max_considered:
            # Honest bound: report "no candidate found within budget", never
            # "no candidate exists".  Stats carry the flag for the JSON log.
            aborted = True
            break

    # A few canonical forms sit outside the cost-bounded structural cap.
    # Theoretical variants are tried as hints, but accepted only after the
    # same exhaustive truth-table proof as every enumerated candidate.
    if best is None:
        for cand in variants(spec):
            considered += 1
            if cost(cand) < spec_cost and tt_of(cand, nv) == (spec_tt if sampled else target):
                if best is None or cost(cand) < cost(best):
                    best = cand
            else:
                rejected += 1
    stats = {"considered": considered, "pruned_equivalent": rejected,
             "spec_cost": spec_cost, "candidate_cost": cost(best) if best else None,
             "nvars": nv, "exhaustive_inputs": 256 if nv == 1 else 65536,
             "search_aborted": aborted, "sampled_signature": sampled,
             "signature_points": len(_FP_POINTS) if sampled else None,
             "signature_false_hits": fp_false_hits if sampled else 0}
    return best, stats


def verify_wider(spec, cand, rng: random.Random) -> bool:
    """Re-check an 8-bit-proven rewrite at 16/32/64 bits (edge+random).

    Exhaustion proves the 8-bit statement only.  Most wraparound identities
    lift to all widths, but shift/select corner cases need not -- so any
    candidate destined for the compiler must pass this gate too, and then a
    bench_kernels measurement.  Constants are re-masked per width, which is
    exactly how the C rewrite would behave.
    """
    nv = max(1, nvars(spec))
    for width in (16, 32, 64):
        mask = (1 << width) - 1
        edge = [0, 1, 2, 3, 7, 31, 32, 255, 256, mask, mask >> 1, (mask >> 1) + 1]
        vecs = [(a, b) for a in edge for b in edge[:4]] if nv == 2 else [(a, 0) for a in edge]
        vecs += [tuple(rng.getrandbits(width) for _ in range(nv)) + (0,) * (2 - nv)
                 for _ in range(64)]
        for v in vecs:
            if ev(spec, v, width, mask) != ev(cand, v, width, mask):
                return False
    return True


BUILTIN_SPECS = ["x*3", "x*5", "x*7", "x*9", "x*10", "x*15", "(x&15)*3", "x+x",
                 "(x<<1)+x", "(x^y)+((x&y)<<1)", "x<y?x:y", "~x+1"]


def run_specs(specs: list[str], max_size: int) -> list[dict]:
    rng = random.Random(0xC0FFEE)
    rows = []
    for text in specs:
        spec = parse_spec(text)
        best, stats = synthesise(spec, max_size)
        row = {"spec": text, "spec_c": to_c(spec), **stats, "candidate": None,
               "verified_8bit": best is not None, "verified_wider": False,
               "masked_shift_semantics": has_var_shift(spec)}
        if row["masked_shift_semantics"]:
            print(f"warning: spec {text!r} has a variable shift count; "
                  "proofs below are for masked-shift (hardware) semantics, "
                  "not C -- do not ship as a C rewrite without a count "
                  "bound proof", file=sys.stderr)
        if best is not None:
            row["candidate"] = to_c(best)
            row["verified_wider"] = verify_wider(spec, best, rng)
        rows.append(row)
        verdict = (f"-> {row['candidate']}  (cost {stats['spec_cost']} -> "
                   f"{stats['candidate_cost']}, wider={'OK' if row['verified_wider'] else 'FAIL'})"
                   if best else f"(no cheaper candidate <= size {max_size}; cost {stats['spec_cost']})")
        print(f"{text:24s} {verdict}  [considered={stats['considered']} "
              f"pruned={stats['pruned_equivalent']}]")
    return rows


def self_test() -> int:
    problems: list[str] = []
    # Positive controls: known strength reductions must be found + proven.
    # Under the documented cost proxy: x*3 -> (x<<1)+x (2<3), x*5 -> (x<<2)+x,
    # x*7 -> (x<<3)-x.  (x*10 has no strictly-cheaper form in this proxy --
    # (x<<3)+(x<<1) ties at 3 -- which is itself a useful negative result.)
    expect = {"x*3": True, "x*5": True, "x*7": True, "x*9": True}
    for text, want_found in expect.items():
        best, _ = synthesise(parse_spec(text))
        if (best is not None) != want_found:
            problems.append(f"{text}: expected found={want_found}, got {best and to_c(best)}")
        if best is not None and not verify_wider(parse_spec(text), best, random.Random(1)):
            problems.append(f"{text}: candidate fails wider-width gate")
    # Negative controls (missed-alarm hunt): inequivalent pairs must NEVER be
    # accepted as refinements.  Directly ask the checker the wrong question.
    wrong = [("x*3", "x*4"), ("x+y", "x*y"), ("x&15", "x&7"), ("x<<1", "x<<2"),
             ("x<y?x:y", "x<y?y:x")]
    for spec_text, bad_text in wrong:
        spec, bad = parse_spec(spec_text), parse_spec(bad_text)
        nv = max(1, nvars(spec), nvars(bad))
        if truth_table(spec, nv) == truth_table(bad, nv):
            problems.append(f"MISSED ALARM: {bad_text} accepted as refinement of {spec_text}")
    # Equivalent control must be recognised (checker is not trivially all-no).
    if truth_table(parse_spec("x+x"), 1) != truth_table(parse_spec("x<<1"), 1):
        problems.append("false alarm: x+x vs x<<1 tables differ")
    # F-3 positive controls: 2-var sampled-signature search must find and
    # exhaustively prove the adder identities (full-table comparison here,
    # not the signature).
    for text in ("(x&y)+(x|y)", "(x^y)+((x&y)<<1)"):
        spec = parse_spec(text)
        best, stats = synthesise(spec)
        if best is None or tt_of(best, 2) != tt_of(spec, 2):
            problems.append(f"{text}: 2-var synthesis failed or unproven")
        elif stats["search_aborted"] or not stats["sampled_signature"]:
            problems.append(f"{text}: expected live sampled-signature search, got {stats}")
    # Parser precedence: unparenthesised input must follow C tiers.
    if to_c(parse_spec("x|y&3")) != "(x | (y & 3))":
        problems.append("parser precedence: x|y&3 is not x|(y&3)")
    # Parser round-trip on the builtin suite.
    for text in BUILTIN_SPECS:
        parse_spec(text)
    print("slice_superopt self-test: " + ("FAIL" if problems else "PASS"))
    for p in problems:
        print(f"  FAIL: {p}")
    return 1 if problems else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--spec", action="append", default=[], help="repeatable; default: builtin suite")
    ap.add_argument("--max-size", type=int, default=7)
    ap.add_argument("--json", dest="json_out")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    rows = run_specs(args.spec or BUILTIN_SPECS, args.max_size)
    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as f:
            json.dump({"candidates": rows}, f, indent=2)
        print(f"wrote {args.json_out}")
    bad = [r for r in rows if r["candidate"] and not r["verified_wider"]]
    if bad:
        print(f"warning: {len(bad)} candidate(s) failed the wider-width gate; "
              "do not ship them", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
