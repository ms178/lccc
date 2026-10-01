# ZERO-ROT-AFFINE: the rotated exit compare folds, and the fusion accepts it

Session 2026-10-01 (session 2). Closes the blocker diagnosed in
`engineering/FOLLOWUP-2026-09-30-affine-exit-compare.md`.

## The three pieces

1. **FileLiveness backward-edge precision.**
   `src/backend/x86/codegen/peephole/passes/liveness.rs`: the analyses take
   `neutralise: Option<(usize, usize, u16)>` and answer "live after the region
   `[start, end)` relative to `at` is dropped" with
   `live_after_dropping_region` (`analyse_function` / `reanalyse_with` clone
   the state and neutralise the region's own effects instead of answering from
   a stale whole-function analysis). `refresh_span` +
   `assert_matches_full_analysis` (`CCC_VERIFY_LIVENESS_SPAN=1`) keep the
   incremental answer pinned to the full analysis, and the stale
   `#[expect(clippy::too_many_arguments)]` was removed.
2. **compare-branch fusion accepts the rotated setcc shape.**
   `.../passes/compare_branch.rs`: the gate is `setcc_dead && relay_dead` on
   the fast path, else the region-dropped `live & mask == 0`; without the
   region-relative answer the rotated latch's setcc relay looked live and the
   fusion refused (`CCC_DEBUG_CMP_FUSE=1` shows the refusal), leaving the
   per-iteration `leaq` + `cmp` + `setcc` + `test` + `jcc` chain in place.
3. **`canonicalise_affine_exit_cmps` (ZERO-ROT-AFFINE) in `loop_rotate`.**
   Rewrites the CLONED latch compare `icmp slt (add iv, C), N` →
   `icmp slt iv, (N - C)` when the offset temporary's only use is that compare
   (`use_count == 1` over the clone), the type is signed (`i8/i16/i32/i64` via
   `signed_type_bounds`), the bound is a constant, and the subtraction cannot
   overflow (`checked_sub`; `s_ovf` falls back to `cmpq $4`). The header's
   original `add`/`cmp` pair is untouched, so nothing outside the rotated latch
   changes. Opt-in with the pass (`CCC_LOOP_ROTATE=1`, `-O2+`, not `-Os/-Oz`);
   `CCC_NO_LOOP_ROTATE` kills it; `CCC_DEBUG_LOOP_ROTATE=1` reports
   `[ROT] affine exit-compare folds: N`.

## Measured effect (`f_const`: `for (int i = 0; i + 4 < 2048; i++) s += v[i];`)

Rotation + fold (4 instructions per iteration):

```asm
.LBB13:
    movzbl (%rdi, %rdx), %eax
    addq %rax, %rsi
    addq $1, %rdx
    cmpq $2044, %rdx        # folded bound, bare IV, fused jcc
    jl .LBB13
```

Without rotation (the shape the blocker measured):

```asm
    leaq 4(%rdx), %r10      # per-iteration offset materialisation
    cmpq $2048, %r10
jl ...
```

The folded compare feeds the conditional jump directly — no `setcc`, no
`test` — which is the fusion half of the fix; `f_bool`'s value-producing
compare still keeps its `setl` (it is a value), and its latch compare is now
fused.

## Gate

`tests/regression/check_affine_exit_compare.sh` (+ fixture
`tests/regression/affine_exit_compare.c`, `.flags` `-O2 -march=x86-64-v3`,
`.env` `CCC_LOOP_ROTATE=1` so the regression corpus runs the rotated path
against the GCC oracle):

1. **correctness** — GCC-oracle parity (stdout + exit status) with rotation on
   (`-O1`, `-O2`, `-O3`), rotation off, and the kill switch;
2. **the fold is counted** — `affine exit-compare folds:` > 0 rotated, 0
   unrotated;
