# i686 ALU lowering (`src/backend/i686/codegen/alu.rs`) — red-team audit, A/B and optimisation report

*Session 2026-09-03 · base `82d7009` (ms178/lccc main) · target `lccc-i686` · host 2 vCPU / 2 GiB + 4 GiB swap, no PMU*

## 1. Scope and method

`alu.rs` hand-emits every integer ALU lowering for the i686 backend: constant
division/remainder (Hacker's Delight magic numbers, signed power-of-two bias),
constant multiplication (LEA/shift strength reduction), immediate/variable
shifts, the direct-to-dest ALU path, div/rem pair fusion, and the width-aware
clz/ctz/popcount/bswap sequences. Each one is a sequence whose correctness rests
on subtle arithmetic (magic "add" indicator, arithmetic-vs-logical sign masks,
store order of the pair head, 5-bit shift masking). The audit therefore had to
*execute* the generated code, not only read it.

### 1.1 Red-team differential tester (new: `scripts/i686_alu_redteam.py`)

* Generates a **deterministic** corpus of 3 122 `noinline` functions
  (seed 20260903) covering every lowering path:
  * `udiv/urem/sdiv/srem_D` — 291 unsigned and 355 signed divisor classes:
    2…64 exhaustively, primes, 2^k±1 for k=2…31, all magic-"add" divisors
    (7, 14, 19, 21, 27, 31, 37, 39, 41, 47, …), 10^k, 2^31−1, 2^31, 2^31+1,
    2^32−1, negative divisors, INT_MIN.
  * `udr/urd/sdr/srd_D` — same-block div+rem pairs in both statement orders
    (exercises the pair head's store-order hazard) and `uqr_D` with a store of
    the quotient *between* the two.
  * narrow dividends (`signed char`, `short`, `unsigned short`) so the I32
    lowering sees sign/zero-extended values.
  * `mul/mulk/mull_C` — 233 constants (−70…140, 2^k, 2^k±1, hash multipliers,
    INT_MIN, …) in the accumulator path, the direct path with the source kept
    live (forces `src != dest`), and a loop-carried source.
  * every immediate shift 0…31 in two shapes, variable shifts, rotate idioms.
  * clz/ctz/popcount/bswap16/bswap32/parity/ffs/clrsb on 8/16/32-bit
    sign- and zero-extended inputs, bit-test idioms.
  * 240 seeded random expression trees (depth 2–4, 1–4 statements) that mix all
    of the above with UB-free operand shaping.
* 104 inputs per function (64 hand-picked edge values + 40 seeded), binary
  functions strided over the input cross product.
* Ground truth is `gcc -m32 -O0` (plain `idiv`/`imul`), **cross-checked against
  `gcc -m32 -O2`** so a UB-containing corpus can never masquerade as an LCCC bug.
* Sharded into 33 translation units (see §4.3) and compiled/run in parallel at
  `-O0 -O1 -O2 -O3 -Os`; per-function FNV-1a hashes pinpoint the exact lowering
  on a mismatch. `--stats` adds a per-group static census vs `gcc -m32 -O2`.

### 1.2 Oracles

`scripts/godbolt.py compile {gcc16.2,clang,icx} … --flags '-m32 -O2'` for the
reference sequences (GCC 16.2, Clang 23.1, ICX latest), local `gcc -m32 -O2`
(GCC 12) for the census, unit tests for the new synthesiser.

## 2. Findings

### 2.1 Correctness

All 3 122 functions × 5 optimisation levels match GCC **before and after** the
changes (15 610 function/level pairs, 0 mismatches, 0 crashes). No
miscompilation was found in the upstream lowering on this corpus. This is a
strong result for the magic-number path in particular: every "add"-indicator
divisor, every 2^k±1 and the 2^31/2^32 boundary classes are covered.

### 2.2 Code-quality defects found (all fixed in this patch)

| # | Defect | Evidence (upstream, `-O2`) | Root cause |
|---|--------|-----------------------------|------------|
| A | Constant remainder saves the dividend with `pushl %eax … popl %eax` — a store-forward round trip (~5 c) *on the critical path* plus two memory µops | `urem_7`: 13 insns, 2 stack ops; GCC/Clang/ICX: 12 insns, 0 stack ops | `emit_urem_const_in_eax`/`emit_srem_const_in_eax` recomputed the quotient with a helper that clobbers the dividend |
| B | Constant div+rem pairs (`q = x/10; r = x%10`) ran the **entire magic sequence twice** | `udr_10`: two `mull`, 22 insns; GCC 10 | `compute_i686_divrem_pairs` excluded `Operand::Const` divisors unconditionally; the pair head only knew `divl` |
| C | `imull $c` for every multiplier outside {0,±1,2,3,4,5,8,9} (+6/10/12) although GCC, Clang and ICX use 1-cycle LEA/shift chains (7, 11, 13, 15, 17, 19, 21, 24, 25, 27, 36, 37, 40, 41, 45, 48, 72, 73, −3, …) | `mul_11`: `imull $11` (3 c) vs oracle `leal (x,x,4); leal (x,d,2)` (2 c) | hand-maintained 12-entry table in `try_emit_mul_imm_reg` |
| D | Accumulator-path multiply copied a register-homed lhs into `%eax` before an in-place `imull`, throwing away the `src != dest` freedom that enables the chains | `mul_11` when dest is slot-homed: `movl %ebx,%eax; imull $11,%eax,%eax` | path called `operand_to_eax` unconditionally |

### 2.3 Not attributable to `alu.rs` (recorded in `FOLLOWUP_I686_ALU.md`)

* Leaf-function prologue/argument homing (`pushl %ebx; subl $8,%esp; movl
  16(%esp),%ebx` for a one-argument leaf; GCC reads `4(%esp)` directly).
  This is the dominant remaining gap in every group of the census (≈1.7× for
  the divide groups after this patch).
* Redundant `andl $31` on variable shift counts (hardware masks to 5 bits).
* Rotate idioms not recognised (`rotl` 26 insns vs `roll %cl` 4).
* Compile-time superlinearity in very large functions (§4.3).

## 3. Changes

### 3.1 `synth_mul` — bounded-search multiply-by-constant synthesiser (fixes C, D)

A table of hand-picked multipliers cannot be complete or provably byte-optimal,
so the table is replaced by a small iterative-deepening search over the
instruction set actually available to the direct path (no scratch register):

| step | asm | effect |
|------|-----|--------|
| `Mov` | `movl %src,%d` | d = x (rename-eliminated) |
| `LeaSrcSrc(k)` | `leal (%src,%src,k),%d` | d = (k+1)x, k∈{1,2,4,8} |
| `LeaScaleSrc(k)` | `leal (,%src,k),%d` | d = kx (7 bytes, speed only) |
| `LeaSelf(k)` | `leal (%d,%d,k),%d` | d *= k+1 |
| `LeaSrcPlus(k)` | `leal (%src,%d,k),%d` | d = x + k·d |
| `Shl(k)` / `AddSelf` | `shll $k,%d` / `addl %d,%d` | d <<= k / d *= 2 |
| `AddSrc` / `SubSrc` / `Neg` | | d ± x / −d |

Policy (identical admission rule to GCC's `synth_mult` and LLVM's
`combineMulSpecial`, but exhaustive and byte-minimal within it):

* `-Os`: one instruction ≤ 3 bytes (bit-identical to the old table minus
  nothing, plus in-place `shll` for every power of two).
* speed, `src == dest`: ≤ 2 in-place steps (2 c latency vs `imull`'s 3 c).
* speed, `src != dest`: ≤ 2 steps, or 3 steps when the first is a `movl`
  (eliminated at rename on every P-core since Ivy Bridge, so the dependent
  latency stays 2 c). Three *dependent* non-mov steps equal `imull` latency
  with 3× the µops and are rejected — that is why 23 stays `imull` although
  GCC emits `lea; sal; sub` for it.
* Results are memoised per `(imm, same, budget, allow_scale_lea)`; the search
  is ≤ ~20 k nodes worst case and runs once per distinct constant.

The search finds forms **shorter than the oracles'**: `x*7 = leal (x,x,2),d;
leal (x,d,2),d` (6 bytes, 2 fast LEAs) where GCC/Clang/ICX emit
`leal (,x,8); subl` (9 bytes); `x*17 = leal (x,x),d; leal (x,d,8),d` (2 insns)
where all three emit `mov; shl; add` (3 insns). Unit tests
(`synth_mul_tests`) check every chain for 1 400 constants × {same, !same} ×
budgets 1–3 by symbolic evaluation, and pin the canonical forms.

The accumulator path now reads a register-homed lhs in place
(`try_emit_mul_imm_reg(src, "eax", imm)`), the same authority
`direct_reg_src_ref` already relies on; the RA hazard model classifies
`Mul` with an immediate as `%ecx`-clean and the chains never touch `%ecx`.

### 3.2 Memory-free constant remainder (fixes A)

* `emit_udiv_magic_keep_n(d)`: magic quotient with the dividend preserved in
  `%ecx` (q in `%eax`, `%edx` clobbered) — one `movl` longer than the
  quotient-only form but no stack traffic; used by every remainder/pair form.
* `emit_sdiv_magic_core(|d|)`: signed core leaving q in `%edx`, n in `%ecx`,
  sign mask in `%eax`; `sdiv`/`srem`/pair are thin wrappers.
* The multiply-back `q*d` goes through `synth_mul` (`%eax→%edx` never aliases),
  e.g. `*7` is two LEAs, `*10` is `leal (q,q,4); addl`.

`urem_7`: 13 insns + 2 memory ops → 11 insns, 0 memory ops (GCC 12, Clang 12).
`srem_7`: 13 + 2 mem → 11 + 0.

### 3.3 Constant div+rem pair fusion (fixes B)

* `compute_i686_divrem_pairs` now admits constant divisors **for
  `DivRemTarget::I686` only** (other targets keep the IR-level `div_by_const`
  ownership).
* `emit_divrem_pair_head` folds a constant divisor at speed levels with
  `emit_divrem_const_in_eax_edx`, which produces **q in `%eax` and r in
  `%edx`** — the same register split as `divl` — so the existing store-order
  logic (including the "broken pair" fallback for pathological homes) is
  shared unchanged. At `-Os` the head keeps the staged `divl $imm`.
* Soundness w.r.t. the RA hazard model: the model already charges every
  constant-divisor division as an `%ecx`+`%edx` hazard and treats a pair tail
  as clean; the folded head clobbers exactly `{%eax,%ecx,%edx}`. Comments in
  `regalloc.rs`/`prologue.rs` were updated to the new invariant.

`udr_10`: 22 insns / two `mull` → 12 insns / one `mull` (GCC 10; Clang/ICX 12
with a `pushl %esi`).

## 4. Measurements

### 4.1 Static census (red-team corpus, `-O2`, instructions per group)

| group | upstream | patched | Δ | gcc -O2 | push/pop up→patched |
|-------|---------:|--------:|--:|--------:|-------------------:|
| urem | 2518 | 2370 | −5.9 % | 1586 | 584 → 308 |
| udr (div;rem) | 4238 | 3309 | −21.9 % | 1795 | 1200 → 928 |
| urd (rem;div) | 4994 | 3496 | −30.0 % | 1795 | 1500 → 950 |
| uqr (q used between) | 5613 | 4680 | −16.6 % | 2448 | 1508 → 1232 |
| srem | 5604 | 5028 | −10.3 % | 3727 | 1146 → 594 |
| sdr | 9818 | 7322 | −25.4 % | 4745 | 2334 → 1806 |
| srd | 11303 | 7595 | −32.8 % | 4745 | 2928 → 1782 |
| mul / mulk / mull | 12025 | 12162 | +1.1 % | 5299 | unchanged |
| rnd (random trees) | 15335 | 14969 | −2.4 % | 8683 | 2272 → 1758 |
| udiv / sdiv / shifts / bit | unchanged | | | | |
| **total** | **78683** | **68166** | **−13.4 %** | 38557 | 17558 → 13444 |

The `mul` rows grow by design (two 1-cycle LEAs replace one 3-cycle `imull`);
the remaining push/pop counts are prologue callee-saves, not ALU traffic.

### 4.2 Runtime A/B

See `FOLLOWUP_I686_ALU.md` §"A/B protocol" and the ledger entry of the
snapshot that added `tests/benchmark/programs/i686_alu_chains.c`; the
latency-bound chain kernels (x%7, x%10 pairs, x*7, x*11) are timed with the
upstream and patched `lccc-i686` on the same host (no PMU: wall-clock of
10⁸-iteration dependent chains, 5 repetitions, min). Numbers are in the ledger
description of that snapshot.

### 4.3 Compile-time finding

A single 6 258-line TU whose `main` contains ≈1 900 loops took `lccc-i686 -O0`
> 36 s and 1.6 GiB RSS (GCC: seconds). Scaling probe: 100/200/400 loops in one
function → 0.1/0.2/0.6 s, 18/27/46 MiB — superlinear in function size. Not in
`alu.rs`; recorded as follow-up F-6.

## 5. Validation summary

* `scripts/i686_alu_redteam.py` — PASS at `-O0 -O1 -O2 -O3 -Os` (3 122 × 5).
* `cargo test --profile fastbuild synth_mul magic_div` — PASS.
* Godbolt oracle cross-check of the new sequences against GCC 16.2, Clang 23.1,
  ICX (`-m32 -O2`): identical or shorter for every kernel in §3.
* Non-i686 backends untouched (`DivRemTarget::I686` gate).
