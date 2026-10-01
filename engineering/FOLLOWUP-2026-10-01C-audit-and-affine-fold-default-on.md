# Session 3 — audit fixes folded in, the affine fold becomes a default-on pass, and a measurement defect in my own benchmark script

Session 2026-10-01C (third session of the day). Base: `ms178/lccc` main
`8db75621`. Series (all one patch, `ms178-1.patch`):

| commit | what |
| --- | --- |
| `992af483` | `vectorize`: refuse the FMA broadcast transform when ANY entry edge is not isolatable |
| `437063e6` | `loop-rotate`: prove the affine exit-compare fold's non-wrap obligation |
| `37dac9f3` | `loop-rotate`: fold affine exit compares in every loop, default-on |
| `e806bc06` | `loop-rotate`: classify the phi's incomings by value, not by CFG position (+ script fix) |

Everything below was measured with Callgrind or the Godbolt oracle; no
wall-clock numbers are claimed anywhere.

---

## 1. The two red-team fixes from the previous round, folded in

**P0 — stale `%ymm1` on a non-isolatable entry edge.** `fma_broadcast_entry_edges`
collected only the *rewriteable* (`Branch`/`CondBranch`) entry edges and asked
whether the resulting list was non-empty. A loop entered through a `Switch`
case or an `IndirectBranch` target therefore got its broadcast hoisted onto the
branch entries and let the other way in carry a stale `%ymm1` — the same
wrong-A miscompile the carrier contract exists to prevent, one CFG shape
further out. The walk now visits every predecessor's **full successor set**
(`Switch` cases and `IndirectBranch` targets included, via a `Terminator`
match) and answers `None` as soon as any entry edge cannot be isolated.
`hoist_fma_a_broadcast` no longer re-derives the edges: it takes the validated
`&[(usize, bool)]`, has no failure path at all, and the call sites are
`let Some(entry_edges) = … else { return 0 }`. Rationale for that split: by the
time the hoist runs, the body has already been rewritten, so a `return 0` there
would leave the IR half-transformed — the refusal must be decided *before*
anything is mutated. Verified: the Switch-entered matmul probe still
vectorizes, the vectorizer gates pass.

**P1 — the affine fold's non-wrap obligation was unproven.** `(x + C) < N` →
`x < N - C` is only equivalent while `x + C` cannot wrap, and the IR's `Add`
carries no `nsw`: a wrapping (unsigned-spelled, defined) add reaches the same
opcode as a source-level signed one, so the original justification — “signed
overflow is UB in the C source” — is a claim this pass cannot verify. The fold
now proves what it needs, per compare:

* `iv_progression` / `phi_start_and_step` resolve the arithmetic progression
  `(start, step)` of the value the compare names, from the canonical shape only:
  a phi with exactly two incomings, one of which is the phi's own increment;
* `c >= 0`, `step >= 1`, and both ends of the sequence representable — the
  first test `start + step + c` and the last reachable one
  `(N - c) + step - 1`. The sequence rises monotonically to its first value
  `>= N - c`, so every test the loop can reach lies between those two bounds.

The bound is `N - c` only when `N - c` is representable (`checked_sub`) and
inside the comparison's signed range (`signed_type_bounds`). A dynamic IV, a
non-unit step, a pointer IV, a runtime bound, a runtime seed or a second entry
keeps the unfolded (correct, longer) shape.

The proof itself was where this round's debugging went: the resolver's first
version decided which incoming was the seed by CFG position (outside the body
or from the loop header), which is the shape the *rotated clone* has but not
the shape a plain loop has. Traces:

```
[IV] Value(19) def = BinOp { dest: Value(19), op: Add, lhs: Value(Value(24)), rhs: Const(I64(1)), ty: I64 }
[IV] phi_start_and_step(24) incomings = [(Const(I64(0)), BlockId(13)), (Value(Value(19)), BlockId(14))]
     loop_body_labels = [(1, 13), (2, 14)]
```

The seed arrives from the loop's own header block (the entry block carries the
pre-header code), so the position rule classified it as a back edge and the
fold stopped firing on the canonical fixture. The fix is in §3.

---

## 2. The fold becomes a default-on pass over every loop

### Why

The fold used to run only on the clone produced by `loop_rotate`, which is
opt-in **and** refuses every nested loop (Guard E). Every filter kernel's hot
loop is the inner loop of a nest, so the fold — and the `leaq 4(...)` per
iteration it deletes — was unreachable exactly where it pays.

### Hard data (Callgrind, `-O2 -march=x86-64-v3`)

