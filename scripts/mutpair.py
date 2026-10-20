#!/usr/bin/env python3
"""Missed-alarm differential testing for LCCC (Regehr/Livinskii transplant).

Source of the method: Regehr & Livinskii, "Looking for Missed Alarm Bugs in a
Formal Verification Tool" (Embedded in Academia, 2024-09-04,
https://blog.regehr.org/archives/2124).  To test Alive2 -- a tool whose job is
to *alarm* when an optimisation is wrong -- they needed test cases that are
*known* to be inequivalent: take a YARPGen function (UB-free by construction),
mutate it slightly while preserving UB-freedom, keep only pairs whose
observable behaviour provably differs on some input (execution filter), and
then require the checker to alarm.  Silence on such a pair is a missed alarm.

This module transplants that idea from a translation validator to a compiler.
The "checker" under test is LCCC's optimiser, and its missed alarm is the
dual failure: treating two observably different computations as if they were
the same (unsound folding / GVN / CSE / reassociation), or breaking two
computations that must stay equal.  Plain differential testing (fuzz_diff.py)
compares compilers on *one* program at a time; a near-miss *pair* in a single
translation unit is far more sensitive per case, because the one-token
difference must survive every canonicalisation the optimiser performs on both
neighbours simultaneously, with both results observable side by side.

Method, step by step (mirroring the blog post):

  1. Generate a random integer function that is UB-free *by construction*:
     uint32_t arithmetic only (wraparound is defined), shift counts masked
     with & 31, division/modulo only by a provably-non-zero constant, no
     signed overflow.  The expression core is wrapped in one of four
     program shapes (straight-line, loop reduction, local-array memory
     traffic, helper call) and decorated with GNU function attributes --
     every shape keeps the same UB-freedom invariants (masked array
     indices, no signed arithmetic) and has a bit-exact model below.
  2. Mutate it slightly, preserving UB-freedom, in one of two polarities:
       * inequivalent-candidate mutations (constant tweak, operator swap,
         operand swap, variable swap, subexpression negation), and
       * equivalent-by-construction mutations (identity rewrites that are
         sound for unsigned wraparound arithmetic: x+0, x*1, ~~x, operand
         commutation of commutative operators, x*2 -> x<<1, De Morgan, ...).
  3. Execution filter: a bit-exact Python model of the uint32_t semantics
     evaluates both versions on an edge-heavy input vector set.  An
     "inequivalent" pair is kept only if the outputs provably differ on at
     least one judged input (a proof of difference); an "equivalent" pair
     is equivalent *by construction* (identity rewrites only), and the
     filter's agreement check on the judged vectors is a belt-and-braces
     sampled cross-check of that construction, not the source of the
     guarantee.  Discarded pairs are counted, never judged.  Both sides
     additionally pass validate_ub_free, a structural audit of the two
     UB-freedom invariants the model itself cannot see (masked shift
     counts, non-zero constant divisors).
  4. Emit ONE C program containing both functions; main() prints both result
     columns.  Every compiler must reproduce the model columns exactly.  In
     fuzz_diff.py (`--engine mutpair`) the reference compilers are the oracle;
     in standalone mode the Python model itself is the oracle, so LCCC can be
     judged even if every reference shared the same bug.
  5. Negative control (`--self-test`): a deliberately broken textual
     "optimiser" (the fake_miscompiler trick: rewrite `* 3u` to `* 4u`) MUST
     be caught.  A harness that cannot catch a known-bad compiler has a
     missed-alarm hole of its own -- exactly the blog post's thesis applied
     to this harness itself.

Usage:
    scripts/mutpair.py --self-test                 # model/mutation invariants
    scripts/mutpair.py --count 200 --seed 1        # standalone vs lccc+gcc
    scripts/fuzz_diff.py --engine mutpair --count 200 --seed 1

Exit status is non-zero iff a judged pair diverges or the negative control
escapes.  Reproducers land in --repro-dir (default artifacts/repros).
"""
from __future__ import annotations

import argparse
import copy
import json
import random
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
M32 = 0xFFFFFFFF

# AST nodes are tuples: ("var", i) | ("const", v) | ("un", op, a) |
# ("bin", op, a, b) | ("sel", c, a, b)  (sel = c ? a : b with c a 0/1 value).
BINOPS = ("+", "-", "*", "&", "|", "^", "<<", ">>", "%", "==", "<", "<=")
COMMUTATIVE = frozenset(("+", "*", "&", "|", "^", "=="))
UNOPS = ("~", "-", "!")
NVARS = 3

