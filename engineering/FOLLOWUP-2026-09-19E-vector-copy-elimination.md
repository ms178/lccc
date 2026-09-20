# Vector copy elimination: what landed, what it is worth, and the one thing left

Status: landed. Follow-up: one item, instruction-selection level, sized below.

## What landed

`src/backend/x86/codegen/peephole/passes/vector_copy.rs` — five passes, gated
individually (`vector_copy_widen|fma|bracket|prop|dead`) and running in the
phase-1 fixed-point loop ahead of `dead_pure_writes`:

| pass | what it removes |
| --- | --- |
| `widen_private_vector_copies` | legacy `movsd`/`movss` into a temporary nothing has defined yet becomes the full-width move of the same flavour, so the packed reads that follow can be served from the source |
| `reassociate_fma_accumulator` | the FMA copy bracket, by rotating the 231 encoding into the 213 (or keeping 231 with a different destination) |
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

## The one thing left: FMA form selection belongs in instruction selection

The rotation that landed handles a *bracket*: copy-in, `vfmadd231`, copy-out, all
registers. The general shape is wider, and the remaining cases are visible in the
residual copy inventory of the gate's runtime kernel (48 vector register copies
in a 60-function FP stress file):

| count | shape | why it survives | correct? |
| --- | --- | --- | --- |
| 14 | `vmovsd %xmm15,%xmm15,%xmm2` then `andpd …,%xmm2` | the VEX merge form **defines** the upper bits as zero, so widening is refused and a 128-bit read cannot be served from a 64-bit copy | yes — folding it would change observable bits |
| 11 | `movsd %xmm0,%xmm2` then `vfmsub132sd …,%xmm2` (and the copy-out-only variant `vfmsub132sd …,%xmm2` / `movsd %xmm2,%xmm0`) | the input form is 132, not 231, and in the copy-out-only variant the accumulator was placed by selection, not by a copy | no — this is real remaining waste |
| 7 | `movapd %xmm2,%xmm4` in `fma_chain` | widened copy whose consumer's width does not match | needs case-by-case review |
| 4 | `movsd %xmm2,%xmm0` then `ret` | the producer wrote a temporary; only selection can make the producer write `%xmm0` | no — same root cause as the 11 |

In the corpus the same shape occurs 4 times (68 FMA sites total: 52 in the 231
form, 16 in the 132 form, none carrying a copy-in bracket), worth 4 instructions
out of 8712 — 0.05%, which is why it is a follow-up and not a blocker.

The general rule is: given any scalar FMA with destination `T` and a scalar
copy-out `T -> D` where `T` is dead after the copy-out, rotate the encoding so
`D` is the destination — 231 when `D` holds the addend, 213 when `D` holds a
multiplicand and the addend is the memory operand (or there is none), 132 when
`D` holds a multiplicand and the *other* multiplicand is the memory operand.
The constraint that makes this fragile as a peephole is the memory operand: only
Intel `src2` may be memory, which is AT&T operand 0 in all three forms, so the
choice between 213 and 132 is dictated by which role the memory operand plays —
and `chain` shows the deeper problem, where the constant `3.0` was allocated to
`%xmm0` and so the accumulator could not be the return register at all:

```text
vaddsd .LCFP_2(%rip), %xmm0, %xmm2      gcc:  vaddsd .LC2(%rip), %xmm0, %xmm0
movsd  .LCFP_4(%rip), %xmm0                   vmovsd .LC3(%rip), %xmm1
vfmsub132sd .LCFP_3(%rip), %xmm0, %xmm2        vfmsub132sd .LC4(%rip), %xmm1, %xmm0
movsd  %xmm2, %xmm0                     (4)                                    (3)
```

Selecting the accumulator form while the value is still in the selector — and
preferring the return register as the accumulator for a tail FMA — removes both
the 11 and the 4, and is where GCC and Clang get their answer. That is an
instruction-selection change with a register-allocation interaction, not a text
rewrite, and it should not be attempted inside a peephole that has already been
proved sound on a narrower contract.

## Reproducing every number above

```sh
scripts/arena_session_restore.sh                       # toolchain, swap, git, kernel tree
cargo build --profile fastbuild --locked -j 2
tests/regression/check_vector_copy_elimination.sh      # shapes, runtime differential, ratchet
scripts/check_benchmark_outputs.sh                     # 204 differential runs vs GCC
scripts/ci_local.sh --fast                             # 48 gates
cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings
```
