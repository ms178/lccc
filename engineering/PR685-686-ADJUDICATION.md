# PR #685 and #686 vs this branch — red-team adjudication

**Date:** 2026-09-30
**Subjects**

| | head | commits | base |
|---|---|---|---|
| PR #685 | `d31e3e456` | 1 | `02d4c9067` |
| PR #686 | `345a5e9c5` | 2 (`67cabbee3` + fix) | `02d4c9067` |
| this branch | — | — | `02d4c9067` |

`git merge-base --is-ancestor pr/685 pr/686` → **false**: they are independent
branches. `git diff --stat pr/685 pr/686` → **6 files, +228/−11**, and the fix
commit is titled *"Close the audit findings: preheader dominance guard, honest
coverage docs, --compile-only contract"*. **So #686 is #685 plus a response to
the audit pasted below.** That single fact reorganises the whole review: most of
the audit's findings are claims about a state that #686's head has already
moved past, and the only way to know which is to test the head.

**Method.** Both trees were built (`fastbuild`, exit 0) and everything below was
produced by running something: 51-program codegen A/B, an 865-file
debug-assertions sweep, the cross-compiler Godbolt survey, interleaved
compile-time measurement, and the repo's own gates. No claim rests on reading a
diff.

---

## 0. Verdict

**#686 is the better of the two competing PRs, and it found three real defects
in my branch that I had not found.** I have adopted all three. It is not the
better *branch*: it ships a 528-line CFG-mutating pass with **zero** end-to-end
coverage, still carries the crate-wide lint suppression that let the original
miscompile ship, and still contains the two structural defects (F3, F4.1) its
own audit named.

After this session the branches produce **byte-identical codegen on all 51
benchmark programs at both `-march=x86-64-v3` and default flags**, and this
branch is a strict superset of #686's engineering: everything they fixed, plus
the gate they lack, plus the four findings they left open.

Scorecard, mine included and measured the same way:

| Dimension | #685 | #686 | this branch (before) | this branch (after) |
|---|---:|---:|---:|---:|
| Correctness of transforms | 8.0 | 9.5 | **7.0**¹ | 9.5 |
| Test engineering | 4.0 | 5.5 | 8.5 | **9.0** |
| Code structure | 6.0 | 6.5 | **8.0**² | **8.5** |
| Build-level hardening | 2.0 | 2.0 | **9.5**³ | **9.5** |
| Measurement discipline | 9.0 | 9.0 | 8.5 | **9.5**⁴ |
| Documentation honesty | 7.0 | 9.5 | 8.0 | **9.5** |

¹ unscoped x86-64 twin (F-A below) **and** the unfixed F2 dominance hole.
² F3 already extracted as `hoist_dynamic_limit`; F4.1 still quadratic then.
³ `unused_variables` enabled crate-wide; neither competitor does this.
⁴ this document's numbers, including two of my own claims I had to retract (§5).

---

## 1. What I found that neither PR nor the audit found

### F-A · my x86-64 late vectorizer twin ran **unscoped** — a 17 % codegen regression · **High, mine**

`src/passes/mod.rs` runs the late vectorizer twice on x86-64: the shared
dispatch entry, and a pre-existing "twin" (`vectorize_function_late`). I had
wrapped the first in `LateMinmaxOnlyScope` and **not the second**. The twin is
precisely the pass that reaches the guarded sums `if_convert` materialises, so
the restriction was decorative: the scoped entry declined the `zlib_ng_adler32`
guarded sum and the twin took it anyway.

Measured, `-O2 -march=x86-64-v3`:

| | instructions | memory refs | ymm/zmm refs | stack frame |
|---|---:|---:|---:|---:|
| mine, unscoped twin | 312 | 74 | 49 | `subq $584, %rsp` |
| mine, scoped twin | **259** | **57** | **24** | none |
| PR #686 | 259 | 57 | 24 | none |

Every vector-op count was exactly 2× — the twin was re-vectorising the whole
loop into a second, spilled copy (175 extra asm lines). Isolation was
unambiguous: `CCC_DISABLE_PASSES=latevec` on my binary reproduced #686's output
*exactly* (259/24), which identifies the twin as the sole cause.

**Output was correct** — all three binaries print `8c331ae0`, and at 10× the
workload `b1bf1275` — so this is code bloat, not a miscompile.

