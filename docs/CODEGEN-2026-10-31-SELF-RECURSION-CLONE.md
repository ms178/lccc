# One-level self-recursion clone

GCC expands a *directly* self-recursive callee one level. LCCC refused every
recursive callee in both ordinary inlining passes, which is the correct default
for a fixed-point inliner — a clone still contains the recursive call, so
repeating the expansion only inflates the caller until the size cap — but it
also left the one bounded level on the table. For call-heavy recursion that
level is where the time is.

`binary_trees` (malloc/recursion/pointer chasing over 2^d nodes, the shape of
the Computer Language Benchmarks Game kernel) is the measurement:

| build | lccc/GCC median (paired, `-O2`, 9 rounds, 1 warm-up) |
|---|---|
| before | 1.2851 |
| after | 1.0243 |

The CI report's ratio for the same benchmark was 1.1412 over the whole run, so
the cluster this closes is 12-20% of the runtime of a benchmark that the
32-shape comparison ranks sixth-worst.

## What the transform does

`clone_self_recursion_sites` (`src/passes/inline.rs`) runs once per inliner
invocation, before the ordinary rounds, and for each function `F` that calls
itself directly it expands every self call site exactly once with the existing
`inline_call_site` machinery:

```
walk_tree:                     walk_tree:
    ... call walk_tree             ... call walk_tree      ; site A, expanded
    ... call walk_tree             ...   (body of walk_tree, with its own
    ret                                  two calls)
                                   ... call walk_tree      ; site B, expanded
                                   ...   (second copy)
                                   ret
```

Two things change at runtime, and only these two:

1. **Per level, half the call/return pairs.** A body with two self calls per
   node now performs one call per *two* nodes instead of one per node, so a
   full traversal pays half the calls, half the prologues, and half the
   epilogues.
2. **Frame depth halves.** The expanded level runs in the parent's frame, so a
   traversal of depth `d` needs `ceil(d/2)` frames, not `d`.

Within one invocation the site list is collected once from the entry body and
the clones' own self calls are never revisited: `inline_call_site` inserts the
clone blocks directly after the split block, so the sites are processed in
reverse program order and every remaining site keeps its
`(block_idx, inst_idx)`.

Across invocations the expansion is *bounded*, not one-shot, and the bound is
what makes it safe. A body with `N` self call sites comes back with `N^2`, so a
tree shape (`N = 2`) is exactly one level — `4 > MAX_SELF_CLONE_SITES` — while
a single-site body may expand again while it still fits
`MAX_SELF_CLONE_INSTRUCTIONS`; every expansion strictly grows the body, and all
four caps bound that growth, so the sequence terminates. GCC's
`max-inline-recursive-depth-auto` makes the same trade.

## Limits, each of which is a gate

| limit | value | why |
|---|---|---|
| body size | `MAX_SELF_CLONE_INSTRUCTIONS` = 64 | the clone duplicates the body once per site; a large recursive body would be resident twice over |
| expansion | bounded, not one-shot | `N` sites become `N^2`; a tree shape stops at one level, a single-site body keeps expanding while it fits |
| blocks | `MAX_SELF_CLONE_BLOCKS` = 6 | branch-light bodies only |
| sites | `MAX_SELF_CLONE_SITES` = 3 | a full binary recursion has two; wider recursion stops paying |
| final size | `MAX_SELF_CLONE_FINAL_INSTRUCTIONS` = 192 | `entry + sites * entry` must fit; bounds code growth |
| frame | `agg_frame_bytes == 0`, no `DynAlloca` | the frame may grow by one *static* level, never by a dynamic object |
| shape | no tail-position self call | `tail_call_elim` owns it (see below) |
| mode | not `-Os`, not the always-inline-only invocation | size policy and the restricted mode stay untouched |
| attributes | not `always_inline` (phase 0 owns it), not `noinline`, no `section` | attribute contracts win |

Static locals are *not* a hazard, unlike ordinary inlining: both copies are the
same function, so they name the same object.

### Why a tail self call is refused

`tce_sum`'s `sum(n, acc)` has a tail-position self call. `tail_call_elim`
converts it into a back edge before the main optimization loop, and
`closed_form_tail_sum` then collapses the loop into an arithmetic identity — the
benchmark folds to a constant. Cloning the body first destroys the pattern that
both of those recognise: measured while developing this pass, `sum` stopped
folding and became a runtime loop (0.95 -> 1.04 of GCC on `tce_sum`). A function
with any tail-position self call is therefore left entirely to TCE.

## The defect this exposed: an unfinishable compile-time fold crashed the compiler

`ipcp`'s `evaluate_constant_calls` interprets constant calls with a
2,000,000-*step* budget. Steps bound work, not stack: one recursion level costs
one step, so a body that recurses per step (`return 1 + deep(n - 1)`) reached
hundreds of thousands of Rust frames and aborted the compiler with

