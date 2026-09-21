#!/usr/bin/env python3
"""Model-check the fma-negation peel's deletion rule: a `Neg` may only be deleted
when every reader of it dies in the same rewrite.

WHY THIS EXISTS
---------------
`src/passes/fma_neg_peel.rs` rewrites `fma(-a, b, -c)` into the signed FMA family
and then DELETES every `Neg` it "absorbed". The deletion is unconditional and by
value id: whatever lands in `absorbed_ids` is swept, with no residual-use check
(`fma_neg_peel.rs:406-460`). That is sound only if the classification that feeds it
guarantees the invariant the pass's own doc states: *every read of an absorbed `Neg`
disappears with the rewrite*.

The rule as landed checks the wrong edge. Badness is documented as flowing from a
materialised outer `Neg` inward to its source ("a materialised outer Neg keeps its
read of the inner Neg alive"), but the loop poisons the source-to-reader direction
instead (`:257-292`). A `Neg` that is read by an fma argument AND by a `Neg` that
stays materialised is therefore classified absorbable, rewritten at the site and
deleted -- while the surviving `Neg` still names it. Reachable shape:

    double t = -x, u = -t;
    double r = __builtin_fma(t, b, c);
    return r + u;                      /* u = -t survives and still reads -x */

This harness re-derives the invariant from the shape space, so the rule can be
re-checked without a compiler, a fix can be validated before the Rust is touched,
and a future regression of the classification is caught by CI.

WHAT IS CHECKED
---------------
  fixtures     the shapes the pass's Rust unit tests build, with the deletion counts
               those tests assert. The specified rule must reproduce every one of them
               (the fix must not change any existing expectation).
  invariant    over an enumerated space of `Neg` chains (default: up to 3 `Negs`,
               4545 deduplicated shapes), no rule may leave an orphan:
                 * a deleted `Neg` with a surviving reader, or
                 * a deleted `Neg` whose non-peeling (`other`) read survives --
                   the sign latitude the family absorption takes is only allowed to
                   be taken when the sign-mask instruction actually dies.
  controls     the landed rule and the edge-flip proposed for its repair MUST both be
               flagged. A harness that cannot see the defect it was written for is
               worthless, so the negative controls are asserted, not merely printed.

THE THREE RULES
---------------
  landed       poison `vid` when its *source* is poisoned, then delete the
               fma-reachable clean `Negs`. This is the rule as it shipped when the
               harness was written; keep it as the counterexample reference even
               after the Rust is corrected -- it is not a description of the tree.
  reversed     the edge flip the fix note proposed: poison from *readers*, then delete
               the fma-reachable clean `Negs`. Necessary but NOT sufficient -- a reader
               that no fma chain reaches is never poisoned, so it still orphans its
               source. This harness pins that finding instead of restating it.
  closure      the specified rule. A `Neg` may be deleted iff
                 (i)   it has no `other` read, and
                 (ii)  every reader of it is deleted too, and
                 (iii) some fma-argument slot reaches it by descending `src` links
                       through deleted `Negs` (else no site ever substitutes it).
               (ii) and (iii) are mutually recursive: computed as the GREATEST
               fixpoint (start from every `other`-free `Neg`, drop the ones that only
               depend on dropped ones) until stable.

PORT NOTES (for the Rust fix this harness accompanies)
-----------------------------------------------------
  * Pass 1b (`fma_neg_peel.rs:175`) is the classification this model encodes as
    `fma` (argument slot of a width-matching 3-arg `FmaScalar`) / `negsrc` (the `src`
    read of another `Neg`) / `other` (everything else, width mismatches included).
  * Pass 1c (`:257`) computes the deleted set; Pass 2 (`:299`) walks the chains and
    records them; Pass 3 (`:378`) rewrites and sweeps. The repair belongs in the
    Pass 1c set, not in the sweep: with `closure`, every reader of a deleted `Neg` is
    deleted by construction, so the unconditional id sweep stays correct.
  * The chain walk in Pass 2 records the absorbed ids outermost-first (`:330-345`).
    Deletion does not care about that order, but any future refinement that retargets
    a *surviving* reader must process deepest-first -- keep that in mind.
  * `verify::verify_after_pass(module, "fmanegpeel")` is not wired at either peel
    call site (`passes/mod.rs:1161`, `:2652`), so malformed IR produced here is
    invisible to CI; `CCC_VERIFY_IR=abort` plus the shape above is the end-to-end
    reproduction.

USAGE
-----
    tools/ir_shape_check.py                  # invariant + fixtures + controls
    tools/ir_shape_check.py --rule landed    # report one rule across the space
    tools/ir_shape_check.py --max-negs 4     # widen it (162009 shapes, ~15s)
    tools/ir_shape_check.py --json           # machine-readable report
    tools/ir_shape_check.py --quiet          # one summary line

Exit status: 0 when every check passes; 1 when the invariant is violated or a
negative control survives (the harness lost its teeth); 2 on a usage error.
"""

