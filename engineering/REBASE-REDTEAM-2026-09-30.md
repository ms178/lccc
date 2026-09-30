# Red-team: rebase onto new main, and the PR #686 convergence

Date: 2026-09-30. Base: `github/main` = `71e3b1ae`.

This is my own work audited adversarially. Where the evidence contradicts
something I asserted earlier, that is recorded rather than smoothed over.

## 0. The rebase changed the shape of the problem

**PR #686 was merged into main at 03:02 today**, four hours before this work
started:

```
71e3b1ae Merge pull request #686 from ms178/arena/01a0efa5-lccc
345a5e9c Close the audit findings: preheader dominance guard, honest coverage
         docs, --compile-only contract
67cabbee Integer min/max vectorization, dedicated loop preheaders, ...
```

My branch and #686 were **two parallel implementations of the same feature**.
Rebasing therefore replayed nine of my ten commits onto a base that already
contained the work I had spent the previous session hand-adopting.

The first commit (`6358140a`, MINMAX-1) is ~98 % superseded — 35 of its 37
files are already in main. The only two that are not
(`minmax_harness.c`, `minmax_refused_main.c`) are covered by a fallback path
that `check_minmax_reduction.sh` already had, and main's placement of them in
`minmax_shapes/` is better documented than mine was. **I skipped that commit
rather than re-litigating a merged PR.**

The seven that replayed cleanly were audits, gates, and docs. Three needed
adjudication, and one needed a real decision.

## 1. A duplicate RAII guard, and the setter that went missing

