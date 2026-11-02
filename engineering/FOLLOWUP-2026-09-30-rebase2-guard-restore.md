# Rebase onto `f9bef39b` (#687/#688) — and a miscompile my own refactor had introduced

Date: 2026-09-30
Base: `f9bef39b` (PR #687 VEX/EVEX coverage + mode-aware encoding oracles, PR #688
loop/volatile/oracle coverage) — previous base was `71e3b1ae`, still an ancestor.
Result: `ci_local.sh --fast` **121 passed / 0 failed / 5 skipped**; `rustfmt` clean;
`clippy -D warnings` clean; 153/153 codegen comparisons byte-identical to main.

## 1. The defect: a guard my own refactor dropped

This is the substantive finding of the session and it is a **latent miscompile I
introduced**, not one I found in someone else's code.

When I extracted `hoist_dynamic_limit` out of the two call sites in
`src/passes/vectorize.rs` (the "F3 dedup", which removed a duplicated ~120-line
block), I dropped the `.filter(...)` on `find_loop_preheader` entirely. Base
`71e3b1ae` and upstream `f9bef39b` both gate the hoist on **two** preconditions:

```rust
find_loop_preheader(func, header_idx, &loop_blocks)
    .filter(|&pre_idx| {
        let cfg = CfgAnalysis::build(func);
        strict_external_value_available(func, &cfg, &loop_blocks, pre_idx, *limit_val)
            && strict_cfg_dominates(&cfg, pre_idx, header_idx)
    })
```

My extracted function called `find_loop_preheader` with **no filter at all**. It
therefore hoisted the loop bound into a preheader without checking that

1. the bound's definition is actually available at that preheader (otherwise the
   hoisted use precedes its def — a use-before-def), and
2. the preheader dominates the header (otherwise the bound is defined on some
   paths and not others).

### Why nothing caught it

The corpus never exercised the rejecting path. A/B against main over 51 programs
x 3 flag settings is **153/153 byte-identical** both before and after the fix.
The filter almost never rejects on real code, so every shape in the corpus took
the accepting path and produced identical output either way.

**Generalised lesson, and it is the one worth keeping:** a byte-identical codegen
A/B proves the *accepting* path of a fail-closed guard is unchanged. It says
nothing about the *rejecting* path, which is the entire reason the guard exists.
Guards must be tested by a test that makes them fire. Recording this so the next
refactor of a `.filter(...)` is not defended by "the codegen is identical".

### What was restored

Both conjuncts, in the single shared copy, with the full proof comment. Upstream's
#688 comment is the better argument and is carried verbatim: `strict_cfg_dominates`
is provably redundant **today** (the 5-step proof is recorded next to it), but it
is the *runtime form* of proof-step 4, so unlike the proof it stays true if
`find_loop_preheader` is ever weakened to admit several entries — which is exactly
when the proof stops holding. Cost is nil (the CFG is built lazily inside the
closure); a redundant refusal costs one missed optimisation, a missing one costs a
miscompile.

**I am retracting my earlier position.** I had argued the dominance conjunct was
provably unnecessary and could go. That was correct about today's
`find_loop_preheader` and wrong about maintenance: the argument establishes that
the check cannot fire, not that it is worth deleting. Upstream's framing —
keep it, because the proof it mirrors is the fragile part — is the right one.

Also removed the dedup's dead `_limit_ty: IrType` parameter. The body recomputed
the exit compare's type itself, so both call sites read `func` a second time to
pass a value the callee ignored; the two now-dead `limit_ty_dyn` computations went
with it. This tree has `unused_variables` enabled (the crate-wide
`#![allow(...)]` is removed by this branch), so leaving them would not have built.

## 2. Rebase decisions, and where I was wrong mid-flight

14 files overlapped; 4 auto-merged. Three fixes I had made had been landed
independently upstream with equivalent reasoning — the `endswith(".tmp")` cache-stats
bug, the empty-oracle-set fail-open in `run_linker_tests.py`, and strict UTF-8
decoding in `godbolt_cache.py`. Those take **upstream's merged form**; re-asserting
my wording would be churn with no content.

Two decisions worth recording because both were wrong the first time:

**2a. `check_multi_entry_loop.sh` — I retired it, then had to put it back.**
Upstream's new `check_loop_preheader.sh` refuses `switch_entered` and
`computed_goto`, which are the same shapes my script covers, so I initially
concluded mine was a duplicate and unwired it. That was wrong: the two assert
different things and fail for different reasons.

| gate | what it actually asserts | catches |
| --- | --- | --- |
| `check_loop_preheader.sh` (upstream) | the two multi-entry shapes are **byte-identical** with the pass on and off | the pass firing when it should decline |
| `check_multi_entry_loop.sh` (mine) | **runs** the shapes and diffs the exit status against the expected value and the host compiler | the generated code being wrong, whatever the cause |