from __future__ import annotations

import argparse
import itertools
import json
import sys
from collections import Counter
from dataclasses import dataclass

# --------------------------------------------------------------------------- #
# Shape model
# --------------------------------------------------------------------------- #
#
# A shape is a small SSA fragment:
#   * `negs`  : (id, src) for every float Neg, `src == 0` meaning "a parameter",
#               any other value naming another Neg (defs are acyclic: src < id).
#   * `fma`   : (id, count) of argument-slot reads by width-matching FmaScalar sites.
#   * `other` : (id, count) of non-peeling reads (add, store, phi, terminator,
#               width-mismatched fma, mistyped use, ...).
# The src read of a Neg by another Neg is implied by `negs` and is never listed:
# it is the `negsrc` kind, derived by `readers()`.


@dataclass(frozen=True)
class Shape:
    """One deduplicated IR shape (all tuples sorted by Neg id; zeros omitted)."""

    negs: tuple[tuple[int, int], ...]
    fma: tuple[tuple[int, int], ...] = ()
    other: tuple[tuple[int, int], ...] = ()

    @property
    def ids(self) -> tuple[int, ...]:
        return tuple(n for n, _ in self.negs)

    def src(self, neg: int) -> int:
        return dict(self.negs)[neg]

    def fma_reads(self, neg: int) -> int:
        return dict(self.fma).get(neg, 0)

    def other_reads(self, neg: int) -> int:
        return dict(self.other).get(neg, 0)

    def readers(self) -> dict[int, tuple[int, ...]]:
        """neg -> every Neg whose `src` is that neg (the `negsrc` reads)."""
        out: dict[int, list[int]] = {n: [] for n in self.ids}
        for n in self.ids:
            s = self.src(n)
            if s in out:
                out[s].append(n)
        return {k: tuple(v) for k, v in out.items()}

    def render(self) -> str:
        readers = self.readers()
        defs = [f"n{n}=-{'x%d' % self.src(n) if self.src(n) else 'param'}"
                for n in self.ids]
        reads = []
        for n in self.ids:
            kinds = []
            if self.fma_reads(n):
                kinds.append(f"fma x{self.fma_reads(n)}")
            if self.other_reads(n):
                kinds.append(f"other x{self.other_reads(n)}")
            if readers[n]:
                kinds.append("read by " + ",".join(f"n{r}" for r in readers[n]))
            if kinds:
                reads.append(f"n{n}: " + ", ".join(kinds))
        return "; ".join(defs) + ("  ||  " + "; ".join(reads) if reads else "")


def canonical(negs: dict[int, int], fma: dict[int, int], other: dict[int, int]) -> Shape:
    ids = sorted(negs)
    return Shape(
        negs=tuple((n, negs[n]) for n in ids),
        fma=tuple((n, fma[n]) for n in ids if fma.get(n)),
        other=tuple((n, other[n]) for n in ids if other.get(n)),
    )


def reach_from_fma(shape: Shape, keep: frozenset[int]) -> frozenset[int]:
    """Negs a site actually rewrites: args, then `src` links while both ends stay."""
    out: set[int] = set()
    ids = set(shape.ids)
    stack = [n for n in ids if shape.fma_reads(n)]
    seen: set[int] = set()
    while stack:
        v = stack.pop()
        if v in seen:
            continue
        seen.add(v)
        if v in keep:
            out.add(v)
            s = shape.src(v)
            if s in ids:
                stack.append(s)
    return frozenset(out)