3. **the object code is the 4-instruction loop** — rotated `f_const` has
   `cmpq $2044, %rdx` immediately feeding a conditional jump, with no
   `leaq 4(`, no `$2048` and no setcc/test; the unrotated build must show the
   `leaq 4(%rdx)` + `cmpq $2048` shape it replaces.

Discrimination: the unrotated body contains no `$2044` compare at all, so
contract 3's rotated predicate fails if the fold stops firing (checked
directly against the unrotated asm, no rebuild needed).

Wired into `scripts/ci_local.sh --fast` and `.github/workflows/ci.yml`
(parity check green).

## Notes

* The pass stays opt-in: rotation trades code size (cloned condition) for a
  shorter latch, and `-Os/-Oz` are excluded. Nothing here changes default
  codegen; the liveness fix only makes more fusions *acceptable* on shapes that
  were already being analysed.
* `f_unsigned` (unsigned `i + 4 < N`) is in the corpus as the refusal case:
  the fold is signed-only, so that shape must merely stay correct.

---

## Session 3 addendum (2026-10-01C): the non-wrap obligation is now proven, and the fold is default-on for every loop

**The obligation.** `(x + C) < N` → `x < N - C` is only equivalent while
`x + C` cannot wrap. The IR's `Add` carries no `nsw` — a wrapping
(unsigned-spelled, defined) add reaches the same opcode as a source-level
signed one — so the original justification (“signed overflow is UB in the C
source”) was a claim this pass could not verify. That is a real hazard, not a
formality: `(int)(u + 4u) < 100` over a wrapping `u` compiles to the *same*
`Add`+`Slt` shape, and there the two spellings differ.

**The proof.** `plan_affine_fold` (single source of the obligation, shared by
the rotation clone and the standalone phase) now requires, per compare:

1. the compared value's progression resolvable from a canonical phi:
   exactly two incomings, one of which is the phi's own increment
   (`phi + Const(step)` defined inside the loop), the other a **constant**
   seed. Classification is by *value*, never by CFG position — that is what
   makes it hold for the guard-at-top form, for a single-block self-loop and
   for the rotated clone alike (`e806bc06`);
2. `C >= 0` and `step >= 1`;
3. both ends of the sequence representable in the comparison's signed type:
   the first test `start + step + C` and the last reachable one
   `(N - C) + step - 1`. The sequence rises monotonically to its first value
   `>= N - C`, so every test the loop can reach lies between them and the
   wrapped and unwrapped compares agree on all of them;
4. `N - C` itself representable (`checked_sub`) and inside
   `signed_type_bounds`.

Refusal is the safe degradation and is pinned in the corpus: dynamic IV,
non-unit step (unless provable), pointer IV, runtime bound, runtime seed,
runtime step, unsigned compare, width-mismatched compare, a two-use temporary,
and any loop whose entry test cannot be bounded.

**The standalone phase.** The fold no longer depends on rotation, which is
opt-in and refuses nested loops (Guard E) — the shape every filter kernel's hot
loop has. `loop_rotate::fold_affine_exit_compares` runs right after rotation,
default-on at `-O2`+ and on the size pipelines, kill switch
`CCC_NO_AFFINE_EXIT_FOLD=1`, reporting under `CCC_DEBUG_AFFINE_FOLD=1`. It is a
pure expression rewrite: no cloning, no phi rewriting, no CFG surgery, so the
only new risk is the proof's correctness — which is why the proof is one
function and the corpus reports iteration counts.

**Measured** (`-O2 -march=x86-64-v3`, Callgrind, nested `i + 4 < 4096`
reduction over a 2000×4112 sweep): GCC 4,094,153,302 Ir / lccc without the
fold 4,913,750,450 (6 instructions per iteration, `leaq 4(%rdi)`) / lccc with
it 4,095,150,450 (5 instructions per iteration, parity with GCC), identical
output. `tests/regression/affine_loop_fold.c` +
`tests/regression/check_affine_loop_fold.sh` hold the line; the rotated path's
own contract stays in `check_affine_exit_compare.sh` under the `[ROT]` prefix.