```
thread '<unknown>' (6558) has overflowed its stack
fatal runtime error: stack overflow, aborting
```

The pipeline runs on a 64 MiB stack (`src/lib.rs`), so the bound is now
`MAX_EVAL_CALL_DEPTH` = 65,536 nested evaluated calls. Sized from the folds the
pass exists for rather than from the budget: `ackermann(3, 11)` — the shape
`tests/benchmark/programs/ackermann.c` and `constant_recursion.c` both use, and
the reason those benchmarks fold to a constant and run 0.014-0.023 of GCC —
reaches an intermediate `ackermann(2, 16379)` and therefore nests ~16.4k calls.
65,536 leaves 4x headroom at ~100 bytes of interpreter frame per level (measured
against the 64 MiB stack: an unbounded 10M-deep fold aborted near 700k levels),
so the bound converts the crash into a refusal without rejecting any fold that
fits. Both benchmarks keep their ratios (0.018 and 0.024 measured after the
bound was added).

`tests/regression/check_ipcp_eval_depth.sh` pins it: a 100k-deep constant
recursion must compile, must print the right answer, and must still contain the
call — if the bound is removed the compile aborts, so the gate cannot pass
vacuously.

## Kill switch

`CCC_NO_SELF_RECURSION_CLONE=1` restores the pre-change codegen exactly (the
clone never runs; no other decision in the pass consumes its result). It is the
negative control of `tests/regression/check_self_recursion_clone.sh`, which
asserts on the emitted assembly that

* `walk_tree` / `count_tree` / `free_tree` go from 2 self calls to exactly 4 —
  the tree shape is expanded one level and is not a fixed point;
* `tail_sum` (tail position) and `big_walk` (175 IR instructions) keep their
  call count *and* instruction count;
* the clone-on and clone-off binaries print the same checksum.

## Rejected shapes

* **Two or more levels.** A second pass over an already-cloned body reaches the
  same size cap quickly (`walk_tree` goes 21 -> 63 IR instructions on one
  level), and the second level halves an already-halved call count while
  tripling code size per site. GCC's `make` reaches ~5x its body, but at the
  cost of an I-cache footprint this benchmark does not repay.
* **Mutual recursion (`F -> G -> F`).** The frame growth is one level *per
  function in the cycle*, and the site that must be identified is not the
  caller's own name. Not worth the proof for the shapes measured here.
* **Cloning a self-recursive callee into a non-recursive caller.** Only the
  outermost node is expanded once, which cannot pay for the body copy; the
  clone is valuable precisely because it applies at *every* level.
* **Raising the frame-neutral leaf rule instead.** `is_frame_neutral_leaf`
  already admits a recursive callee that is one block, call-free and
  frame-free. `make`/`check`/`destroy` call `malloc`/`free` and branch, so the
  rule cannot reach them without losing the property that makes it safe.

## What is left in the worst-25 cluster

Ranked by the CI report's `lccc/GCC` ratio, with the diagnosis each one now has:

| benchmark | CI ratio | diagnosis |
|---|---|---|
| `expat_siphash24` | 1.3161 | the inlined `sip24_final` switch emits one `mov`+`jmp` per fall-through case (8+8 instructions GCC does not); the copies come from per-case phi elimination in the switch lowering, not from inlining |
| `nbody` | 1.3071 | GCC SLP-vectorises the `dx,dy` pair through `sqrtpd`/`divpd`; LCCC's pair vectorisation stops at the `d^2` `mulpd` and scalarises the `sqrt`/`div`, plus a redundant `movdqa` after each `unpcklpd` broadcast and a dead `xorpd` before `sqrtsd` |
| `double_reduction` | 1.2062 | the two-accumulator `pmuludq` loop recomputes the shared `a` shuffles per statement (`20` instructions per 4 elements vs GCC's `23` for *both* accumulators) — a CSE opportunity across vectorised statements, not a count problem |
| `glibc_strstr` | 1.1996 | a dead stack store (`movq %rax, 0(%rsp)`) survives in the skip-table build loop |
| `expat_xml_scan` | 1.1734 | the branch-chain name scan still wants the bitmask/`bt` idiom |
| `binary_trees` | 1.1412 | **fixed here** (12-20%) |
| `zstd_count` | 1.1341 | the inner 8-byte compare loop compares against a spilled loop bound (`cmpq 0(%rsp), %r9`) where GCC keeps it in a register |
| `zlib_ng_compare256` | 1.0829 | address arithmetic is fixed (`ptr_iv_collapse`, 1.219 -> 1.069); the residual is loop layout |

Everything from 1.0660 down is within 2.6% and mostly noise on a shared runner.