# --------------------------------------------------------------------------- #
# The rules under test: each returns the set of Negs it deletes
# --------------------------------------------------------------------------- #


def rule_landed(shape: Shape) -> frozenset[int]:
    """The implementation as landed: badness propagates source -> reader."""
    bad = {n for n in shape.ids if shape.other_reads(n)}
    changed = True
    while changed:
        changed = False
        for n in shape.ids:
            if n in bad:
                continue
            src = shape.src(n)
            if src in bad:
                bad.add(n)
                changed = True
    return reach_from_fma(shape, frozenset(set(shape.ids) - bad))


def rule_reversed(shape: Shape) -> frozenset[int]:
    """The proposed one-line repair: badness propagates reader -> source."""
    keep = {n for n in shape.ids if not shape.other_reads(n)}
    changed = True
    while changed:
        changed = False
        for n in list(keep):
            if any(r not in keep for r in shape.readers()[n]):
                keep.discard(n)
                changed = True
    return reach_from_fma(shape, frozenset(keep))


def rule_closure(shape: Shape) -> frozenset[int]:
    """The specified rule: greatest fixpoint incl. the reader closure."""
    keep = {n for n in shape.ids if not shape.other_reads(n)}
    changed = True
    while changed:
        changed = False
        reach = reach_from_fma(shape, frozenset(keep))
        for n in list(keep):
            if n not in reach or any(r not in keep for r in shape.readers()[n]):
                keep.discard(n)
                changed = True
    return frozenset(keep)


RULES = {
    "landed": rule_landed,
    "reversed": rule_reversed,
    "closure": rule_closure,
}

# --------------------------------------------------------------------------- #
# The invariant
# --------------------------------------------------------------------------- #


def orphans(shape: Shape, deleted: frozenset[int]) -> tuple[tuple[int, str], ...]:
    """Every way `deleted` breaks the invariant, as (neg, reason) pairs."""
    out: list[tuple[int, str]] = []
    readers = shape.readers()
    for n in sorted(deleted):
        if shape.other_reads(n):
            out.append((n, "deleted with a surviving non-peeling read"))
        for r in readers[n]:
            if r not in deleted:
                out.append((n, f"deleted but still read by surviving n{r}"))
    return tuple(out)


def under_fold(rule, shapes) -> int:
    """Shapes where the specified rule deletes more than `rule` does."""
    return sum(1 for s in shapes if rule_closure(s) - rule(s))


# --------------------------------------------------------------------------- #
# Space enumeration
# --------------------------------------------------------------------------- #


def enumerate_shapes(max_negs: int):
    """Every deduplicated shape with up to `max_negs` Negs (acyclic, src < id)."""
    seen: set[Shape] = set()
    for k in range(1, max_negs + 1):
        ids = list(range(1, k + 1))
        domains = [[0] + [m for m in ids if m < n] for n in ids]
        for srcs in itertools.product(*domains):
            negs = dict(zip(ids, srcs))
            for fma_counts in itertools.product(range(3), repeat=k):
                for other_counts in itertools.product(range(3), repeat=k):
                    shape = canonical(negs, dict(zip(ids, fma_counts)),
                                      dict(zip(ids, other_counts)))
                    if shape not in seen:
                        seen.add(shape)
                        yield shape


# --------------------------------------------------------------------------- #
# Fixtures: the shapes the Rust unit tests build, and the counts they assert
# --------------------------------------------------------------------------- #
#
# `expected` is the value the Rust test asserts for `peel(&mut module)` (the number
# of Neg instructions removed). Every one of these must hold for the specified rule:
# the correction may not change any existing expectation. `landed` is asserted too --
# the two agree on all existing tests, which is exactly why the gap went unnoticed.

RUST_TESTS_TOTAL = 16
# 14 tests assert on a fold pattern; `source_spans_stay_aligned` asserts on span
# bookkeeping for a shape that is itself covered by the last fixture, and
# `gate_off_is_a_no_op` exercises the capability gate, which has no IR shape.
SHAPE_MODELLABLE_TESTS = 14
NOT_SHAPE_MODELLABLE = (
    "gate_off_is_a_no_op (capability flag, no IR shape)",
)


