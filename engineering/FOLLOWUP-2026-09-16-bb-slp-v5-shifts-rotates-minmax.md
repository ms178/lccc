# FOLLOWUP 2026-09-16 — BB-SLP v5: packed shifts, rotate decomposition, FP min/max

Session 49 (continuation). Base: `820f45c9` (merge PR #540). All work
rebased on that commit; `git diff 820f45c9..HEAD` is the deliverable.

Environment: 2-core sandbox, 4.1 GB RAM, no swap possible (no root),
gcc 14.2 as the local oracle (the Godbolt oracle needs network API
access this sandbox does not have; clang/icx are not installed). All
builds: `fastbuild` profile, `-j1`, foreground. The three i686 suite
failures and five `ci_local --fast` gate failures are ENVIRONMENTAL
(missing 32-bit libc headers — `bits/types.h` et al.; the `-m32`
preprocessor stage dies before any compiler logic runs). They fail
identically without this session's changes.

## 1. What landed

### 1.1 Packed lane-shift families (16 new intrinsics)

`VecShl/VecLShr/VecAShr` × {I16x8, I16x16, I32x4, I32x8, I64x2, I64x4}
(LShr only for I64). No packed BYTE shift and no packed qword AShr exist
before AVX-512 — those (type, op) pairs have no intrinsic and the pack
builder rejects the lanes.

- IR (`src/ir/intrinsics.rs`): enum variants with the amount contract
  (`args = [vec, Const(amount)]`, amount ∈ [1, lane_bits−1]); classified
  in `vector_result_width` (both width groups), `produces_vector_value`
  (⇒ pure), `from_name`, and the width-grammar unit test.
- Backend (`src/backend/x86/codegen/intrinsics.rs`):
  `emit_sse_shift_imm_128` (VEX.128 three-operand immediate form under
  AVX2 — one instruction for every homed shape; SSE2-only stages through
  the destination home or `%xmm0`) and `emit_avx_shift_imm_256`
  (memfold-first, all-homed, held-in-ymm0, staged — the
  `emit_avx_binary_256` discipline for unary+immediate). Both assert the
  per-family amount bound (15/31/63) at the dispatch site where the lane
  width is known.
- Regalloc (`src/backend/regalloc.rs`): `collect_vecreg_candidates`
  (arg count 1), the reduction web (classes 5/6/7/8/10 as producers and
  legal consumers), and the map-intermediate web (classes 3/6).
- VLFOLD (`src/backend/stack_layout/copy_coalescing.rs`):
  `memfold_consumer_unary_imm_256` + the unary+immediate branch in
  `compute_vector_memfold_values` + the homed-ok admission — the VEX
  immediate forms read r/m256, so `vpslld $imm, MEM, %dst` fuses a
  deferred load with zero staging.

### 1.2 Rotate decomposition — both spellings, one shared operand

`rotl(x, k) = (x << k) | (x >> (W−k))` holds per-lane bitwise, so it
holds on the packed value: the composite is ONE operand pack + a shl
pack + a lshr pack + an or pack. x86 has no packed rotate before
AVX-512; this is exactly the triple GCC/Clang emit for vectorized ARX
code.

Two recognizers, because the early SLP sweep runs BEFORE simplify
canonicalizes the funnel-shift idiom:

1. `RotateLeft`/`RotateRight` lanes with a uniform constant amount
   (normalized modulo W; 0 is the identity and rejects).
2. The RAW C spelling: `Or(Shl(x_i, k), LShr(x_i, W−k))` lanes in either
   operand order. The naive whole-side recursion builds TWO identical
   MemLoad packs (one per shift half) and the emitted code loads the
   vector twice; the look-through builds one shared operand pack.

The look-through's soundness hinges on the two spelling sides reading
the same value: `same_source` (identity-cast strip; same value; or two
loads of the same symbolic address with NO memory write between them —
the explicit interval check, because rule (c) only covers the pack's own
lane range and the B-side load of the LAST lane can sit past it). The
adversarial test (`v5_rot_interleaved_write`, a write to the loaded
address between the two halves) stays scalar and computes the correct
split-halves value.

Frontend reality handled: the rotate amount arrives as
`Copy(Cast(Const(7)))` before simplify folds it — `const_amount`
resolves the chain exactly (a `Copy` forwards; an integer `Cast` is
followed only when the constant fits the cast's output width, so a
truncating cast of an out-of-range constant refuses). The Or's operands
arrive wrapped in identity `Cast{ty→ty}` — stripped by
`strip_identity_casts`.

### 1.3 Not / Neg / Sub(x, 1) composites

- `Not(x) == Xor(x, −1)` and `Neg(x) == Sub(0, x)`: bit-exact in
  two's-complement lane arithmetic for every value including the wrap
  edges; the all-ones splat is ONE `pcmpeqd`/`vpcmpeqd`, the zero splat
  one `vpxor`/`pxor`.
- `Sub(x, 1) → Add(x, −1)`: bit-exact (x−1 and x+(−1) wrap identically),
  and the −1 splat takes the one-instruction self-compare path where
  the 1 splat paid the staged `mov/movd/pshufd` chain — the
  GCC/Clang `vpcmpeqd + vpaddd` idiom (ksub_w4 from the oracle
  showdown).
- Cost model refinement: an integer all-ones splat costs 1 (matching
  `try_all_ones_splat`'s one-instruction lowering), not the staged
  broadcast's 2.

### 1.4 FP strict min/max fold

`Select(cond = Cmp(op, l, r), t, f)` lanes fold to
`VecMin/MaxF32x4/x8, F64x2/x4` in the four STRICT-ORDERED spellings —
the same proof the loop vectorizer's `MapExpr::MinMax` carries: a
strict-ordered ternary returns the false arm exactly when MINPS/MAXPS
returns src2 (NaN, ±0 of either sign, equal values all agree), so the
fold is lane-exact under STRICT FP semantics with the false arm in the
src2 slot. Non-strict `<=`/`>=` forms differ on ±0 and stay scalar.
The mirrored spellings fold to the MIRRORED intrinsic
(`l < r ? r : l` is `Max(r, l)`), operand identity compared BIT-EXACTLY
via `lane_key` — never `Operand`'s derived float equality (`-0.0 ==
+0.0` would misfold a ±0 mask).

The `Cmp` defs die with the fold (`cond_lanes`: removed, external-use
checked with NO exceptions — every non-removed use, early or late or
terminator, rejects the seed; the early-only variant left the cmp's
later consumers reading a deleted def and ICE'd the backend, caught by
the battery's `v5_cmp_external_use`).

### 1.5 Sub-word shift promotion

`(uint16_t)(x << 5)` is `trunc(Shl(zext(x), 5))` in the IR; the
promotion rewrite now demotes shift trees to the lane width with the
promotion kind deciding the effective op (the IR encodes zext/sext in
the widening cast's `from_ty` — the lane type itself):

- `Shl`: low n bits depend only on the low n input bits — sound under
  both zext and sext.
- `AShr`: sext → `psraw`; zext (unsigned lanes) → the promoted value is
  non-negative, arithmetic IS logical → `psrlw`.
- `LShr`: zext → `psrlw`; sext REJECTS (the sign bits enter the
  truncated window: `trunc(sext(x) >>u k)` is no sub-word shift).
- Amount must be a uniform constant in `[1, lane_bits)`: at or above the
  lane width the hardware masks the packed count while the promoted
  shift's low bits are all zero — never equal.

### 1.6 Hardening of pre-existing hazards

- `VecRotlI32x4`/`VecShufdI32x4`: a non-constant immediate silently
  became `rotl(0)`/`pshufd $0xE4` (a lane permutation!) in release
  builds where the old `debug_assert!` never fired — now a loud panic.
  Two duplicated `if self.avx2_enabled {` nestings collapsed.
- `tests/regression/check_fixed_slp_distances_codegen.sh` hardcoded
  `./target/release/lccc` — now the shared `LCCC_BIN`/`CCC`/fastbuild
  resolution convention.

### 1.7 The battery and gates

`tests/regression/bb_slp_v5.c` (+`check_bb_slp_v5_codegen.sh`, wired
into `ci.yml` and `ci_local.sh` as `bb-slp-v5`): 27 shapes — every shift
family at both widths with sign-bit/all-ones/edge lanes, both rotate
spellings and operand orders, every rotate amount over asymmetric
patterns, Not/Neg/Sub(x,1) with INT_MIN/INT_MAX wraps, the four FP
min/max spellings with qNaN/−qNaN payloads and ±0 compared BIT-EXACTLY,
the ChaCha-shaped vector quarter-round, and the adversarial rejections
(varying amounts, non-strict compares, cmp external use, the
interleaved-write rotate). Runtime is verified on THREE configurations
bit-identically: SLP on, `CCC_NO_BB_SLP=1`, and gcc -O2 -march=x86-64-v3.

## 2. Verification

- Full regression suite: **PASS=710 FAIL=3 SKIP=16, AB-diff 0** (the 3
  are the environmental i686 header gaps).
- `ci_local.sh --fast`: **33 passed, 5 failed, 3 skipped** (all 5
  failures are the `-m32` preprocessor environment gap; cargo-test,
  bb-slp-v1/v3/v4/v5, codegen-quality, rustfmt all green).
- `cargo test --all-targets`: **2861 passed, 0 failed** (including the
  width-grammar test with the 16 new variants).
- clippy: lib + tests + bins each clean with `-D warnings`
  (`--all-targets` in one process OOMs the 4 GB box; each target
  individually passes — the same lint set).
- Local gcc oracle on the new shapes (instructions, `ret` excluded):

  | shape | gcc 14.2 | lccc | note |
  |---|---:|---:|---|
  | shl4 (I32x4 << 3) | 3 | **3** | tie — both fold the load |
  | rot4 (ROTL32 ×4) | 5 | **5** | tie — one load + shl/shr/or |
  | shl16w (I16x8 << 5) | 3 | **3** | tie |
  | ksub_w4 (x − 1) | 3 | 5 | pcmpeqd+vpaddd idiom landed; gap = 128-bit load fold |
  | not4 (~x) | 3 | 5 | same gap |
  | min4 (FP min) | 4 | 6 | same + broadcast splat |
  | qr4 (add+rot+xor) | 7 | 12 | same, ×3 |

  Against the PREVIOUS lccc: ksub 6→5, not 12 (fully scalar)→5, shl
  ~12 (scalar)→3, rot 6 (double load)→5, min ~48 (scalar cmov
  chain)→6, qr ~30 (scalar)→12. Every shape is at or near GCC parity;
  the entire remaining gap is the ONE missing infrastructure piece
  below.

## 3. Open follow-ups (priority order)

1. **128-bit VEX memory folding (the ksub/not/min/qr gap, 2 insns per
   shape).** GCC folds the vector load into the consumer:
   `vpaddd (%rdi), %xmm0, %xmm0`. Design (audited, not yet
   implemented): extend `compute_vector_memfold_values` with a
   128-bit consumer branch (the `emit_sse_binary_128` VEX-path op set,
   commutativity table: Add/Mul/And/Or/Xor true; Sub/Div/FP-Min/Max
   false), add those ops to `compute_vector_memfold_homed_ok`, teach
   `emit_sse_binary_128`'s VEX.128 path to consume
   `pending_vec_memfold` as the src1 memory operand (at most one mem
   per VEX instruction; src2 stays register-only), make
   `vec_operand_reg` reject pending-memfold values (mirroring
   `all_vec_homes_256` — the in-place path must not read a
   never-written home), and gate the 128-bit elision in
   `try_elide_vec_load` on `avx2_enabled` (legacy SSE memory operands
   require 16-byte alignment; VEX forms do not). The
   `materialize_pending_memfold` helper hardcodes `%ymm0` — a 128-bit
   materialization must read only 16 bytes (a 32-bit read of a 16-byte
   object can cross a page), so the pending tuple needs a width flag;
   that is the wide-touch-point (≈20 destructuring sites) that makes
   this a deliberate change, not a rush.
2. **Dead frame slots for unhomed vector values** (the known showdown
   follow-up; `v5_rotl_u64` still shows slot-staged intermediates:
   7 mem ops vs the ideal 2). Requires proving the emit-time bail
   paths cannot fire for analysis-admitted values.
3. **General cmp+select packs** (`VecCmp* + VecBlendv*`): integer
   Select-of-Cmp lanes and the non-foldable FP forms (non-strict,
   mixed arms). The FpMinMax fold covers the four strict forms; the
   blendv composite covers the rest. Exact per-lane (packed compares
   are IEEE-exact; blendv is a bitwise select).
4. **adler32 DO8 (`s1 += *buf++; s2 += s1;`)** — a LOOP epic
   (horizontal prefix-sum, needs in-vector phadd/slli+add trees and
   reassociation under the applicable fast-math gates). Not BB-SLP
   territory (BB-SLP is reassociation-free by design).
5. **nbody multi-store scatter + computed-invariant dot** — loop
   vectorizer epic.
6. **Cross-block SLP (phis), call-argument seeds** — the v2 list.
7. **FP Neg via XOR sign-mask** needs `VecXorF32/F64` families (the
   scalar lowering's bit-op spelling must be proven identical first).
8. **`VecSmaxI32x4` asymmetry**: the 128-bit signed max exists but not
   min (and the 256-bit family has both via SSE4.1) — complete the
   128-bit signed min/max pair when a consumer needs it.

## 4. Reproducing

```
cargo build -j1 --profile fastbuild --locked
LCCC_BIN=target/fastbuild/lccc bash tests/regression/check_bb_slp_v5_codegen.sh
LCCC_BIN=target/fastbuild/lccc bash scripts/run_regression_suite.sh
bash scripts/ci_local.sh --fast          # 33 pass + 5 environmental (-m32 headers)
LCCC_DEBUG_SLP=1 ./target/fastbuild/lccc -O2 -march=x86-64-v3 -S <file.c>
```