# Edge-heavy input vectors: every value that historically breaks shift,
# division-by-constant, sign-extension and boundary folding, in a small
# cross-product.  Kept tiny (24 vectors) so a full sweep is microseconds.
_EDGE = (0, 1, 2, 3, 7, 16, 31, 32, 33, 63, 255, 256, 1000, 65535, 65536,
         0x7FFFFFFF, 0x80000000, 0x80000001, 0xFFFFFFFE, 0xFFFFFFFF,
         0x55555555, 0xAAAAAAAA, 0x12345678, 0x9E3779B9)


def input_vectors(rng: random.Random, randoms: int = 24) -> list[tuple[int, ...]]:
    vecs: list[tuple[int, ...]] = []
    for i, v in enumerate(_EDGE):
        vecs.append((v, _EDGE[(i * 7 + 3) % len(_EDGE)], _EDGE[(i * 13 + 5) % len(_EDGE)]))
    for _ in range(randoms):
        vecs.append(tuple(rng.getrandbits(32) for _ in range(NVARS)))
    return vecs


# ---------------------------------------------------------------------------
# Bit-exact uint32_t model (the execution filter / standalone oracle)
# ---------------------------------------------------------------------------

def evaluate(node, env: tuple[int, ...]) -> int:
    kind = node[0]
    if kind == "var":
        return env[node[1]] & M32
    if kind == "const":
        return node[1] & M32
    if kind == "un":
        a = evaluate(node[2], env)
        op = node[1]
        if op == "~":
            return (~a) & M32
        if op == "-":
            return (-a) & M32
        return 0 if a else 1  # "!"
    if kind == "sel":
        return evaluate(node[2], env) if evaluate(node[1], env) else evaluate(node[3], env)
    # bin
    _, op, lhs, rhs = node
    a, b = evaluate(lhs, env), evaluate(rhs, env)
    if op == "+":
        return (a + b) & M32
    if op == "-":
        return (a - b) & M32
    if op == "*":
        return (a * b) & M32
    if op == "&":
        return a & b
    if op == "|":
        return a | b
    if op == "^":
        return a ^ b
    if op == "<<":
        return (a << (b & 31)) & M32
    if op == ">>":
        return a >> (b & 31)
    if op == "%":
        # Generator guarantees rhs is a non-zero constant expression.  Say
        # so loudly if a future mutation ever breaks that invariant: a
        # ZeroDivisionError with context beats a bare crash, and silently
        # modelling UB would be worse than either.
        if b == 0:
            raise AssertionError(f"mutpair invariant broken: %-by-zero in {node!r}")
        return a % b
    if op == "==":
        return 1 if a == b else 0
    if op == "<":
        return 1 if a < b else 0
    return 1 if a <= b else 0  # "<="


def to_c(node, names=("v0", "v1", "v2")) -> str:
    kind = node[0]
    if kind == "var":
        return names[node[1]]
    if kind == "const":
        return f"{node[1] & M32}u"
    if kind == "un":
        return f"({node[1]}{to_c(node[2], names)})"
    if kind == "sel":
        return f"({to_c(node[1], names)} ? {to_c(node[2], names)} : {to_c(node[3], names)})"
    return f"({to_c(node[2], names)} {node[1]} {to_c(node[3], names)})"


# ---------------------------------------------------------------------------
# UB-free-by-construction generator
# ---------------------------------------------------------------------------