@dataclass(frozen=True)
class Fixture:
    name: str
    shape: Shape
    expected: int
    note: str


FIXTURES: tuple[Fixture, ...] = (
    Fixture("negated_multiplier_peels_into_signed_family",
            Shape(((1, 0),), ((1, 1),)), 1,
            "fma(-a, b, c) -> Signed(true, false)"),
    Fixture("both_sides_negated_uses_fnmsub_family",
            Shape(((1, 0), (5, 4)), ((1, 1), (5, 1))), 2,
            "fma(-a, b, -c) -> Signed(true, true)"),
    Fixture("double_multiplier_negation_cancels_to_plain",
            Shape(((1, 0), (5, 3)), ((1, 1), (5, 1))), 2,
            "fma(-a, -b, c) -> plain family"),
    Fixture("chained_negations_peel_transitively",
            Shape(((1, 0), (5, 1)), ((1, 0), (5, 1))), 2,
            "fma(-(-a), b, c) -> plain family reading a"),
    Fixture("non_adjacent_negations_peel",
            Shape(((1, 0), (5, 4)), ((1, 1), (5, 1))), 2,
            "unrelated work between the Negs"),
    Fixture("multi_use_negation_is_left_alone",
            Shape(((1, 0),), ((1, 1),), ((1, 1),)), 0,
            "an add reads the Neg: the materialisation stays"),
    Fixture("same_value_in_two_positions_cancels_to_plain",
            Shape(((1, 0),), ((1, 2),)), 1,
            "fma(n, n, c) with n = -a: both flips cancel"),
    Fixture("shared_negation_across_two_fma_sites_peels_both",
            Shape(((1, 0), (5, 4)), ((1, 2), (5, 2))), 2,
            "the phi-diamond headline: two sites share both Negs"),
    Fixture("shared_negation_with_external_reader_stays_materialised",
            Shape(((1, 0),), ((1, 2),), ((1, 1),)), 0,
            "the add pins the materialisation, so no site peels"),
    Fixture("cross_block_negation_dominating_both_sites_peels",
            Shape(((1, 0),), ((1, 1),)), 1,
            "dominance is transitive; block layout is irrelevant"),
    Fixture("chained_shared_negation_peels_transitively_at_both_sites",
            Shape(((1, 0), (5, 1)), ((5, 2),)), 2,
            "two sites read the same double negation"),
    Fixture("width_mismatch_is_never_absorbed",
            Shape(((1, 0),), (), ((1, 1),)), 0,
            "a width-mismatched fma slot classifies as `other` (defensive reject)"),
    Fixture("f32_peels_with_own_width",
            Shape(((1, 0),), ((1, 1),)), 1,
            "the F32 family takes the same path"),
    Fixture("addend_only_negation_peels",
            Shape(((1, 4),), ((1, 1),)), 1,
            "fma(a, b, -c) -> Signed(false, true)"),
    Fixture("source_spans_stay_aligned (shape half)",
            Shape(((1, 0), (5, 4)), ((1, 1), (5, 1))), 2,
            "the same shape as both_sides_negated; spans must stay parallel"),
)

# --------------------------------------------------------------------------- #
# Counterexamples this harness was written for
# --------------------------------------------------------------------------- #

@dataclass(frozen=True)
class Control:
    """A shape the harness MUST flag, with which rules it must flag it for.

    `reversed` fixing the first shape but not the second is the whole point: the
    one-line edge flip repairs the case a live reader exposes, which is why it looks
    sufficient in review, but it is not a specification.
    """

    name: str
    shape: Shape
    why: str
    expect_landed: bool
    expect_reversed: bool
    c_sketch: str