A structural refusal check and a runtime differential check are not the same
statement. The F2 finding from the PR #686 audit needs both: the pass could keep
declining correctly while something downstream breaks the same shapes, and only
the runtime arm would notice. Both are wired in `ci_local.sh` and `ci.yml`, with a
comment at the wiring site saying why they are not duplicates — the comment is
there so the next person does not make my mistake.

**2b. The `.filter()` guard needed a gate, and the gate needed a mutation test.**
Upstream's `check_loop_preheader.sh` became canonical (6 contracts, including an
idempotence contract I lacked, plus a correction to my framing — see §3). I added
one contract to it rather than shipping a second gate:

> **2b — magnitude.** Contract 2 asserts a *direction* (the load moved out of the
> loop). That is necessary but not sufficient: a transform could hoist the load
> and simultaneously push other traffic *into* the loop — a spill, a
> rematerialised address, a widened accumulator slot — and still satisfy
> contract 2 while making the steady state slower. Counting every memory operand
> in the loop body pins the magnitude instead. Measured on this shape: **2 -> 1**.
> Asserted as exact equality, not `<=`, because a number that silently drifts is a
> number nobody is watching.

Mutation-verified: forcing `loop_preheader::enabled()` to `false` fails **4**
contracts, including 2b (`"the pass did not reduce in-loop memory traffic
(2 -> 2)"`), and the gate exits 1. An assertion that was not seen to fail is not
evidence.

## 3. Upstream corrected one of my claims — accepted

My backlog entry asserted the pass *hoists* the spec's original first shape
(`int f(const int *c,int n){int t=0;for(i<n;i++)t+=c[0];}`). Upstream's entry
measures it and finds the opposite: it is **byte-identical with the pass on and
off** because LICM already hoists the unguarded load, so the pass has nothing left
to unlock. The gate therefore asserts *refusal* for that shape, which is the true
behaviour. Upstream's version is kept. This is the same failure mode as §1 in a
milder form — I described a shape by what I expected the pipeline to do rather
than by what it does.

## 4. Environment / process failures, all self-inflicted

Recorded so they stop recurring:

- **Ran the full CI before reinstalling the multilib packages** after the sandbox
  wipe, producing 6 phantom failures (`reassoc-latency`, `copy-alias-sizes`,
  `notype-code-routing`, `i686-integer-isa-parity`, `map-i64-two-lane`,
  `linker-suite`). All 6 passed untouched once `gcc-multilib`, `g++-multilib` and
  `libc6-dev-i386` were installed and `-m32` was verified working for both C and
  C++. The recovery order is swapfile -> rustup -> clone -> remote -> **multilib
  -> CI**. My own notes said this and I skipped the step.
- **`git apply --check ... | head -5 && echo "APPLIES CLEAN"` printed "APPLIES
  CLEAN" on a failed apply**, because `&&` binds to `head`'s status, not
  `git apply`'s. The same `PIPESTATUS` trap is already in my notes; it recurred
  because the failing command was in a pipeline. Use `if git apply --check ...`,
  which does not pipeline.
- **`grep -rl "^<<<<<<<"` missed an indented conflict marker** in
  `.github/workflows/ci.yml`, leaving invalid YAML that two gates tripped over.
  Conflict markers inherit the indentation of the surrounding block. The check has
  to be `grep -rnE "^[[:space:]]*(<<<<<<<|>>>>>>>|\|\|\|\|\|\|\|)"`.

## 5. Deliverable state

- `/home/user/ms178-1.patch` regenerated against `f9bef39b`, verified with a
  non-pipelined `git apply --check` in a clean `gh/main` worktree, then a full
  `git apply` (93 files land).
- `artifacts/.base_ref` was the cause of the previous "does not apply" report: it
  cached `71e3b1ae`, so the snapshot script kept diffing against the *old* base
  and stamping `APPLIES-CLEAN` against the wrong commit. Updated to `f9bef39b`.
- Slow gates deliberately **not** run locally; they are deferred to GitHub CI per
  instruction.

## 6. Open, unchanged from last session

- **P0 `IVOPTS-1`** — `nbody.c` `.LBB7` is 110 instructions against GCC's 14
  (7.9x) with **zero** stack references, so it is not register pressure:
  `imulq $56` x2 (index x stride, no IV strength reduction), `leaq bodies(%rip)` x2
  (invariant static base rematerialised), `cmpl $5000000` x3, FP spill x2. 56 of
  110 instructions are data movement. Cheapest-first order: hoist invariant `leaq`
  bases -> strength-reduce `idx*stride` and prefer displacement addressing ->
  coalesce scalar FP moves. Do not start from the register allocator.
- Corpus census still stands: `movsd` 3.2x GCC, `leaq sym(%rip)` 1.59x,
  `imul $const` 1.58x; excluding the 4 recursion-heavy files LCCC is 15.8%
  *larger* than GCC, larger on 37/47, median ratio 1.17. The headline
  "12.5% smaller" total is dominated by 4 files and must not be quoted.
- No runtime/PMU claims: no PMU on this box. Every static count here is a
  screening metric only.