class PairGenerator:
    def __init__(self, seed: int):
        self.rng = random.Random(seed)
        self.seed = seed

    def _const(self) -> int:
        r = self.rng
        return r.choice((0, 1, 2, 3, 4, 5, 7, 8, 15, 16, 31, 32, 63, 127, 255,
                         256, 1000, 65535, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF,
                         r.getrandbits(32), r.getrandbits(8)))

    def expr(self, depth: int, nvars: int = NVARS):
        r = self.rng
        if depth <= 0 or r.random() < 0.30:
            if r.random() < 0.55:
                return ("var", r.randrange(nvars))
            return ("const", self._const())
        roll = r.random()
        if roll < 0.15:
            return ("un", r.choice(UNOPS), self.expr(depth - 1, nvars))
        if roll < 0.25:
            cond = ("bin", r.choice(("==", "<", "<=")),
                    self.expr(1, nvars), self.expr(1, nvars))
            return ("sel", cond, self.expr(depth - 1, nvars), self.expr(depth - 1, nvars))
        op = r.choice(BINOPS)
        lhs = self.expr(depth - 1, nvars)
        if op == "%":
            # Divisor: non-zero constant only -> no UB, ever.
            rhs = ("const", r.choice((2, 3, 5, 7, 10, 16, 100, 255, 1000, 65521)))
        elif op in ("<<", ">>"):
            # Mask the count in the emitted source shape itself.
            rhs = ("bin", "&", self.expr(1, nvars), ("const", 31))
        else:
            rhs = self.expr(depth - 1, nvars)
        return ("bin", op, lhs, rhs)

    def function(self, nvars: int = NVARS):
        return self.expr(self.rng.randint(3, 5), nvars)


# ---------------------------------------------------------------------------
# Mutations (both polarities preserve UB-freedom by construction)
# ---------------------------------------------------------------------------

def _paths(node, prefix=()):
    yield prefix, node
    kind = node[0]
    if kind == "un":
        yield from _paths(node[2], prefix + (2,))
    elif kind == "bin":
        yield from _paths(node[2], prefix + (2,))
        yield from _paths(node[3], prefix + (3,))
    elif kind == "sel":
        for i in (1, 2, 3):
            yield from _paths(node[i], prefix + (i,))


def _replace(node, path, new):
    if not path:
        return new
    items = list(node)
    items[path[0]] = _replace(items[path[0]], path[1:], new)
    return tuple(items)


def _is_shift_count(tree, path) -> bool:
    """True if `path` addresses (part of) a shift's masked count operand."""
    node = tree
    for step in path:
        if node[0] == "bin" and node[1] in ("<<", ">>") and step == 3:
            return True
        node = node[step]
    return False


def mutate_inequivalent(tree, rng: random.Random, nvars: int = NVARS):
    """One small, UB-preserving edit that *usually* changes the value.

    The execution filter has the final say; this only has to stay UB-free.
    Shift-count subtrees are never edited (the & 31 mask is load-bearing),
    and "%" divisors stay non-zero constants.
    """
    spots = [(p, n) for p, n in _paths(tree) if not _is_shift_count(tree, p)]
    if not spots:
        return tree
    path, node = rng.choice(spots)
    kind = node[0]
    if kind == "const":
        v = node[1]
        new = ("const", rng.choice(((v + 1) & M32, (v - 1) & M32, v ^ 1,
                                     (v ^ (1 << rng.randrange(32))) & M32)))
        # A "%" divisor must stay a non-zero constant.
        parent = tree
        for step in path[:-1]:
            parent = parent[step]
        if len(path) and parent[0] == "bin" and parent[1] == "%" and path[-1] == 3:
            new = ("const", rng.choice((2, 3, 7, 11, 100, 65521)))
        return _replace(tree, path, new)
    if kind == "var":
        return _replace(tree, path, ("var", rng.randrange(nvars)))
    if kind == "un":
        return _replace(tree, path, ("un", rng.choice(UNOPS), node[2]))
    if kind == "bin":
        swap = {"+": "-", "-": "+", "&": "|", "|": "&", "<<": ">>", ">>": "<<",
                "*": "+", "^": "&", "==": "<", "<": "<=", "<=": "<", "%": "&"}
        # Operand swap is UB-preserving for -, <, <= only.  Swapping a
        # shift's operands would move the unmasked value into the count
        # position: the model masks counts (& 31) but C does not, so the
        # mutant would have UB the model cannot see.  (Red-team find: the
        # original swap produced an unmasked count in ~1% of pairs; the
        # model-level unit test could not observe it because the model
        # itself masks.  validate_ub_free below now polices this
        # structurally on every emitted pair.)
        if rng.random() < 0.5 and node[1] not in COMMUTATIVE \
                and node[1] not in ("%", "<<", ">>"):
            return _replace(tree, path, ("bin", node[1], node[3], node[2]))
        return _replace(tree, path, ("bin", swap[node[1]], node[2], node[3]))
    if kind == "sel":
        return _replace(tree, path, ("sel", node[1], node[3], node[2]))
    return tree