**Runtime, stated honestly:** at 10× scale, min-of-7, mine 0.2765 s / #686
0.2770 s / gcc 0.2741 s. The fix is **runtime-neutral within noise (±1 %)** and
is a **static code-size** win. I am not claiming a speedup.

In the cross-compiler survey `zlib_ng_adler32::main` went from a gap of **239**
instructions to **204** versus the best oracle (`icc=73`).

### F-B · `debug_assert!(scaled)` was over-strict and **panicked on real corpus files** · **High, mine**

The audit's §2 lists this as one of #686's good calls. It is, and my branch
still had the bug. A `-C debug-assertions=on` build of my tree, swept over 865
corpus + regression files, panicked on three:

```
thread panicked at src/passes/vectorize.rs:20040:21   (vec_no_remainder.c)
thread panicked at src/passes/vectorize.rs:20040:21   (vectorize_fp_fast_math_contract.c)
thread panicked at src/passes/vectorize.rs:20040:21   (vectorize_max_reduction_neon.c)
```

The byte-IV form reaches a scaled marching pointer without a scaled GEP and the
transform is correct there; release builds never saw it because the assertion
is compiled out. Adopted #686's `scaled || use_byte_iv` at **both** sites
(20042 and 21295). After the fix: **865 files, 0 panics**.

### F-C · **my branch had the unfixed F2 hole** · **High, mine**

The audit's F2 was about #686's *first* commit. #686 fixed it. I had not — my
`find_loop_preheader` still read:

```rust
let enters = match &block.terminator {
    Terminator::Branch(label) => *label == header_label,
    Terminator::CondBranch { .. } => …,
    _ => false,          // Switch and IndirectBranch invisible
};
```

Fixed, with the proof recorded at the site. **I could not construct a C-level
trigger**: `LCCC_DEBUG_VECTORIZE=1` reports zero dynamic-limit/preheader
activity on switch- and computed-goto-entered reduction loops, so the
vectorizer's reduction detection does not currently reach that branch. That
matches the audit's "probability is low" and I am not claiming a reproduced
miscompile. The fix is cheap defensive completeness; the new
`check_multi_entry_loop.sh` pins the shapes behaviourally at five optimisation
levels against the host compiler — the gate the audit's Task 4(c) asked for and
**neither PR shipped**.

---

## 2. The audit, finding by finding, tested against #686's head

| # | Audit claim | Verdict at #686 head | How verified |
|---|---|---|---|
| F1 | no coverage; cites two non-existent files | **half-fixed.** Docs rewritten to admit the gap (its Task 2, second option); `git cat-file -e` on both cited paths → **still MISSING** | `git cat-file -e` |
| F2 | `find_loop_preheader` misses `Switch` | **fixed** — `Switch` and `IndirectBranch` now counted | read the match arms |
| F3 | ~100 lines duplicated verbatim | **unfixed** — the marker string occurs **2×** in their `vectorize.rs` | `grep -c` |
| F4.1 | O(\|body\|²·depth) | **unfixed**, and the complexity claim is **correct** (see below) | read source; `body: FxHashSet<usize>` |
| F4.2 | one preheader dirties the module-wide set | **unfixed** | read source |
| F4.3 | added compile time unquantified | now quantified — **statistically indistinguishable** (§4) | interleaved min-of-5 |
| F5 | stale comment in `licm_no_speculative_load…c` | **fixed**, +23 lines, and well written | read the file |
| F6 | `--compile-only` trains and runs PGO | **fixed** — `if compile_only:` at line 266 inside the PGO branch | `grep -n` |
| F7 | PR description under-describes the delta | not testable from the tree | — |
| F8 | no `debug_assert!(header_idx != 0)` | **unfixed** in #686 *and* in mine → now fixed in mine | `grep` |
| F9 | `global_derived_values` is an O(n²) re-scan fixpoint | **unfixed**; claim is correct — `loop { before = len; for block … }` until stable | read source |
| F10 | pure-Python tests sit in the compiler-dependent step | **unfixed** — `ci.yml:649-650` | `grep -n` |

So of ten findings, #686's head closes **four** (F2, F5, F6, and F1's
documentation half), leaves **six** open, and one (F7) is not verifiable from
the tree.

### Where the audit over-reached

