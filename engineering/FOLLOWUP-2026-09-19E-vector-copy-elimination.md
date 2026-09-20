# Vector copy elimination: what landed, what it is worth, and what is left

Status: landed in two increments (PR #566/#567 and the follow-up below).

## What landed

`src/backend/x86/codegen/peephole/passes/vector_copy.rs` — six passes, gated
individually (`vector_copy_widen|fma|retarget|bracket|prop|dead`) and running in the
phase-1 fixed-point loop ahead of `dead_pure_writes`:

| pass | what it removes |
| --- | --- |
| `widen_private_vector_copies` | legacy `movsd`/`movss` into a temporary nothing has defined yet becomes the full-width move of the same flavour, so the packed reads that follow can be served from the source |
| `reassociate_fma_accumulator` | the copies around a scalar FMA — a run of copy-ins, a copy-out, or both — by re-encoding the instruction (132/213/231) so the register the code wanted is its destination; `fma_forms.rs` holds the role algebra |
| `retarget_vex_scalar_result` | a VEX arithmetic/logical producer whose result is only ever copied elsewhere writes that register directly (its destination is a plain output, unlike an FMA's) |
| `coalesce_vector_brackets` | `copy-in / ops / copy-out` around a private temporary: the destination family is renamed to the source family and both copies go |
| `propagate_vector_copies` | forward substitution of a copy's source into the reads of its destination, width-checked at every step |
| `eliminate_dead_vector_copies` | a copy whose destination cannot be observed again, asked of `FpLiveness` rather than a textual scan |

The GP side of this was already solved; the vector side was declined by
construction, because `is_xmm_family` exists to keep families 24..39 out of the
GP `reg_refs` bitmask and `copy_propagation`, `coalesce_register_copies`,
`relay_and_lea` and `dead_writes` all bail out on a register they cannot
represent.

## Measured, on real compiler output

`-O2 -march=x86-64-v3`, instruction counts including `ret`, GCC 16.2 as the
reference on the same host:

| source | before | after | gcc |
| --- | --- | --- | --- |
| `__builtin_floor` / `ceil` / `trunc` / `rint` / `floorf` | 3 | **1** | 1 |
| `double t = a; t += b; return t;` | 4 | **1** | 1 |
| `double t = a; t *= b; return t;` | 4 | **1** | 1 |
| `__builtin_fma(a, b, c)` | 7 | **1** (`vfmadd213sd`) | 1 (`vfmadd132sd`) |
| `__builtin_copysign(x, y)` | 8 | **3** | 4 |

Copysign is one instruction better than GCC and equal to Clang 23.1; the
rounding, two-operand and FMA forms are at parity with GCC, Clang and ICX.

Over the archived oracle corpus (`engineering/evidence/godbolt/s59-rank`, 51
benchmarks, records for gcc 16.2 / clang 23.1.0 / icc 2021.10.0 / icx):

```text
TOTAL 8757 -> 8712 instructions (-45, -0.51%)
ratio to best-of-oracles 1.5111 -> 1.5034
regressions: none (41 of 51 unchanged, 10 improved)
reduction_vecreg   1.010 -> 0.981   (below parity: better than every oracle)
vector_remainder   1.000 -> 0.990   (below parity)
libm_round_family  234 -> 217       (1.872 -> 1.736)
```

Validation, all of it run, none of it assumed:

* `cargo test`: 2994 passed, 0 failed (35 in `vector_copy` alone).
* `scripts/check_benchmark_outputs.sh`: PASS=204 FAIL=0 — the whole benchmark
  corpus compiled, executed and diffed against the GCC oracle. This is the gate
  that exists because an induction-variable widening once passed 563 regression
  tests and silently changed SQLite's output.
* `tests/regression/check_vector_copy_elimination.sh` (new, registered in
  `ci_local.sh` as `vector-copy-elimination`): 23 checks — the instruction
  shapes above, a runtime differential against GCC over kernels built to hit
  every legality rule (live-in temporary, packed read of a scalar copy, call and
  branch inside a bracket, two-register FP return, `%ymm` aliasing `%xmm`, mixed
  float/double, dead and live results side by side, all four FMA families,
  AVX-512 where the host has it), and a corpus instruction ratchet. It asserts
  that the differential really contains 213-rotated FMAs, because a bit-exact run
  that never executes the new code proves nothing.
* `scripts/ci_local.sh --fast`: 48 passed, 0 failed.
* `cargo clippy --all-targets --profile fastbuild --locked -- -D warnings`: clean.
* `cargo fmt --all -- --check`: clean.

## Second increment: FMA form algebra, negation, self-moves

The first increment left "general FMA form selection" as an
instruction-selection item.  Measured again, it is not one: every case in the
inventory is a peephole re-encoding once the three forms are treated as what
they are — one computation with the roles permuted.  `fma_forms.rs` is that
table, with an exhaustive test (4 families × 3 forms × 2 widths × 2 memory
placements × every choice of result register = 120 re-encodings, each checked
for role preservation, destination, memory-operand position and round-trip).

What changed, `-O2 -march=x86-64-v3 -ffp-contract=fast`, GCC 16.2 on the same
host:

| source | before | after | gcc |
| --- | --- | --- | --- |
| `a * b + c` | 5 | **1** `vfmadd132sd %xmm1, %xmm2, %xmm0` | 1 (identical text) |
| `x * x + c` | 4 | **1** | 1 |
| `__builtin_fma(a, *p, c)` | 3 | **1** `vfmadd132sd (%rdi), …` | 1 (identical text) |
| `-x` (double / float) | 5 | **1** `vxorpd mask(%rip), %xmm0, %xmm0` | 1 (identical) |
| `double t = -a; return sin(t) + t;` | 10 | **6** | 8 |
| `chain` (the gate's residual) | 4 | **3** | 3 |
| nbody `bodies[i].mass` FMA staging | `movsd` + FMA | FMA with memory operand | same |

Three defects fixed on the way, each found by the census rather than by
inspection:

* `fold_fma_memory_src2` only folded a load feeding AT&T operand 0 of a 231
  form.  A load feeding operand 1 (nbody, four sites) or any operand of a
  132/213 form was left as a `movsd`; now the instruction is re-encoded so the
  loaded role sits at operand 0, in whichever form allows it.
* scalar FP negation went through the GPR accumulator: `movq %xmm, %rax;
  movabsq $sign, %rcx; xorq; movq %rax, %xmm; movsd` — five instructions and
  two domain crossings where every oracle emits one `vxorpd`.  Fixed at the
  emitter (`emit_fp_neg_direct`), which also unblocks the contraction of
  `-(a*b) - c` shapes that the GPR detour had hidden from the FMA fuser.
* `eliminate_vector_self_moves` did not know the legacy scalar spellings
  (`movsd %xmm0, %xmm0`) or the VEX three-operand one; five survived in the
  corpus.

Corpus (51 archived benchmarks): **8646 → 8627 instructions**, ratio to
best-of-oracles **1.4619 → 1.4591**, vector register copies **40 → 31**, GPR
sign-flip negates **4 → 0**, no regressions (`libm_round_family` 217 → 207,
`nbody` 319 → 312, `struct_copy` 138 → 136).  Gate budgets are pinned to the
new numbers; the runtime kernel's copy budget is 42 (was 48).

Validation: `cargo test` 3036 passed / 0 failed; the gate 35 checks including a
new FMA-algebra kernel (builtins bit-exact against GCC; contracted kernels
checked against their builtin twins inside the same binary, since
`-ffp-contract=fast` output is not comparable across compilers; per-encoding
anti-vacuity), and a negation kernel bit-exact against GCC at seven flag sets
(`-O0`…`-O3`, v2/v3, with and without contraction) that also asserts zero GPR
sign-mask round trips; `check_benchmark_outputs.sh` 204/204; clippy `-D
warnings` clean; rustfmt clean.

## What is left

Two items, both now register allocation rather than encoding:

* **Copy-in with no copy-out** (`movsd %S, %T; vfmadd… %T`, result staying in
  `T`): a loop accumulator the allocator did not coalesce.  The result's home is
  right; the staging is the waste.  4 corpus sites.
* **`fma(-a, b, -c)` builtin sign variants**: the negations are separate
  `vxorpd`s feeding a `vfmadd`; GCC folds them into `vfnmsub`.  That is a
  simplify-level rewrite of `Neg` into the FMA's sign family, not an encoding
  question.  Zero corpus sites; the gate's algebra kernel excludes these from
  its one-instruction list and says so.

The residual 31 corpus copies and the runtime kernel's 42 are inventoried by
the census scripts in this document's history; 14 of the kernel's are VEX
merge-form copies whose zeroed upper bits a 128-bit `andpd` genuinely reads,
and they are correct to keep.

## Reproducing every number above

```sh
scripts/arena_session_restore.sh                       # toolchain, swap, git, kernel tree
cargo build --profile fastbuild --locked -j 2
tests/regression/check_vector_copy_elimination.sh      # shapes, runtime differentials, ratchet
scripts/check_benchmark_outputs.sh                     # 204 differential runs vs GCC
scripts/ci_local.sh                                    # every gate
cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings
```