def _has_var(node) -> bool:
    if not isinstance(node, tuple):
        return False
    if node[0] == "var":
        return True
    return any(_has_var(x) for x in node[1:] if isinstance(x, tuple))


def validate_ub_free(node, what: str = "tree") -> None:
    """Structural UB-freedom audit of one AST.  Raises AssertionError.

    The generator/mutators argue UB-freedom by construction; this checker
    *verifies* the two load-bearing invariants on the emitted artifact
    itself, so a future mutation that breaks them fails loudly here
    instead of manufacturing a false miscompile report downstream:

      * every shift count is exactly ``(<expr> & 31)`` -- the model masks
        counts, C does not, so anything else is model-invisible UB;
      * every ``%`` divisor is variable-free and folds to a non-zero
        constant.

    Everything else in the language (uint32 wraparound, masked array
    indices in the shape wrappers, no signed arithmetic) is UB-free for
    all operand values, so these two checks are complete.
    """
    kind = node[0]
    if kind == "un":
        validate_ub_free(node[2], what)
        return
    if kind == "sel":
        for i in (1, 2, 3):
            validate_ub_free(node[i], what)
        return
    if kind != "bin":
        return
    _, op, lhs, rhs = node
    if op in ("<<", ">>"):
        ok = rhs[0] == "bin" and rhs[1] == "&" and rhs[3] == ("const", 31)
        if not ok:
            raise AssertionError(
                f"mutpair UB-freedom broken in {what}: unmasked shift "
                f"count in {node!r}")
    if op == "%":
        if _has_var(rhs) or evaluate(rhs, (0, 0, 0)) == 0:
            raise AssertionError(
                f"mutpair UB-freedom broken in {what}: %-divisor is not "
                f"a non-zero constant in {node!r}")
    validate_ub_free(lhs, what)
    validate_ub_free(rhs, what)


def validate_program_ub_free(prog: dict, what: str) -> None:
    for key in ("body", "body2", "helper"):
        if prog.get(key) is not None:
            validate_ub_free(prog[key], f"{what}.{key}")


def mutate_equivalent(tree, rng: random.Random):
    """One identity rewrite, sound for unsigned wraparound arithmetic."""
    spots = [(p, n) for p, n in _paths(tree) if not _is_shift_count(tree, p)]
    if not spots:
        return tree
    path, node = rng.choice(spots)
    kind = rng.randrange(8)
    if kind == 0:
        new = ("bin", "+", node, ("const", 0))
    elif kind == 1:
        new = ("bin", "*", node, ("const", 1))
    elif kind == 2:
        new = ("un", "~", ("un", "~", node))
    elif kind == 3:
        new = ("un", "-", ("un", "-", node))
    elif kind == 4:
        new = ("bin", "^", node, ("const", 0))
    elif kind == 5 and node[0] == "bin" and node[1] in COMMUTATIVE:
        new = ("bin", node[1], node[3], node[2])
    elif kind == 6:
        new = ("bin", "|", node, ("const", 0))
    else:
        new = ("bin", "&", node, ("const", M32))
    return _replace(tree, path, new)


# ---------------------------------------------------------------------------
# Program shapes (F-2/F-5): the same expression core wrapped in the program
# shapes real optimisers actually transform -- loop reductions (LICM,
# induction-variable strength reduction, unrolling), local-array memory
# traffic (SROA, load/store forwarding, GVN through memory), and helper
# calls (inlining, interprocedural constant propagation) -- plus GNU
# function-attribute decoration of both pair members and any helper.
# Attributes are the blog post's suspected blind spot ("rarely-stressed
# constructs"), so they are an orthogonal per-pair dimension, identical for
# orig and mut: the pair still differs by exactly one mutation.
#
# A program is a plain dict:
#   shape : "expr" | "loop" | "mem" | "call"
#   body  : AST over v0..v2           (loop: v0 is the accumulator)
#   body2 : AST over v0..v2 | None    (mem value / call second argument)
#   helper: AST over x,y | None       (call shape only)
#   trip  : loop trip count           (loop shape only)
#   store : mem sub-variant flag      (mem shape only)
#   attrs : key into FN_ATTRS         (both pair members)
#   hattr : key into HELPER_ATTRS     (call shape only)
# Every shape is UB-free by the same construction as the expression core:
# unsigned wraparound, masked shifts, non-zero constant divisors, array
# indices masked & 7, no signed arithmetic anywhere.
# ---------------------------------------------------------------------------