The merge produced `vectorize.rs` containing **both** `LateMinMaxOnlyScope`
(mine) and `LateMinMaxOnlyScope` (main's), and dropped main's
`set_late_minmax_only` — so the tree **would not have compiled**.

They are not equivalent, and the difference is the whole point:

| | mine | main's |
|---|---|---|
| `enter()` | `set(true)` | saves `previous`, `debug_assert!(!previous, "nested…")` |
| `drop()` | `set(false)` | `set(self.previous)` |

Mine hard-codes `false` on drop, so a nested scope would clear the outer
scope's flag. Main's restores the previous value and **debug-asserts on
nesting**, failing loudly in debug builds instead of silently mis-restricting
the vectorizer for the next translation unit on that thread.

**Main's is strictly better and I kept it**, deleting mine. This is a real
improvement over my branch that I had not found in the previous round — the
earlier audit compared the two branches only where the review pointed, and
`LateMinMaxOnlyScope` was on my list to check but the session ended first.
Recording that rather than quietly fixing it.

Two call sites in `mod.rs` also had `let _g = …enter(); …; drop(_g);`. The
explicit `drop()` is redundant next to a named binding and actively
misleading — it implies the drop must be manual. Replaced with block scope,
which is both shorter and scopes the guard to exactly the calls it covers.

I also collapsed a comment duplication: #686 had added a six-line block
restating the volatile-LICM rationale already in the paragraph above it. One
authoritative explanation, keeping the non-obvious part ("checked FIRST and
unconditionally — not a profitability judgement").

## 2. Three merge conflicts I committed, and the gate that now prevents them

The rebase left conflict markers **inside the committed tree** in
`ci_local.sh`, `check_volatile_licm.sh`, and `ci.yml`. This is the most
serious defect I introduced this session, and the reason it is dangerous is
that it is *quiet*:

> A conflict marker inside a shell script is a syntax error only when the
> shell **reaches the line**. Every gate registered above the damage still
> runs and still prints PASS.

`ci_local.sh` died at line 375 with every gate below it unrun, reported as
`exit 2`. Had the markers landed lower in the file, the run would have
reported a wall of green for a suite that never executed.

Adjudication, not mechanical merge:

* **`ci_local.sh`** — both sides registered `regression-corpus-link`, and
  HEAD's side also brought `volatile-licm`, which was **already registered five
  lines above**. Taking HEAD wholesale would have run that gate twice. I took
  the union without the duplicate.
* **`check_volatile_licm.sh`** — this side's `|| rc=$?` is required, and
  HEAD's was a **latent bug**: it set `rc=0` before the heredoc and then read
  `rc=$?` after it, but with `|| rc=$?` in play `$?` is 0 at that point, so
  the line **clobbers the captured failure** and a failing structural check
  would have reported PASS.
* **`ci.yml`** — comment only.

The fix is not the three markers, it is `scripts/check_no_conflict_markers.py`
(mutation-verified: planted conflict → exit 1; clean tree → exit 0). It
requires the real three-line opener/middle/closer shape, so a `=======` under
a Markdown heading and a bare `=======` in prose both pass — verified
separately. Wired before `build` locally and before the first executed step in
CI, so the damage is reported as itself.

## 3. The peephole, and what I got wrong building it

`cmov` reads EFLAGS but does not write it. The backend emits one `cmp` per
`cmov`, so an if-converted chain sharing a condition gets one dead
recomputation per `cmov` after the first. SQLite's varint decoder is the
canonical shape.

**My first implementation never fired.** I instrumented the pass and found
the reason: I dispatched on `flags_effect(tk)` *before* testing `tk == t`, and
the second `cmp` is itself a flag **writer**, so the walk broke out on the very
line the pass exists to delete. The identity test has to come first. I only
found this because I checked *why it didn't fire* instead of assuming the
pattern was unreachable.

### The soundness condition I nearly got wrong

The operand-write guard (nothing between the two `cmp`s may redefine an
operand register) looked like belt-and-braces. It is not:

```
mutation: delete the operand-write guards
  → 2 corpus tests miscompile
    bb_slp_i64_to_i32_select: gt_big[2]: got 6 want 100000
    array_string_init_matrix_O0
```

**And the negative result matters as much as the positive one.** I wrote a C
fixture for the hazard, and it *passed under the mutant* — because the
register allocator normally gives the two compares different registers, so the
texts differ and the fold declines anyway. The hazard only opens where
allocation reuses one register across the redefinition. I deleted the fixture
rather than ship a test that proves nothing, and recorded the corpus case as
the real evidence. A green test that cannot fail is worse than no test,
because it buys false confidence.

### Measurement, and the noise floor

| | result |
|---|---|
| corpus files shrunk / grew | **28 / 0** (76 instructions) |
| runtime differential, 10 kernels | checksums **identical** |
| regression corpus | **846 passed, 0 failed** |
| varint, interleaved A/B, 41 samples | min **−4.32 %**, median **−2.66 %**, paired win **25/41 = 61 %** |

The 61 % only means something because I calibrated the floor: `strcmp_signed`,
which this pass **does not touch**, scores **52 %** paired win and +0.12 % on
min. A 52 % reading is noise. That is also why the first single-shot
`bench_kernels` geomean that appeared to *regress* on `matchlen` should be
discounted — `matchlen`'s assembly did not change at all, so that was the box,
not the compiler.

This is why I added `scripts/ab_interleaved.py`: running arm A to completion
and then arm B lets slow drift masquerade as a speedup.

## 4. Performance: what I found, and what I deliberately did not do

Priority kernels, wall-clock, ratio > 1 means LCCC is slower, all checksums
matching GCC:

| kernel | ratio | verdict |
|---|---|---|
| zlib_ng_adler32 | 0.930 | LCCC wins |
| lz4_compress | 1.015 | parity |
| sieve | 1.121 | **missing vectorization** |
| expat_xml_scan | 1.256 | **named primary case** |
| sqlite_varint | 1.257 | |
| matmul | 1.399 | **missing loop interchange** |

Overall LCCC emits *fewer* instructions than GCC across the benchmark corpus
(13211 vs 20495, ratio 0.645). The losses are concentrated and structural,
not diffuse — which is useful, because it means they are nameable.

### Two gaps, isolated with a four-case experiment

| shape | LCCC | GCC |
|---|---|---|
| `c += p[i]`, `int*` | **18** ✓ | 25 ✓ |
| `c += p[i]`, `signed char*` | 2 ✗ | 43 ✓ |
| `if (p[i]) c++`, `int*` | 1 ✗ | 22 ✓ |
| `if (p[i]) c++`, `signed char*` | 1 ✗ | 46 ✓ |

So LCCC vectorizes plain I32 sum reductions and **nothing else**: not
byte-width reductions, and not masked (conditional) reductions at any width.
`sieve` is exactly the bottom-right cell — GCC emits
`vpcmpeqb` → `vpmovsxbw` → `vpsubd` for the prime-counting loop; LCCC emits
**zero** vector instructions for the whole file.

The blocker is concrete: `src/ir/intrinsics.rs` has **only** I32→I64 widening
(`VecWidenAddI32x4ToI64x2`, `VecLoadWidenI32ToI64x2`). There is no I8→I32
widening op anywhere in the IR or the x86 backend. Closing this needs a new
intrinsic, its `vpmovsxbwd`+`vpaddd` lowering, vectorizer pattern matching for
I8 element types, and remainder handling — three layers.

### What I did not do, and why

**I did not implement byte/masked reduction vectorization.** It is the right
next move and I believe it is worth more than anything else on this list, but
it is a feature spanning IR, backend, and the vectorizer, and the only way to
"finish" it in the time left would be to skip the corpus run and the mutation
test. On a vectorizer that is a miscompile waiting to ship, and correctness is
the hard constraint. I would rather hand over a proven peephole and a precise,
reproducible isolation of the gap than an unvalidated vectorizer.

**I did not implement loop interchange** (`matmul`, worst at 1.399). There is
no such pass — `grep` finds nothing. GCC rewrites `C[i][j] += A[i][k]*B[k][j]`
from i,k,j into i,j,k so `C[i][j]` is contiguous and `A[i][k]` is broadcast.
Loop interchange requires dependence analysis to prove legality and index
remapping to rewrite the nest. That is a project, not a patch.

**I did not chase the move-count gap in `zlib_ng_adler32`.** LCCC emits 39
reg→reg moves against GCC's 12, which looks alarming. But they are spread
across cold blocks — the hot inner loop `.LBB13` contains only **2** of them.
That is a code-size defect, not a speed defect, and the kernel already runs
**faster** than GCC (0.930). Optimising it would have been measurement-free
work that makes a number look better without making the program faster.

## 5. Verdict

The rebase is sound: nine commits, one skipped as superseded, three conflicts
adjudicated on the merits (one of which was a latent PASS-on-failure bug in
HEAD's side), one duplicate RAII guard resolved in main's favour because it is
strictly better, and one comment de-duplicated.

Two defects I introduced are now structurally prevented rather than merely
fixed: committed conflict markers, and a peephole without mutation evidence.

One optimization landed, proven by instruction counts, runtime checksums, a
full corpus run, a mutation, and a calibrated interleaved measurement.

Two larger optimizations are identified, isolated to a four-case experiment,
and explicitly not attempted — with the reason recorded so the next engineer
starts from the isolation rather than from the symptom.

---

## Post-rebase validation on `f9bef39b` (2026-09-30)

The rebase onto `f9bef39b` is a **merge of two implementations**, not a replay:
`#688` (`b18decc5`) independently landed the loop-preheader gate, the volatile
gates, the godbolt-cache hardening and the empty-oracle fail-closed. Each
overlap was resolved deliberately; where upstream's version was a strict
superset, upstream's was taken.

### Gate results

`scripts/ci_local.sh --fast`: **116 passed, 0 failed, 5 skipped** — ALL GATES
GREEN, on the rebased tree.

Two regressions surfaced and were fixed, both real:

* **`incapable` had no producer.** The class existed only in
  `reloc_oracle_agreement`; all five of its unit tests hand-wrote the verdict
  into a notes dict, so they passed while the live pipeline could never emit
  it. Same "test passes but proves nothing about the real path" failure this
  series criticises elsewhere — committed one level down. Fixed in
  `918f8eed`; the real linker suite went 299p/2f -> **301p/0f** with mold on
  the box, and that red run is the mutation evidence that the fix is
  load-bearing.
* **`incapable` then inverted the verdict.** `agree` was filtered on
  `verdict != "inapplicable"`, so an incapable oracle entered as
  `(name, False)` — a *disagreement*. "This linker cannot parse the fixture"
  became "this linker disagrees". The five unit tests missed it because they
  test the two functions in isolation; the bug was in the glue between them,
  which nothing covered. Suite 40 -> **44 cases (24 verdict, 20 agreement)**,
  the four new ones exercising the *producer* with mold's real captured stderr.

### Performance, measured (not estimated)

Callgrind, 28-benchmark corpus, `mine` = rebased HEAD, `ref` = unpatched
`github/main` `f9bef39b` built from a worktree:

    geomean Ir mine/ref = 0.99861        worst case 1.00012 (constant_recursion)

`binary_search` **0.96071** — 3.9% fewer instructions. Attributed exactly: the
peephole deletes one `cmpl %r11d, %esi`, and with
`CCC_PEEPHOLE_SKIP=flags_compare` the output is **byte-identical to unpatched
upstream**, which proves the peephole is the only delta.

Godbolt oracle, `bench_run` of `tests/oracle/varint_decode.c` (-O2):

| lccc | gcc 16.2 | clang 23.1 | icc 2021.10 | icx |
| ---: | ---: | ---: | ---: | ---: |
| 53 | 38 | 34 | 62 | 146 |

lccc is 3rd of 5 and 39% above GCC. Traced to a systematic missed
strength-reduction on the loop exit compare (1 of 5 instructions per
iteration). Prototyped and **deliberately not landed** — `loop_rotate` is
opt-in, so the fold would be dead code at default `-O2`, and default-enabling
is blocked by a `FileLiveness` precision bug across backward edges. Full
analysis, minimal reproduction, and the order of work:
[`FOLLOWUP-2026-09-30-affine-exit-compare.md`](FOLLOWUP-2026-09-30-affine-exit-compare.md).

### Where I disagree with my own earlier self

The `incapable` class was reported as "tested, 33 cases pass". That was true
and misleading: the tests exercised a consumer with a hand-built input and
never the producer. **Agreement counts are not evidence of coverage** — the
number that mattered was 40 passing tests and a code path that could not
execute. The generalisable lesson, and the one now encoded in this series'
own review standard: a new semantic category must be traced from its
*producer* to the *live pipeline* before its tests are counted, and a
two-function seam needs a test that drives the glue, not just each half.