Kernel: `i + 4 < 4096` reduction inside a 2000×4112 sweep with a rotating
buffer (`tests/benchmark/programs/affine_countdown.c` (the same shape)), i.e. the nested shape:

| build | Ir | instructions/iteration |
| --- | ---: | ---: |
| GCC 16.2 | 4,094,153,302 | 5 |
| lccc, fold off | 4,913,750,450 | 6 (`leaq 4(%rdi)` per iteration) |
| lccc, fold on | 4,095,150,450 | 5 |

−818.6M Ir (**−16.7 %**) and parity with GCC; all three print
`104743755764`. The residual +1M Ir is rotation's guard block, untouched here.
The shape is now a corpus program (`tests/benchmark/programs/affine_countdown.c`,
measured at 1.0006× GCC on the ISA-matched corpus run) so the win stays
monitored.

### What the pass is

`loop_rotate::fold_affine_exit_compares`, its own phase right after rotation:
`-O2`+ and the size pipelines (`-Os`/`-Oz` — the rewrite removes an ALU op when
the temporary dies, so it is never a size regression), kill switch
`CCC_NO_AFFINE_EXIT_FOLD=1`, reporting under `CCC_DEBUG_AFFINE_FOLD=1`. No
cloning, no phi rewriting, no CFG surgery: the planner
(`plan_affine_fold`) is shared with the rotation clone, so the proof lives in
exactly one place.

### Validation (focused)

* `tests/regression/affine_loop_fold.c` — new differential corpus, 40 kernels,
  each reporting its **iteration count** as well as a checksum (a wrong fold
  shows up first as a wrong count): constant grid over `C`/`N`/start/step in 32-
  and 64-bit IVs, both ends of the type range, the accepted and refused edge of
  every proof check, `do/while` self-loops, guard-only loops, nested 2- and
  3-deep, multi-block bodies, interior compares, pointer IVs, runtime
  bound/addend/seed/step, unsigned and width-mismatched compares, non-profitable
  two-use temporaries.
* `tests/regression/check_affine_loop_fold.sh` — GCC-oracle parity across fold
  on/off × rotation on/off × `-O1/-O2/-O3/-Os`; default-on/kill-switch
  reporting; object code: `nested_two` compares the bare IV (`cmpq $4092`, no
  `leaq 4(`), `near_high` carries the folded type-top bound
  `movabsq $9223372036854775803`, `selfloop`/`while_guard` fold, and the
  runtime-bound/runtime-seed shapes still materialise their offsets.
* `check_affine_exit_compare.sh` — the rotation-clone contract now matches its
  own `[ROT]` prefix; the unrotated `f_const` must fold through the standalone
  pass instead of keeping the 5-instruction shape.
* 94 loop-family regression fixtures: PASS with GCC differential.
* `near_low`/`near_high`/`high_one_iter` are the representability boundaries in
  the *accepted* direction and are well-defined C; the *violating* directions
  can only be expressed with a zero-trip loop (any program that reaches a test
  whose `iv + c` overflows is UB), which is why they are covered by the
  differential alone and not by an asm contract.

---

## 3. Classifying a phi's incomings by value, not by CFG position

The position rule (§1) is sound but incomplete: `do { …; i++; } while (i + 4 < N)`
puts the phi, its increment and the exit test in **one block**, so the seed
edge's predecessor is the header and the rule misread the back edge. The
classification is now by value — the back edge is the incoming that is the
phi's own increment (a value defined in the loop body as `phi + Const(step)`),
the other incoming must be a constant seed — and nothing about the CFG enters
the argument, so it holds for the guard-at-top form, a self-loop and the rotated
clone alike. A seed can never be the phi's own increment (in SSA the phi's value
cannot reach an entry edge), so the classification is total; two increments and
no seed refuse the pair. `loop_header` disappears from the resolver, the
planner and the fold: the proof no longer mentions the loop's shape at all.
Reported folds in the new corpus: 26 (was 24); the two new kernels are pinned in
the gate.

---

## 4. Fresh static oracle ranking (Godbolt, `-O2 -march=x86-64-v3`)

`scripts/codegen_oracle.py --rank tests/benchmark/programs/*.c --local
target/fastbuild/lccc --oracles gcc16.2,clang,icx,icc`: 100 functions
compared, **1843 instructions of total gap to the best oracle**, 77 functions
behind. Top rows (gap = lccc insns − best oracle insns):