FN_ATTRS = {
    "plain": "static",
    "noinline": "__attribute__((noinline)) static",
    "always_inline": "static inline __attribute__((always_inline))",
    "hot": "__attribute__((hot)) static",
    "cold": "__attribute__((cold)) static",
    "flatten": "__attribute__((flatten)) static",
}
HELPER_ATTRS = {
    "noinline": "__attribute__((noinline)) static",
    "const": "__attribute__((const, noinline)) static",
    "pure": "__attribute__((pure, noinline)) static",
    "always_inline": "static inline __attribute__((always_inline))",
}
_SHAPE_WEIGHTS = (("expr", 0.40), ("loop", 0.20), ("mem", 0.20), ("call", 0.20))


def build_program(gen: "PairGenerator") -> dict:
    r = gen.rng
    shape, acc = "expr", 0.0
    roll = r.random()
    for name, w in _SHAPE_WEIGHTS:
        acc += w
        if roll < acc:
            shape = name
            break
    prog = {"shape": shape, "body": gen.function(), "body2": None,
            "helper": None, "trip": r.choice((1, 3, 8, 13)),
            "store": bool(r.getrandbits(1)),
            "attrs": r.choice(tuple(FN_ATTRS)),
            "hattr": r.choice(tuple(HELPER_ATTRS))}
    if shape in ("mem", "call"):
        prog["body2"] = gen.expr(r.randint(2, 4))
    if shape == "call":
        prog["helper"] = gen.function(nvars=2)
    return prog


def _mem_cells(env: tuple[int, ...]) -> list[int]:
    v0, v1, v2 = (x & M32 for x in env)
    return [v0, v1, v2, (v0 ^ v1) & M32, (v1 + v2) & M32, (~v0) & M32,
            0x9E3779B9, (v2 << 1) & M32]


def model_program(prog: dict, env: tuple[int, ...]) -> int:
    """Bit-exact model of one program shape on one input vector."""
    shape = prog["shape"]
    if shape == "expr":
        return evaluate(prog["body"], env)
    if shape == "loop":
        acc = env[0] & M32
        for _ in range(prog["trip"]):
            acc = evaluate(prog["body"], (acc, env[1], env[2]))
        return acc
    if shape == "mem":
        cells = _mem_cells(env)
        idx = evaluate(prog["body"], env) & 7
        if prog["store"]:
            cells[idx] = evaluate(prog["body2"], env)
            return (cells[idx] ^ cells[3]) & M32
        return (cells[idx] + evaluate(prog["body2"], env)) & M32
    # call
    return evaluate(prog["helper"],
                    (evaluate(prog["body"], env), evaluate(prog["body2"], env), 0))


def mutate_program(prog: dict, rng: random.Random, polarity: str) -> dict:
    """Deep-copy a program and mutate one AST component of the copy."""
    mut = copy.deepcopy(prog)
    fn = mutate_inequivalent if polarity == "inequivalent" else mutate_equivalent
    parts = [("body", NVARS)]
    if mut["body2"] is not None:
        parts.append(("body2", NVARS))
    if mut["helper"] is not None:
        parts.append(("helper", 2))
    key, nvars = parts[rng.randrange(len(parts))] if len(parts) == 1 else \
        rng.choices(parts, weights=[2] + [1] * (len(parts) - 1))[0]
    mut[key] = fn(mut[key], rng, nvars) if fn is mutate_inequivalent \
        else fn(mut[key], rng)
    return mut