COUNTEREXAMPLES: tuple[Control, ...] = (
    Control(
        "chain link with a surviving outer reader (the shipped defect)",
        Shape(((1, 0), (2, 1)), ((1, 1),), ((2, 1),)),
        "n1 is read by the fma argument and by n2; n2 has a non-peeling read, so it "
        "survives -- and n1 is deleted underneath it.",
        expect_landed=True,
        expect_reversed=False,
        c_sketch=(
            "double t = -x;\n"
            "double u = -t;                        /* u reads t */\n"
            "double r = __builtin_fma(t, b, c);    /* the site reads t */\n"
            "return r + u;                         /* u survives */"
        ),
    ),
    Control(
        "surviving reader that no fma chain reaches (kills the one-line repair)",
        Shape(((1, 0), (2, 1)), ((1, 2),)),
        "n2 has no non-peeling read and no fma read, so the reader-side flip never "
        "poisons it and never deletes it -- yet n1 is deleted while n2 still reads it. "
        "A dead reader is normally removed by DCE before the peel, so this is the "
        "weaker of the two hazards; it is pinned because it is what makes the "
        "one-liner a repair rather than a rule.",
        expect_landed=True,
        expect_reversed=True,
        c_sketch="(no faithful C spelling: the outer Neg must be read by nothing "
                 "else, which DCE removes first)",
    ),
)


# --------------------------------------------------------------------------- #
# Reporting
# --------------------------------------------------------------------------- #


def report_space(rule_name: str, shapes: list[Shape], quiet: bool) -> dict:
    rule = RULES[rule_name]
    stats: Counter = Counter()
    examples: list[dict] = []
    for s in shapes:
        deleted = rule(s)
        bad = orphans(s, deleted)
        stats["shapes"] += 1
        if bad:
            stats["unsafe"] += 1
            if len(examples) < 3:
                examples.append({"shape": s.render(), "orphans": list(bad)})
        if deleted:
            stats["folds"] += 1
        if rule_closure(s) - deleted:
            stats["under_fold"] += 1
    result = {
        "rule": rule_name,
        "shapes": stats["shapes"],
        "unsafe": stats["unsafe"],
        "folds": stats["folds"],
        "under_fold": stats["under_fold"],
        "examples": examples,
    }
    if not quiet:
        n = stats["shapes"] or 1
        tag = " (the rule this harness was written for; kept as the counterexample)" \
            if rule_name == "landed" else ""
        print(f"  rule `{rule_name}`{tag}: "
              f"unsafe {stats['unsafe']}/{n} ({100.0 * stats['unsafe'] / n:.2f}%) | "
              f"shapes with a fold {stats['folds']}/{n} "
              f"({100.0 * stats['folds'] / n:.2f}%) | "
              f"under-folded vs the specified rule {stats['under_fold']}")
        for e in examples:
            print(f"      counterexample: {e['shape']}")
            for neg, why in e["orphans"][:2]:
                print(f"        n{neg}: {why}")
    return result