**Its F2 prescription was stronger than necessary, and it presented the stronger
version as the only correct one.** It demanded deleting the function, switching
to the predecessor-based `loop_analysis::find_preheader`, *and* adding a
`strict_cfg_dominates` conjunct. Completing the terminator match is sufficient
on its own, and provably so:

> In a natural loop every body block is dominated by the header. Take any path
> from entry to the header `H`. If it visits a body block before `H`, that block
> is dominated by `H`, so the path already passed through `H` — contradiction.
> So the path enters `H` directly from an out-of-loop predecessor, and there is
> exactly one. Hence that block lies on every path to `H`, i.e. it dominates
> `H`. ∎

#686 chose the simpler correct fix. The extra conjunct is belt-and-braces, not a
requirement, and a reviewer insisting on it as a blocker is asking for work that
the proof makes redundant.

**Its F4.1 arithmetic is right but its reasoning is incomplete.** It says "for
every body block it re-checks dominance against every body block". `.all()`
short-circuits, so a block that does *not* dominate the loop usually fails fast.
The quadratic survives for a different reason, which matters because it decides
whether the fix is worth making: `body` is an `FxHashSet<usize>`, so iteration
order is arbitrary and the header — the one block that never short-circuits —
sits at an expected position of |body|/2. Expected cost ≈ |body|²/2 walks. On a
5 000-block loop that is ~12.5 M, the same ballpark as the audit's 25 M, so the
conclusion stands on a corrected argument.

**Its biggest omission.** It rates "Risk containment 8.0/10" without noticing
that **both #685 and #686 still carry**

```rust
#![allow(dead_code, unused_variables, unused_mut, unused_assignments,
         unused_imports, unreachable_code)]
```

in `src/lib.rs`. That is the mechanism that let the original volatile
miscompile ship silently, and it is the cheapest, broadest hardening available:
with it removed, `rustc` rejects a deleted guard at the exact line, for *every*
destructured safety flag in the crate, not just the ones someone wrote a
fixture for. Neither PR touches it.

**And it never ran the compiler.** Its own table says the suites it ran need
"no compiler". Every correctness verdict in it — including "0 confirmed
miscompiles" — rests on hand-derivation. The hand-derivations I re-checked (the
min/max truth table, the preheader soundness proof) are correct; but F-B above
is a defect that only an executed debug-assertions build reveals, and it sat in
the same file the audit scored 9.0/10 for correctness.

---

## 3. Where #686 is genuinely better, adopted verbatim

1. **Twin scoping** — F-A. The single largest defect in the whole comparison and
   it was in *my* tree.
2. **Re-entrant RAII guard.** Theirs saves `previous` and restores it, plus
   `debug_assert!(!previous)` against nesting. Mine set `false` on drop and
   merely *documented* non-re-entrancy. With two sequential scopes now in
   `run_passes`, restore-previous is the only version that cannot corrupt a
   later pass on the same worker thread. Adopted.
3. **`scaled || use_byte_iv`** — F-B, both sites.
4. **F5's comment rewrite** — it tells the next reader the hoist is correct and
   must not be "fixed". That is exactly the trap.

## 4. Where this branch is better, and the numbers

* **Coverage.** #686's flagship pass has no gate. Mine asserts **7 contracts
  over 2 fixtures**: their A/B kill switch (`CCC_DISABLE_PASSES=loop_preheader`)
  and their `guarded_sum` soundness contract, plus the exact insertion count, the
  measured 2→1 in-loop-memory-operand delta, and hoisted-load-strictly-after-the
  NULL-guard ordering. Plus the new `check_multi_entry_loop.sh`.
* **F3, already done.** The dynamic-limit block is extracted as
  `hoist_dynamic_limit` with two call sites; their marker string still occurs
  2×. The audit's Task 5 is complete here and was proven behaviour-preserving.
* **F4.1, now done — and done better than prescribed.** Header-only scan, with
  the antisymmetry proof as the comment *and* a `debug_assert` that re-checks
  the full quadratic property in debug builds, so relaxing LICM's must-execute
  rule later cannot silently break the shortcut. Verified: **865 files, 0
  assertion failures** — the assert would have fired otherwise.
  Measured effect on this corpus: **none that is resolvable**, because the pass
  fires on **0 of 51** benchmark programs at default settings (`loop_rotate` is
  opt-in). It is a latent scalability fix for kernel/glibc-scale CFGs and is
  recorded as such, not sold as a speedup.