def _program_c(prog: dict, fname: str, helper_name: str) -> str:
    """C text for one side of a pair (helper, if any, then the function)."""
    shape = prog["shape"]
    prefix = FN_ATTRS[prog["attrs"]]
    out = ""
    if shape == "call":
        out += (f'{HELPER_ATTRS[prog["hattr"]]} uint32_t {helper_name}'
                f'(uint32_t x, uint32_t y) {{\n'
                f'    return {to_c(prog["helper"], ("x", "y"))};\n}}\n')
    head = f"{prefix} uint32_t {fname}(uint32_t v0, uint32_t v1, uint32_t v2) {{\n"
    if shape == "expr":
        body = f"    return {to_c(prog['body'])};\n"
    elif shape == "loop":
        body = ("    uint32_t acc = v0;\n"
                f"    for (unsigned i = 0; i < {prog['trip']}u; i++)\n"
                f"        acc = {to_c(prog['body'], ('acc', 'v1', 'v2'))};\n"
                "    return acc;\n")
    elif shape == "mem":
        init = "{ v0, v1, v2, v0 ^ v1, v1 + v2, ~v0, 2654435769u, v2 << 1 }"
        if prog["store"]:
            tail = (f"    a[idx] = {to_c(prog['body2'])};\n"
                    "    return a[idx] ^ a[3];\n")
        else:
            tail = f"    return a[idx] + ({to_c(prog['body2'])});\n"
        body = (f"    uint32_t a[8] = {init};\n"
                f"    uint32_t idx = ({to_c(prog['body'])}) & 7u;\n" + tail)
    else:  # call
        body = f"    return {helper_name}({to_c(prog['body'])}, {to_c(prog['body2'])});\n"
    return out + head + body + "}\n"


# ---------------------------------------------------------------------------
# Pair classification (execution filter) + C emission
# ---------------------------------------------------------------------------

@dataclass
class Pair:
    seed: int
    polarity: str  # "inequivalent" | "equivalent"
    orig: object   # body AST (kept for direct model probing in tests)
    mut: object    # body AST of the mutant
    vectors: list[tuple[int, ...]]
    expected_orig: list[int]
    expected_mut: list[int]
    # Full programs (shape wrappers around the body ASTs).  Defaulted so
    # hand-built expr-only pairs (negative control) need no ceremony:
    # pair_to_c synthesises the trivial expr program when these are None.
    prog_orig: dict | None = None
    prog_mut: dict | None = None
    shape: str = "expr"


def make_pair(seed: int) -> Pair | None:
    """Generate + mutate + execution-filter.  None = pair discarded."""
    gen = PairGenerator(seed)
    prog = build_program(gen)
    rng = random.Random(seed ^ 0x5EED5)
    polarity = "inequivalent" if rng.random() < 0.5 else "equivalent"
    mut_prog = mutate_program(prog, rng, polarity)
    # Verify (do not merely trust) UB-freedom of both sides before any
    # judging: a violation is a harness bug and must be loud, never a
    # silently discarded pair.
    validate_program_ub_free(prog, f"orig(seed={seed})")
    validate_program_ub_free(mut_prog, f"mut(seed={seed},{polarity})")
    if mut_prog == prog:
        return None
    vecs = input_vectors(random.Random(seed ^ 0x1CE5))
    eo = [model_program(prog, v) for v in vecs]
    em = [model_program(mut_prog, v) for v in vecs]
    differ = eo != em
    if polarity == "inequivalent" and not differ:
        return None  # mutation was semantically a no-op: discard (blog step 3)
    if polarity == "equivalent" and differ:
        return None  # identity rewrite mis-modelled somewhere: discard, loudly countable
    return Pair(seed, polarity, prog["body"], mut_prog["body"], vecs, eo, em,
                prog, mut_prog, prog["shape"])


def _as_program(body, prog: dict | None) -> dict:
    if prog is not None:
        return prog
    return {"shape": "expr", "body": body, "body2": None, "helper": None,
            "trip": 1, "store": False, "attrs": "plain", "hattr": "noinline"}


def pair_to_c(pair: Pair) -> str:
    vec = ",\n        ".join(
        "{" + ", ".join(f"{x}u" for x in v) + "}" for v in pair.vectors)
    po = _as_program(pair.orig, pair.prog_orig)
    pm = _as_program(pair.mut, pair.prog_mut)
    return f"""/* mutpair seed={pair.seed} polarity={pair.polarity} shape={po['shape']} attrs={po['attrs']} (UB-free by construction) */
#include <stdint.h>
#include <stdio.h>

{_program_c(po, "f_orig", "h_orig")}{_program_c(pm, "f_mut", "h_mut")}

static const uint32_t inputs[][3] = {{
        {vec}}};

int main(void) {{
    unsigned n = (unsigned)(sizeof(inputs) / sizeof(inputs[0]));
    for (unsigned i = 0; i < n; i++) {{
        printf("%08x %08x\\n",
               (unsigned)f_orig(inputs[i][0], inputs[i][1], inputs[i][2]),
               (unsigned)f_mut(inputs[i][0], inputs[i][1], inputs[i][2]));
    }}
    return 0;
}}
"""