| gap | benchmark | function | lccc insns | best |
| ---: | --- | --- | ---: | --- |
| 203 | zlib_ng_adler32 | main | 276 | icc 73 |
| 184 | nbody | main | 302 | gcc 118 |
| 86 | i686_alu_chains | main | 195 | icc 109 |
| 86 | moving_stats | main | 217 | gcc 131 |
| 70 | glibc_strstr | main | 194 | gcc 124 |
| 64 | linux_rbtree | main | 237 | gcc 173 |
| 62 | strlen_bench | main | 196 | gcc 134 |
| 58 | expat_xml_scan | main | 152 | gcc 94 |
| 53 | sha256_transform | main | 147 | gcc 94 |
| 52 | loop_patterns | main | 231 | gcc 179 |
| 46 | conv_u8_3x3 | main | 137 | gcc 91 |
| 43 | double_reduction | main | 135 | gcc 92 |
| 43 | ring_fifo | main | 56 | gcc 13 |
| 40 | zstd_count | main | 131 | gcc 91 |
| 38 | chacha20_block | chacha20_core | 101 | icx 63 |

The tool flags 16 of the 77 rows as **inlining artifacts** (the function's call
count differs between compilers, so the gap measures an inlining decision):
zlib_ng_adler32, nbody, i686_alu_chains, glibc_strstr, struct_copy, fannkuch,
sqlite_varint, chacha20_block:main, base64_enc, reduction_vecreg,
binary_trees, fp_memfold_stencil5 and four more. Those were not acted on.

The static ranking's blind spot is documented in §5: chacha20_core (101 vs 63
insns) is *two and a half times faster* than GCC dynamically. Static counts are
triage; the verdict is Callgrind.

---

## 5. Measurement integrity: a defect in my own A/B script

`scripts/callgrind_ab.py` passed the caller's `OPT` string as a **single argv
element** and never pinned an ISA. lccc's default target enables AVX2 (19 `%ymm`
refs on `double_reduction.c` with no `-march`), gcc's default is baseline
x86-64 (0 `%ymm`). Every ratio the previous round recorded with a bare `-O2`
therefore measured the vector width, not the compilers — including the headline
“matmul 0.29×” and “double_reduction 0.29×”. The script now splits `OPT` with
`shlex` (so `-O2 -march=x86-64-v3` works), slugs the output directory from the
whole flag set, and states in its docstring that ISA matching is the caller's
job and why. This round's corpus numbers are all ISA-matched.

The same audit point applies to the **static** oracle ranking: `chacha20_core`
(lccc 101 insns vs icx 63) and `sha256_transform` (147 vs 94) look like large
codegen deficits, but dynamically (**Callgrind**, ISA-matched, same source)
lccc is far ahead of GCC 16.2 on both:

| benchmark | Ir GCC 16.2 | Ir lccc | lccc/GCC |
| --- | ---: | ---: | ---: |
| chacha20_block | 2,711,739,806 | 1,117,900,740 | **0.41** |
| sha256_transform | 3,553,748,382 | 3,384,924,827 | **0.95** |

(these are the ISA-matched numbers from the run described below; the
pre-fix run's chacha 0.43 / sha256 0.79 / double_reduction 0.29 rows are
superseded — `double_reduction` is *behind* once GCC may use AVX2.) A static count cannot see that lccc's
chacha is a 4-wide vector loop where gcc's is scalar, so **static rank rows are
a triage tool, not a verdict** — every "we are behind" claim in this document
is backed by Callgrind.

---

### The ISA-matched corpus picture, and what it costs us

Callgrind, `-O2 -march=x86-64-v3` on **both** compilers, 18 programs, lccc vs
GCC 16.2 (Callgrind Ir; `< 1.0` = lccc ahead):

| benchmark | Ir GCC 16.2 | Ir lccc | lccc/GCC |
| --- | ---: | ---: | ---: |
| chacha20_block | 2,711,739,806 | 1,117,900,740 | **0.41** |
| arith_loop | 1,310,120,862 | 1,020,117,983 | **0.78** |
| matmul | 25,997,510 | 17,427,346 | **0.67** |
| conv_u8_3x3 | 498,291 | 367,904 | **0.74** |
| sha256_transform | 3,553,748,382 | 3,384,924,827 | **0.95** |
| affine_countdown | 95,664,031 | 95,730,710 | 1.0007 |
| global_addr_pressure | 6,342,807 | 6,367,542 | 1.004 |
| fir_filter | 126,000 | 138,561 | 1.100 |
| strlen_bench | 817,912,361 | 900,414,878 | 1.101 |
| double_reduction | 243,396,316 | 280,094,778 | 1.151 |
| expat_xml_scan | 586,721,478 | 686,010,910 | 1.169 |
| zstd_count | 87,546,369 | 108,777,287 | 1.243 |
| loop_patterns | 176,501,657 | 233,060,508 | 1.320 |
| csv_field_sum | 214,999 | 303,904 | 1.414 |
| binary_search | 397,830 | 574,290 | 1.444 |
| moving_stats | 179,425 | 301,865 | 1.682 |
| ascii_case_fold | 1,189,088 | 2,522,590 | **2.121** |
| ring_fifo | 216,873 | 594,123 | **2.740** |