* **F8** entry-block invariant asserted at the insertion site.
* **`unused_variables` enabled**, 99 warnings across 49 files resolved, build
  clean at 0 errors / 0 warnings under `-D warnings`.
* **Oracle provenance.** `live_semvers()` reads `semver`; #686's
  `_compiler_version` reads `version`/`fullVersion`, which are null for **0 of
  1049** compilers on Compiler Explorer (`semver` is populated for 1019), so its
  cache-invalidation rule can never fire and fails open.
* **`check_volatile_destructuring.py`**, `lib_loop_bounds.sh`, and the
  negative-control volatile gates.

**Compile time (F4.3, now quantified).** My first measurement said mine was
15.8 % faster. That was noise: re-run interleaved, min-of-5 over the 51-program
corpus, mine 2.571 s vs #686 2.631 s → **−2.31 %**, with per-run stdev ≈0.8 s
(~30 %). **Statistically indistinguishable.** The retraction is the finding.

**Cross-compiler survey** (`codegen_oracle.py --rank`, `-O2 -march=x86-64-v3`,
GCC 16.2 / Clang 23.1 / ICC / ICX, 100 functions): **77 behind, 3 tied, 20
ahead**, total gap 1899 instructions. Worst: `zlib_ng_adler32::main` 277 vs
`icc=73` (gap 204), `nbody::main` 306 vs `gcc16.2=118` (188), `moving_stats`
(90), `i686_alu_chains` (86). That is the real work queue for "beat every other
C compiler", and neither PR moves it: both *refuse* the adler32 guarded-sum
reduction by design, which is why 204 instructions are still on the table.

---

## 5. Two claims of mine this session had to be retracted

Recorded because the mechanism is more useful than the correction.

1. **"My branch compiles the corpus 15.8 % faster than #686."** False — single
   non-interleaved run. Interleaved min-of-5: −2.31 % with ~30 % run-to-run
   stdev. Two compilers emitting byte-identical code do not differ by 16 %.
2. **An earlier note that the min/max `Select` truth table needed checking.**
   `diff` of the two regions: **byte-identical**. The audit's hand-derived
   16-case table is right and there was never a difference to find.

---

## 6. Landed this session

| change | file | why |
|---|---|---|
| scope the x86-64 twin | `src/passes/mod.rs` | F-A, 312→259 insns |
| terminator-complete `find_loop_preheader` + proof | `src/passes/vectorize.rs` | F-C |
| `scaled \|\| use_byte_iv`, 2 sites | `src/passes/vectorize.rs` | F-B, 3 corpus panics |
| re-entrant `LateMinmaxOnlyScope` | `src/passes/vectorize.rs` | #686's better design |
| header-only scan + debug-verified proof | `src/passes/loop_preheader.rs` | F4.1 |
| entry-block `debug_assert` | `src/passes/loop_preheader.rs` | F8 |
| new gate, 2 shapes × 5 opt levels | `tests/regression/check_multi_entry_loop.sh` | audit Task 4(c), shipped by neither PR |

**Verification run:** 51/51 programs byte-identical to #686 at two flag
settings · 865 files, 0 debug-assertion panics · `check_vec_dead_remainder`
PASS (the late map vectorization the narrowing must not kill) ·
`check_loop_preheader` 7/7 · `check_multi_entry_loop` PASS · parity
`PASS (104 commands)` · `cargo fmt --check` clean · build 0 errors 0 warnings.

## 7. Still open, in priority order

1. **`zlib_ng_adler32` gap 204 vs ICC's 73.** Both PRs refuse the guarded sum
   by design; ICC vectorises it properly (`vpsadbw`/`vpmaddwd`). The largest
   single codegen opportunity in the corpus and neither competing PR attempts it.
2. **F3 in #686** — if it lands in any form, extract the twin.
3. **F9** `global_derived_values` worklist instead of a re-scan fixpoint.
4. **F10** move `test_hot_loop_metric.py` / `test_godbolt_cache.py` out of the
   compiler-dependent CI step; they need no `lccc`.
5. **F4.2** per-function dirtying instead of module-wide.
6. `nbody` (188), `moving_stats` (90), `i686_alu_chains` (86) — next on the
   survey queue.