def expected_stdout(pair: Pair) -> str:
    return "".join(f"{a:08x} {b:08x}\n"
                   for a, b in zip(pair.expected_orig, pair.expected_mut))


# ---------------------------------------------------------------------------
# Standalone driver + negative control
# ---------------------------------------------------------------------------

def _compile_run(cc: list[str], src: Path, opt: str, tmp: Path,
                 timeout: float = 30.0) -> tuple[int, str, str]:
    exe = tmp / "a.out"
    build = subprocess.run([*cc, opt, "-std=c99", "-w", str(src), "-o", str(exe)],
                           capture_output=True, text=True, timeout=timeout)
    if build.returncode != 0:
        return build.returncode, "", build.stderr
    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=timeout)
    return run.returncode, run.stdout, run.stderr


def judge_pair(pair: Pair, compilers: dict[str, list[str]], opts: list[str],
               repro_dir: Path) -> list[str]:
    """Return failure messages (empty = pair passed under every compiler)."""
    failures: list[str] = []
    want = expected_stdout(pair)
    src_text = pair_to_c(pair)
    with tempfile.TemporaryDirectory(prefix="mutpair_") as td:
        tmp = Path(td)
        src = tmp / "pair.c"
        for name, cc in compilers.items():
            for opt in opts:
                # Re-materialise the source for every invocation: a
                # "compiler" that rewrites its input in place (the fake
                # optimiser negative control does exactly this) must not
                # poison the next compiler/opt-level judged on this pair.
                src.write_text(src_text)
                try:
                    rc, out, err = _compile_run(cc, src, opt, tmp)
                except subprocess.TimeoutExpired:
                    failures.append(f"{name} {opt}: timeout")
                    continue
                if rc != 0:
                    failures.append(f"{name} {opt}: rc={rc} {err.strip()[:160]}")
                elif out != want:
                    flavour = ("collapsed an inequivalent pair (missed alarm)"
                               if pair.polarity == "inequivalent"
                               else "broke an equivalent pair")
                    failures.append(f"{name} {opt}: {flavour}")
        if failures:
            repro_dir.mkdir(parents=True, exist_ok=True)
            dest = repro_dir / f"mutpair_{pair.polarity}_{pair.seed}.c"
            dest.write_text(pair_to_c(pair))
            failures.append(f"repro: {dest}")
    return failures


FAKE_OPTIMISER = """#!/usr/bin/env python3
# Deliberately broken compiler (negative control): rewrites `* 3u` to `* 4u`.
import os, subprocess, sys
from pathlib import Path
args = sys.argv[1:]
src = next(a for a in args if a.endswith(".c"))
text = Path(src).read_text().replace("* 3u", "* 4u")
Path(src).write_text(text)
sys.exit(subprocess.call([os.environ.get("MUTPAIR_FAKE_CC", "gcc"), *args]))
"""