geomean **1.123** — with the ISA matched, lccc is ~12 % behind GCC on this
corpus, and the previous round's 0.925/0.954 geomeans were the same corpus with
lccc on AVX2 and GCC on SSE2. Two rows even flip sign (`loop_patterns` 0.88 →
1.32, `double_reduction` 0.29 → 1.15): GCC vectorizes both once it is allowed
to, and lccc's advantage there was the flag, not the compiler.

### Follow-ups, in order of measured payoff

1. **`ring_fifo` 2.74× (and the class it represents).** GCC collapses
   `main`'s 20000-iteration FIFO loop to a *closed form* — 15 static
   instructions, `imul/add/add/sub/jne` per iteration, `ring[]`, `head` and
   `tail` eliminated entirely (it proves both guards hold and the read always
   returns the previous write). lccc's loop is 14 instructions per iteration
   with two real branches and phi-copy moves (`mov %rbx,%rsi; mov %r12,%r11`).
   This is a *loop-collapsing / quadratic symbolic recurrence* transform, not a
   per-iteration fix; lccc has `quadratic_sr` but it does not recognize this
   shape. Callgrind: 216,873 vs 594,123 Ir.
2. **`ascii_case_fold` 2.12×.** `if (c >= 'A' && c <= 'Z') c += 32;` emits a
   *materialised boolean* per byte: `sub; cmp; setbe; movzbl; movsbq; test;
   mov; cmovne; movzbl` — nine instructions for a two-way select, and the
   extend chain is re-done per unrolled copy. GCC emits a compare plus a
   conditional add. The IR at that point is a real diamond (`CondBranch`), so
   the boolean appears during the backend's if-conversion/lowering, not in the
   middle end; the fix belongs next to the x86 select lowering (fold a `Cmp`
   that feeds `Setcc`→`ZExt`→`Cmov` into a single `Cmp`+`Cmovcc`). Callgrind:
   `main` 1,072,235 vs 2,408,491 Ir.
3. **`moving_stats` 1.68×, `binary_search` 1.44×, `csv_field_sum` 1.41×.**
   Small programs dominated by one loop; not yet root-caused (all three are
   inlining-equal, so the gap is inside the loop bodies). Next round: per-
   function Callgrind attribution (`callgrind_annotate`) then asm diff, exactly
   as done for `ascii_case_fold` above.
4. **`loop_patterns` 1.32×, `double_reduction` 1.15×.** Both are vectorization
   *coverage* rows: GCC 16.2 vectorizes them at `-march=x86-64-v3` and lccc
   either does not or emits a weaker shape. The vectorizer's zero-remainder
   elision (ZERO-REM-1/2) is the tool; measure whether it fires on these two.
5. **Runtime-start counted loops cannot fold** (`for (i = k; i + 4 < n; i++)`).
   The proof needs a constant seed to bound the *entry* test; with a runtime
   start the fold is still sound for every test *inside* the loop but the first
   test's representability is unknowable without a range analysis. A cheap
   sufficient condition a later round can add: if the comparison's type is
   `i32`/`u32` and the addend is small, a `Range` fact on `start` (iv_widen and
   the loop analysis already compute bounds for other passes) would unlock the
   shape without weakening the proof.
6. **Rotation for nested loops (Guard E).** The fold is now independent of
   rotation, so this is a pure branch-count question: default-enabling rotation
   for the nested case needs the `adler sz` failure mode fixed (the guard exists
   because GVN+LICM froze an outer IV to its init). Upside is one branch per
   inner iteration; risk is a documented miscompile class, so it stays
   opt-in until it has its own differential.

### Red-team audit of the round's own code (what I found and what I did about it)

The audit was run against the *new* code, not just the old, and it found real
defects:

1. **Wrong rewrite in the flipped orientation (fixed, `09829d11`).** The proof's
   planner rewrote `N op (iv + C)` as `(N - C) reversed(op) iv` — the
   comparison's own negation. Subtracting `C` from both sides preserves the
   order, so the operator must stay put. Present since ZERO-ROT-AFFINE landed
   (verified in `a9f7afb9`), so this is a pre-existing bug, not a new one.
   *Reachability mattered:* the pipelines canonicalise constant-first compares
   before the phase runs, so it never miscompiled from C — measured, not
   assumed (`for (long i = 0; 4096 > i + 4; i++)` counts 4092 iterations under
   GCC, under lccc and under lccc with rotation, before and after). It is
   fixed rather than deleted because the rotation clone folds compares copied
   verbatim, and because dead-but-wrong code is a landmine for the next pass
   that reorders operands. Covered by two unit tests that fail on the old code.
2. **A proof with no unit coverage (fixed, `09829d11`).** The whole non-wrap
   obligation was only covered end-to-end by the C corpus. `loop_rotate.rs` now
   has a `#[cfg(test)] mod tests` that builds the canonical loop by hand and
   pins both orientations, a non-unit step, the accepted type-top boundary, and
   eight refusals (dynamic seed, dynamic step, unsigned operator,
   non-representable bound, first-test and last-test overflow, negative addend,
   non-positive step). 13 tests.
3. **The seed classification rule (fixed, `e806bc06`).** The position rule
   (outside the body, or from the loop header) was sound but refused
   single-block `do/while` loops, where the seed's predecessor *is* the header.
   Classification is now by value, and the `loop_header` parameter is gone from
   the resolver, the planner and the fold — the proof no longer mentions CFG
   shape at all.
4. **The measurement script (fixed, `e806bc06`).** See §5: `callgrind_ab.py`
   gave lccc AVX2 and GCC SSE2 on the same command line. Every performance
   claim from the previous round that used a bare `-O2` has been re-measured or
   withdrawn; nothing in this document rests on the old numbers.
5. **Audit checks that came back clean** (recorded so the next reviewer does
   not have to redo them): the affine fold's `use_count` gate is a
   profitability test and stale counts can only make it *more* conservative,
   never fold twice; a compare inside two nested loops is planned once, per
   loop, from the current IR, and a second attempt sees the phi (not the
   temporary) and refuses; `plan_affine_fold` refuses a temporary whose only use
   is a terminator (use counts are instruction-only); the affold phase runs
   after the vectorizer, and vectorizer-created remainder loops have runtime
   seeds, so they are refused; the `Switch`/`IndirectBranch` refusal in the
   vectorizer is exercised by a Switch-entered matmul probe that still
   vectorizes.

Residual risks I accept, stated plainly: (a) the fold is a *silent-miscompile*
class of transform, and its safety rests on the proof being right, so the C
differential reports iteration counts and the planner is unit-tested — but a
bug in `iv_progression` for a shape nobody wrote down would still need a
differential to show up, which is why the corpus is part of the patch; (b) the
`first_test` check uses `start + step + c`, stricter than the plain-loop
requirement (`start + c`) because the same planner serves the rotated clone;
that costs a fold only when a loop starts within `step` of the type top; (c)
`-O0` does not run the phase (it sits in the `-O2`/size tier with its
neighbours), and that is a policy choice, not a proof limitation.

### Validation actually run this round (and what was deliberately not)

* the four structural gates: `check_affine_loop_fold.sh` (new),
  `check_affine_exit_compare.sh`, `check_vec_chain_exit_phi.sh`,
  `check_vec_dead_remainder.sh`;
* `affine_loop_fold.c` GCC differential (40 kernels × fold on/off × rotation
  on/off × `-O1/-O2/-O3/-Os`) plus the 94-fixture loop-family regression subset
  with `--compare-gcc`;
* the wrap probes (`tests/regression/wrap_affine_fold.c`, `tests/regression/wrap_loop_fold.c`, imported this round) against GCC;
* `scripts/ci_local.sh --fast` (rustfmt + clippy + the non-slow gates);
* Callgrind for every performance number in this document, and the Godbolt
  oracle (`codegen_oracle.py --rank`, 36 programs × {gcc16.2, clang 23.1.0,
  icx, icc}) for the static ranking.

Deliberately **not** run, per the round's "minimal validation" instruction: the
full 1300-fixture regression corpus, the slow CI gates (SSA corpus,
benchmark-output oracle, peephole whitespace invariance), the kernel-boot
harnesses, and any wall-clock benchmarking. The ledger therefore records
`ci_gate=ci_local-fast-PARTIAL` instead of a full stamp.