def check_fixtures(quiet: bool) -> tuple[bool, list[dict]]:
    rows: list[dict] = []
    ok = True
    for f in FIXTURES:
        spec = len(rule_closure(f.shape))
        landed = len(rule_landed(f.shape))
        good = spec == f.expected and landed == f.expected
        ok &= good
        rows.append({"name": f.name, "expected": f.expected, "closure": spec,
                     "landed": landed, "ok": good, "note": f.note})
        if not quiet:
            mark = "ok " if good else "BAD"
            print(f"  [{mark}] {f.name:56s} expected {f.expected} | "
                  f"specified {spec} | landed {landed}")
    return ok, rows


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Model-check the fma-negation peel's deletion invariant.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--rule", choices=sorted(RULES),
                    help="report one rule over the space instead of the full check")
    ap.add_argument("--max-negs", type=int, default=3,
                    help="widen the enumeration (default 3 -> 4545 shapes; "
                         "4 -> 162009 shapes, ~15s)")
    ap.add_argument("--json", action="store_true", help="machine-readable report")
    ap.add_argument("--quiet", action="store_true", help="one summary line only")
    args = ap.parse_args()
    if args.max_negs < 1:
        print("error: --max-negs must be >= 1", file=sys.stderr)
        return 2
    if args.max_negs > 5:
        print("error: --max-negs is capped at 5 -- the space grows factorially "
              "(3 -> 4545 shapes, 4 -> 162009, then factorially)", file=sys.stderr)
        return 2

    # `--json` and `--quiet` both mean "no human prose on stdout".
    verbose = not (args.quiet or args.json)
    shapes = list(enumerate_shapes(args.max_negs))
    if not shapes:  # pragma: no cover - defensive
        print("error: the enumeration produced no shapes", file=sys.stderr)
        return 2

    payload: dict = {
        "max_negs": args.max_negs,
        "shapes": len(shapes),
        "rust_tests_total": RUST_TESTS_TOTAL,
        "shape_modellable_tests": SHAPE_MODELLABLE_TESTS,
    }

    if args.rule:
        if verbose:
            print(f"space: {len(shapes)} shapes (<= {args.max_negs} Negs)")
        rep = report_space(args.rule, shapes, quiet=not verbose)
        payload["report"] = {args.rule: rep}
        if args.json:
            print(json.dumps(payload, indent=2, sort_keys=True))
        elif args.quiet:
            print(f"ir-shape-check: rule `{args.rule}` unsafe "
                  f"{rep['unsafe']}/{rep['shapes']}")
        rc = 1 if rep["unsafe"] and args.rule == "closure" else 0
        return rc

    if verbose:
        print(f"space: {len(shapes)} deduplicated shapes (<= {args.max_negs} Negs)")
        print(f"\n1. fixtures -- {SHAPE_MODELLABLE_TESTS} shape-modellable Rust unit "
              f"tests + the spans test's shape, out of {RUST_TESTS_TOTAL} tests total; "
              "not shape-modellable:")
        for note in NOT_SHAPE_MODELLABLE:
            print(f"       - {note}")
    fixtures_ok, rows = check_fixtures(quiet=not verbose)
    payload["fixtures"] = rows
    payload["fixtures_ok"] = fixtures_ok

    if verbose:
        print("\n2. invariant -- no rule may delete a Neg a surviving reader names")
    report = {name: report_space(name, shapes, quiet=not verbose) for name in RULES}
    payload["report"] = report

    if verbose:
        print("\n3. negative controls -- the harness must see both known defects")
    controls: list[dict] = []
    controls_ok = True
    for c in COUNTEREXAMPLES:
        landed_orphans = orphans(c.shape, rule_landed(c.shape))
        reversed_orphans = orphans(c.shape, rule_reversed(c.shape))
        closure_orphans = orphans(c.shape, rule_closure(c.shape))
        seen_landed = bool(landed_orphans)
        seen_reversed = bool(reversed_orphans)
        good = (seen_landed == c.expect_landed
                and seen_reversed == c.expect_reversed
                and not closure_orphans)
        controls_ok &= good
        controls.append({
            "name": c.name,
            "landed_orphans": [list(g) for g in landed_orphans],
            "reversed_orphans": [list(g) for g in reversed_orphans],
            "closure_orphans": [list(g) for g in closure_orphans],
            "seen_landed": seen_landed, "seen_reversed": seen_reversed,
            "expected": {"landed": c.expect_landed, "reversed": c.expect_reversed},
            "why": c.why, "c": c.c_sketch,
        })
        if verbose:
            mark = "ok " if good else "BAD"
            print(f"  [{mark}] {c.name}")
            print(f"         {c.shape.render()}")
            print(f"         {c.why}")
            print(f"         orphans -- landed {len(landed_orphans)}, "
                  f"reader-side flip {len(reversed_orphans)}, "
                  f"specified rule {len(closure_orphans)}")
            if c.c_sketch:
                first = c.c_sketch.splitlines()[0].strip()
                more = "  ..." if "\n" in c.c_sketch else ""
                print(f"         C: {first}{more}")
    payload["controls"] = controls
    payload["controls_ok"] = controls_ok

    closure_unsafe = report["closure"]["unsafe"]
    summary = (f"ir-shape-check: fixtures {'ok' if fixtures_ok else 'FAILED'} | "
               f"specified rule unsafe {closure_unsafe} | "
               f"controls {'ok' if controls_ok else 'FAILED'}")
    payload["summary"] = summary
    rc = 0 if (fixtures_ok and closure_unsafe == 0 and controls_ok) else 1
    if args.json:
        print(json.dumps(payload, indent=2, sort_keys=True))
    else:
        print(summary)
        if verbose and rc == 0:
            print("The specified rule deletes a Neg only when every reader dies with "
                  "it; the landed rule and the one-line repair both do not.")
    return rc


if __name__ == "__main__":
    sys.exit(main())