def self_test(verbose: bool = True) -> int:
    """Model/mutation invariants + negative control.  0 = harness is sound."""
    problems: list[str] = []
    kept = {"inequivalent": 0, "equivalent": 0}
    shapes: dict[str, int] = {}
    discarded = 0
    for seed in range(400):
        pair = make_pair(seed)
        if pair is None:
            discarded += 1
            continue
        kept[pair.polarity] += 1
        shapes[pair.shape] = shapes.get(pair.shape, 0) + 1
        if pair.polarity == "inequivalent" and pair.expected_orig == pair.expected_mut:
            problems.append(f"seed {seed}: inequivalent pair does not differ")
        if pair.polarity == "equivalent" and pair.expected_orig != pair.expected_mut:
            problems.append(f"seed {seed}: equivalent pair differs")
    if kept["inequivalent"] < 50 or kept["equivalent"] < 50:
        problems.append(f"execution filter starved: kept={kept} discarded={discarded}")
    for want in ("expr", "loop", "mem", "call"):
        if shapes.get(want, 0) < 10:
            problems.append(f"shape {want!r} starved: shapes={shapes}")

    # Model vs gcc cross-check (the oracle of the oracle) when gcc exists.
    gcc = shutil.which("gcc")
    checked = 0
    if gcc:
        repro = Path(tempfile.mkdtemp(prefix="mutpair_selftest_"))
        for seed in range(60):
            pair = make_pair(10_000 + seed)
            if pair is None:
                continue
            fails = judge_pair(pair, {"gcc-model-check": [gcc]}, ["-O0", "-O2"], repro)
            checked += 1
            if fails:
                problems.append(f"seed {pair.seed}: Python model disagrees with gcc: {fails[:1]}")
                break
        # Negative control: the fake optimiser MUST be caught.  Use a
        # hand-built pair whose result provably depends on `* 3u` (a random
        # pair's `* 3u` can sit in a dead/masked subtree, which would let the
        # control escape without the harness being at fault).
        caught = False
        with tempfile.TemporaryDirectory(prefix="mutpair_neg_") as td:
            fake = Path(td) / "fakecc.py"
            fake.write_text(FAKE_OPTIMISER)
            ctrl_orig = ("bin", "+", ("bin", "*", ("var", 0), ("const", 3)), ("var", 1))
            ctrl_mut = ("bin", "+", ctrl_orig, ("const", 0))
            vecs = input_vectors(random.Random(7))
            eo = [evaluate(ctrl_orig, v) for v in vecs]
            assert eo == [evaluate(ctrl_mut, v) for v in vecs]
            pair = Pair(-1, "equivalent", ctrl_orig, ctrl_mut, vecs, eo, eo)
            assert "* 3u" in pair_to_c(pair)
            caught = bool(judge_pair(pair, {"fake": [sys.executable, str(fake)]},
                                     ["-O2"], repro))
        if not caught:
            problems.append("negative control ESCAPED: fake optimiser was not caught "
                            "(harness has a missed-alarm hole)")
    else:
        problems.append("gcc not found: model cross-check + negative control skipped")

    if verbose:
        print(f"mutpair self-test: kept={kept} shapes={shapes} discarded={discarded} "
              f"gcc-cross-checked={checked}")
        for p in problems:
            print(f"  FAIL: {p}")
        print("mutpair self-test: " + ("FAIL" if problems else "PASS"))
    return 1 if problems else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--self-test", action="store_true", help="run invariants + negative control")
    ap.add_argument("--count", type=int, default=100, help="kept pairs to judge")
    ap.add_argument("--seed", type=int, default=0, help="first seed")
    ap.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    ap.add_argument("--refs", default="gcc", help="comma-separated reference compilers")
    ap.add_argument("--opts", default="-O0,-O1,-O2,-O3")
    ap.add_argument("--repro-dir", default=str(REPO / "artifacts/repros"))
    ap.add_argument("--json", dest="json_out")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    compilers: dict[str, list[str]] = {}
    if Path(args.lccc).exists():
        compilers["lccc"] = [args.lccc]
    else:
        # Never let a missing lccc masquerade as a pass: the whole point of
        # the standalone mode is judging lccc against the model.
        print(f"WARNING: lccc not found at {args.lccc}; judging references only "
              "(build lccc or pass --lccc)", file=sys.stderr)
    for ref in args.refs.split(","):
        ref = ref.strip()
        if ref and shutil.which(ref):
            compilers[ref] = [ref]
    if not compilers:
        print("error: no compiler available (build lccc or install gcc)", file=sys.stderr)
        return 2
    opts = [o.strip() for o in args.opts.split(",") if o.strip()]
    repro_dir = Path(args.repro_dir)

    judged = failures = discarded = 0
    seed = args.seed
    # Bounded: a starved execution filter must error out, not spin forever.
    max_attempts = args.count * 50 + 1000
    attempts = 0
    while judged < args.count:
        attempts += 1
        if attempts > max_attempts:
            print(f"error: execution filter starved after {attempts} attempts "
                  f"(judged={judged}, discarded={discarded})", file=sys.stderr)
            return 2
        pair = make_pair(seed)
        seed += 1
        if pair is None:
            discarded += 1
            continue
        judged += 1
        fails = judge_pair(pair, compilers, opts, repro_dir)
        if fails:
            failures += 1
            print(f"X seed={pair.seed} {pair.polarity}: {fails[0]}")
            for f in fails[1:]:
                print(f"    {f}")
        else:
            print(".", end="", flush=True)
    print(f"\nmutpair: judged={judged} failures={failures} discarded={discarded} "
          f"compilers={','.join(compilers)} opts={','.join(opts)}")
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(
            {"judged": judged, "failures": failures, "discarded": discarded,
             "compilers": list(compilers), "opts": opts}, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
