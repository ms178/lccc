# FOLLOWUP — S61 (fix C): nested promoted sub-word SELECT demotion in BB-SLP

**Date**: 2026-09-21 · **Landed in `main`**: `66c8687d` (PR #578, merge `1ff53e85`)
**Scope**: `src/passes/slp_vectorizer.rs` (+485/−43), 4 test files.

---

## 1. The defect

The block-level SLP vectorizer's sub-word SELECT demotion (fold 1b) handled
**one** level of promoted clamp. A two-sided clamp

```c
uint8_t t = a[i] > 200 ? 200 : a[i];   /* or the single-expression spelling */
d[i] = t < 16 ? 16 : t;
```

promotes the *whole tree* to `int`, so the outer `Select`'s arm is itself a
promoted `Select`. `strip_arm` accepted only a widening cast or a constant,
so shape extraction rejected the arm and `demote_ok` went false **before any
pack was built**. Every nested sub-word clamp stayed scalar:

| kernel | lccc before | clang |
|---|---|---|
| `clamp_u8x16` | 161 insns (16×`cmovll`/`cmovgl`, branch-free but scalar) | 5 |

This was the single largest item on the SLP probe: gap 204 of a total 480.

## 2. Why the first three diagnoses were wrong

Recorded so they are not re-proposed:

1. **"if-conversion of the second level is the blocker"** — right symptom,
   wrong mechanism. The selects *do* form; D-A/D-B (previous commit `d1a7368`)
   already fixed that.
2. **"pipeline ordering: selects materialise in iter=1, SLP runs too early"**
   — disproven by `CCC_DUMP_EACH_PASS`: `vectorize` collapses the diamonds at
   `iter=0` and `slp` sees 8 selects in one block.
3. **"a third SLP sweep after `if_convert` will pick them up"** — implemented
   and measured: **byte-identical assembly on all 52 corpus programs**. Do not
   re-add a post-`if_convert` SLP sweep.

## 3. The fix — four parts, each proven by measurement

Diagnosis was done by instrumentation, not by reading: six `demote_ok = false`
bail points tagged, a gate trace, per-arm build results, and a per-`return
None` trace across `build_plan`'s nine bail points. That is what found part 4 —
it was invisible in the assembly and in every dump.

### (C1) `build_demoted_select_arm` — recursive arm demotion

A new arm-pack builder that demotes a promoted `Select` arm in turn. It reuses
every soundness primitive of the top-level demotion rather than restating it:
`demote_cmp_pred`, `fits_narrow`, `narrow_const`, `packed_cmp_blendv`,
`cmp_predicate_imm`, `same_source`, `packed_int_minmax`, and the same two
folds — (a) min/max when both arms are the compared source, (b) the general
cmp+blendv composite.

It accepts **both** compare spellings, which is what the two C forms produce:

| C form | inner compare |
|---|---|
| temporary (`uint16_t t = …; d[i] = t < 4 ? 4 : t;`) | **promoted** to `I32` |
| single expression (`d[i] = a[i] > hi ? hi : (a[i] < lo ? lo : a[i])`) | already **narrowed** to lane width |

The first version of the helper only handled the promoted spelling and
silently lost every single-expression kernel. Both paths are now covered and
both are value-checked in the battery.

**Fail-closed**: if the recursion cannot demote an arm, the arm pack returns
`None`, the enclosing `if let (Some, Some, Some, Some)` fails, and the whole
seed is rejected. A partial demotion is never emitted.

### (C2) `strip_arm` normalizes a nested arm

An arm can reach `strip_arm` as a bare `Select`, or as a truncation back to
lane width around one (`zext(trunc(sel))` — what 16-bit lanes produce, because
the front end materializes the intermediate `uint16_t` temporary). Both
normalize to the bare inner `Select` so the recursion sees one shape.

**Rejected alternative, kept for the record**: a general `nested_select_arm`
helper applied inside the *widening*-cast arm regressed `clamp_i8x16` from
27 vectorized instructions to 192 scalar ones, because it unwrapped arms the
existing recognizers already handled and the recursion then failed where
`build_pack` had succeeded. Reverted; the narrow per-shape arms are what
shipped.

### (C3) `Copy` transparency in `build_pack`

`build_pack` had **no** `Instruction::Copy` recognizer — the only `Copy`
handling in the whole file was inside `const_amount`. The front end
materializes a C temporary read twice as `Copy` of the load:

```
Load  V16 = a[i]                       (U16)
Cmp   V19 = V16 > 32000
Copy  V25 = V16            <-- the arm's operand
Cast  V26 = zext(V25)
```

With no recognizer, the lane shape matched nothing and the seed was lost. A
`Copy` is a pure value move — same type, same bits, no side effects — so
following it is a value-identity substitution: the vector operation reads the
copy's source, and the scalar copy is left to DCE. `strip_copies` is bounded
like `strip_bitidentity_casts` and runs before the `LaneKey` is computed, so
the dedup map and the splat test see normalized shapes.

### (C4) `Copy` joins the retire fixpoint's feeder kinds — the actual blocker

This one was invisible without instrumentation. After C1–C3 **all four arm
packs built successfully** and the plan was *still* rejected. The per-`return
None` trace named it:

```
[plan-bail 5035] lane_val=16 def_at=Some(3) sched=108 use_at=6
    lane def inst = Load  { dest: V16, ty: U16 }
    use  inst     = Copy  { dest: V25, src: V16 }
    uses(16) = [4, 6]
```

Legality rule (a) rejects a plan when a lane value has an in-block use that is
not in `removed` and sits at or before the pack's slot — the replacing extract
is placed *with* the vector op, so an earlier use would read a deleted def.
The `Copy` at position 6 was exactly such a use, and the retire fixpoint
never removed it because its feeder-kind filter was

```rust
Instruction::Cast { .. } | Instruction::BinOp { .. }
    | Instruction::Select { .. } | Instruction::Cmp { .. }
```

— `Copy` missing. The comment on that filter already described the mechanism
("rule (a) would otherwise see its reads of the load lanes as live in-block
uses at or before the pack slot"); the kind list simply did not include the
one instruction that reads the load first. Retiring a copy whose every use is
removed is sound by the identical argument already applied to the other four
kinds: no side effects, no external uses, not a kept operand.

## 4. A latent miscompile found and closed while red-teaming

`packed_int_minmax` mapped **both** signednesses onto one intrinsic:

```rust
(IrType::I32 | IrType::U32, 8)  => VecMaxI32x8    /* lowers to vpmaxsd */
(IrType::I16 | IrType::U16, 16) => VecMaxI16x16   /* lowers to vpmaxsw */
```

Both lower to **signed** instructions. For an unsigned lane with the top bit
set the signed compare reads it as negative and selects the wrong operand —
`max(0x80000000u32, 5u32)` returns 5. There is no unsigned word or dword
packed min/max in the intrinsic vocabulary (SSE4.1 `pmaxud`/`pminud` are
encodable but have no `IntrinsicOp`); only `U8` has a genuine unsigned form
(`pminub`/`pmaxub`).

**Reachability**: not reachable from C today. Integer promotion delivers an
unsigned sub-word/word compare as `Ugt`/`Ult`, the min/max spelling table
accepts only `Slt|Sle|Sgt|Sge`, so every unsigned case takes the cmp+blendv
composite, which emulates the unsigned predicate with a sign-bit flip.
Verified: `clamp_hi_u32x8` emits `vpcmpgtd` + `vblendvps`, never `vpmaxsd`.

**Action**: hardened to fail closed (unsigned 16/32-bit arms removed from the
match, signedness stated in the doc comment). A missed optimization, never a
miscompile. Probe rank unchanged at 304. Pinned by
`tests/regression/bb_slp_unsigned_minmax.c`, which asserts **both**
directions: 6 unsigned kernels vectorize with **zero** signed min/max
instructions, and 2 signed controls still **do** use it — so a future
tightening that breaks signed lanes fails too.

## 5. Results

`-O2 -march=x86-64-v3`, instruction counts from each function's own body:

| kernel | before | after | vec refs | branches | gcc -O2 |
|---|---|---|---|---|---|
| `clamp_lo_u8x16` | scalar | **20** | 16 | 0 | 78 |
| `clamp_u8x16` | 161 | **33** | 28 | 0 | 72 |
| `clamp_i8x16` | scalar | **27** | 22 | 0 | 62 |
| `clamp_u16x8` | 86 | **31** | 26 | 0 | 52 |
| `clamp_i16x8` | 96 | **22** | 17 | 0 | 42 |
| `clamp_i32x4` | 50 | **15** | 10 | 0 | 32 |
| `clamp3_i32x4` (3 levels) | scalar | **23** | 19 | 0 | — |

**lccc now beats gcc -O2 on every straight-line clamp in the family**, by
2.0×–3.9×. Negative controls hold: `half_guard` 1 branch,
`side_effect_arm` 16 branches.

- **SLP probe** (24 functions): gap vs best-of-oracles **480 → 304 (−176)**,
  all of it `clamp_u8x16` (204 → 28). **No other function moved** — verified
  by per-function diff of the two oracle JSONs.
- **Runtime** (`clamp_runtime_bench.c`, 4000 reps × 4096 lanes, min of 7,
  checksum `10929567620924779191` identical across lccc-before/after/gcc-O2/O3):

| kernel | before | after | gcc-O2 | speedup | vs gcc-O2 |
|---|---|---|---|---|---|
| `clamp_u8` | 0.0205 | **0.0004** | 0.0002 | **51.25×** | 0.50× |
| `clamp_i32` | 0.0177 | **0.0014** | 0.0009 | **12.64×** | 0.64× |
| `abs_i16` | 0.0121 | 0.0120 | 0.0007 | 1.01× | 0.06× |
| `sat_add_i16` | 0.0198 | 0.0198 | 0.0037 | 1.00× | 0.19× |

## 6. Test infrastructure

- `tests/regression/bb_slp_nested_ifconv.c` — runtime battery, now the single
  source for both the value checks and the codegen contracts. Gains six
  temporary-form kernels (`clamp_lo_u8x16`, `clamp_tmp_{u8,i8,u16,i16,i32}`)
  with boundary-dense value checks, including the values where a signed
  predicate would invert the answer (0x8000, 0xffff, 65535, −32768), plus two
  store-side negative controls (`half_store` — a half-covered store set;
  `volatile_store_arm` — a volatile write in one arm).

  *Interim design, reverted*: the clamp kernels first went into a separate
  `bb_slp_nested_ifconv_codegen.c` on the theory that a runtime battery must
  not be written to invite packing. That was wrong here — the battery's clamps
  are *supposed* to vectorize, that is contract (b) — and a `.c` file with no
  `main` fails the regression suite, which auto-discovers every
  `tests/regression/*.c` and links it. Merged back; the separate file is gone.
- `tests/regression/check_bb_slp_nested_ifconv.sh` — rewritten. **A missing
  symbol is now a hard failure.** The previous version ran `awk` over a
  function that did not exist in the file under test, got an empty body, and
  read that as "zero branches" — silently satisfying a *must-branch* contract.
  Two of its five contracts were checking names that were not in the file
  (`clamp_u16x8`…`clamp_i16x8` in the probe; `side_effect_arm` vs the real
  `side_effect_arms`). Self-tested: substituting a bogus name exits 1.
- `tests/regression/bb_slp_unsigned_minmax.c` — **new**, §4 above.
  Auto-discovered by `run_regression_suite.sh` (lccc vs gcc differential +
  `CCC_NO_SMALL_SLOTS` A/B).

## 6b. Validation

Every command below was run, not inferred.

| check | command | result |
|---|---|---|
| unit + integration tests | `cargo test --profile fastbuild --locked` | all `ok`, 2 doctests ignored |
| regression corpus | `scripts/run_regression_suite.sh` | **PASS=739 FAIL=0 SKIP=8, AB-diff failures: 0** |
| CI | `scripts/ci_local.sh --fast` | **55 passed, 0 failed, 4 skipped — ALL GATES GREEN** |
| style | `cargo fmt --check` | clean (the gate compiles with `-D warnings`) |
| doc links | `scripts/check_doc_links.py` | all markdown references resolve |
| codegen contracts | `tests/regression/check_bb_slp_nested_ifconv.sh` | PASS |
| value checks | battery at `-O1/-O2/-O3`, baseline `-march`, `CCC_DISABLE_PASSES=if_convert`, and gcc `-O2 -Wall -Wextra` | all `all pass (0 fails)`, no warnings |
| probe rank | `codegen_oracle.py --rank slp_probe.c` | 480 → **304**, `clamp_u8x16` only |

The regression suite went from `PASS=727 FAIL=4 SKIP=17` to
`PASS=739 FAIL=0 SKIP=8`. The arithmetic accounts for every test: the 4
failures became passes (3 i686 tests unblocked by installing
`libc6-dev-i386`/`gcc-multilib`, 1 was the un-linkable codegen source, §6) and
the 8 former SKIPs — the ELF32 tests, which skip when the host cannot execute
them — now run and pass. `727 + 4 + 8 = 739`.

## 6c. Follow-up D — promoted sub-word negate/complement (same session)

Item 1 of §7 below was closed the same session. Two defects, both found by
instrumenting the rejection:

1. `build_pack`'s unary recognizer required every lane to be a `UnaryOp`
   whose type **equalled** the lane type. C promotion never delivers that for
   a sub-word lane, so nothing matched. `unary_lane_src` normalizes the
   sandwich to the narrow unary — exact, because negation and complement are
   per-bit / modulo-2^bits, so negating wide then truncating agrees
   bit-for-bit with truncating then negating. `Not` is covered by the same
   change.
2. `UnaryOp` was missing from the retire fixpoint's feeder kinds — **the same
   defect class as C4**, one instruction further along. The negate kept the
   widening cast live, which kept the load's only use live before the pack
   slot, so rule (a) rejected a plan whose every pack had built.

| kernel | before | after | gcc -O2 |
|---|---|---|---|
| `abs_i16x8` | 63 | **16** | 7 |
| `neg_i16x8` | 33 | **4** | 4 |
| `not_u16x8` | scalar | **4** | 4 |
| `abs_i8x16` | scalar | **9** | 6 |
| `neg_i32x4` | 10 | 10 | 4 |
| `abs_i32x4` | 18 | 18 | 3 |
| `absclamp_i16x8` | scalar | **22** | 11 |

Probe rank **304 → 226** (`abs_i16x8` 91 → 13); session total **480 → 226**.
Pinned by `tests/regression/bb_slp_unary_demote.c` — `INT16_MIN` first, whose
negation overflows back to itself and which a widened negate reproduces only
if the truncation is preserved. Suite 740/0; `ci_local.sh --fast` 56/0 with
clippy enabled.

**Still scalar**: the *loop* form (`clamp_runtime_bench.c`'s `abs_i16`). That
goes through the loop vectorizer, not BB-SLP, so this fix does not reach it.
`abs_i16` runtime is therefore unchanged at 0.0121 s vs gcc's 0.0007 s.

**Residual inefficiency in the vectorized form — diagnosed, not fixed.**
`abs_i16x8` emits a 72-byte frame, spills the loaded vector to `48(%rsp)` and
reloads it twice, and copies the zero register redundantly, where gcc holds
all of it in registers.

It is **not** an SLP problem. The IR after `slp` is 11 clean instructions with
no spill and no stack slot — `V139` (the vector load) simply feeds three
consumers (`VecSubI16x8`, `VecCmpI16x8`, `VecBlendvI16x8`). The frame, the
spill and the copies are all introduced by the backend.

Isolating it by use count, at `-O2 -march=x86-64-v3`:

| kernel | uses of the loaded vector | lccc | frame | gcc |
|---|---|---|---|---|
| `d[i] = a[i] + 1` | 1 | 7 | none | 6 |
| `d[i] = a[i] + a[i]` | 2 | 4 | none | 4 |
| `d[i] = t < 0 ? -t : t` | **3** | **16** | **72 B, 3 slot accesses** | 7 |

So the vector register allocator spills when one vector value has three
simultaneous live uses, with 16 xmm registers available. The fix belongs in
`src/backend/regalloc.rs`, not in the vectorizer, and it is worth roughly 5
instructions on every cmp+blendv kernel in this family.

## 7. Still open (next session)

Ordered by measured gap:

1. **The loop vectorizer does not vectorize a negate/select body.**
   `clamp_runtime_bench.c`'s `abs_i16` is still 63 scalar instructions against
   gcc's vectorized+unrolled 424/128-vec, and its runtime is unchanged at
   17× behind. This is now the largest gap in the family. `vectorize.rs`
   already has `UnaryOp` machinery (lines 3641, 4335, 4489), so the gap is in
   its seed/legality logic, not in the instruction vocabulary.
2. **The vector register allocator spills at three live uses of one vector
   value** (§6c above, fully diagnosed): a 72-byte frame plus a store and two
   reloads where 16 xmm registers are free and gcc needs none. The IR is
   already optimal, so this is `src/backend/regalloc.rs` work, not vectorizer
   work. Worth ~5 instructions on every cmp+blendv kernel in this family.
3. **`VecMaxU32x8` / `VecMinU32x8`** (SSE4.1 `pmaxud`/`pminud`, already
   encodable) would recover the unsigned min/max fold closed in §4. Needs an
   `IntrinsicOp` plus entries in the regalloc tables at
   `src/backend/regalloc.rs:6206,7412,7631`.
4. `same_source` is still `Copy`-blind — C3 fixed `build_pack`, not
   `same_source`, so a copy-wrapped arm can still miss a min/max fold.
   Missed optimization only.
5. SLP probe remainder, worst first: `sha256_schedule` 38, `clamp_u8x16` 28,
   `copy_bytes` 16, `add4` 14, `abs_i16x8` 13,
   `scale4`/`narrow_i32_to_i16` 11.
6. Corpus rank (`rank-before.json`, 51 files): total gap 2743; worst
   `linux_rbtree:main` 430, `csv_field_sum:main` 382, `zlib_ng_adler32:main` 277.
   See `engineering/tasks/TASK-BB-SLP-01.md` §4.

## 7b. The gate defect that let a red PR ship

`ci_local.sh` marked clippy `slow`, so `--fast` skipped it — and the standing
instruction is "`ci_local.sh --fast` must be green before ending the turn".
The pre-push configuration therefore could not see the exact failure CI exists
to report: PR #576's Clippy job failed with exit 101 while Test Suite passed,
over a tree that had reported `ALL GATES GREEN` locally. clippy is now `fast`
and always runs, the header names precisely which three gates `--fast` skips
(four were marked `slow`, not the documented "two slowest oracles"), and
`CI_LOCAL_JOBS` selects its parallelism so a low-memory host can pass 1 rather
than OOM-kill rustc mid-gate. The lint itself was
`clippy::redundant_pattern_matching` on a dead guard in `strip_arm`.

## 8. Method note

Three consecutive diagnoses were wrong and each was *plausible*. What settled
it was refusing to reason from the assembly and instead instrumenting the
decision points: six tagged bail sites, a gate trace, per-arm results, and a
per-`return None` trace over `build_plan`. The blocking condition (C4) was a
legality rejection that leaves **no trace whatsoever** in the IR dump or the
assembly — the plan simply never appears. Any future work on a pass that
builds plans and then validates them should assume the same: instrument the
rejection, do not infer it.
