# FOLLOWUP 2026-09-16 (evening) — BB-SLP v7: red-team battery, seed windows, FP-select VEX, sub-word SELECT demotion

Session 50 continuation. Base: upstream `main` = `8ca2fd48` (merge PR
#544 — the v6 wave). The v5 work (PR #541) and the v6 wave (PR #544)
are IN upstream; this session's deliverable is the v7 wave on top.

## 1. What landed

### 1.1 The v7 red-team battery (`tests/regression/bb_slp_v7.c` + gate)

Adversarial edges of the v6 feature set the v6 battery does not reach:

- cross-block rule-(b) corners: self-loop backedge phis (the mandatory
  rejection), phi incomings in dominated successors, nested domination,
  mixed in-block + cross-block uses, stored-and-live-out;
- cmp+blendv corners: i8/f64 lanes, mirrored FP relations on NaN lanes,
  unsigned vs signed relations on identical sign-bit patterns, a compare
  consumed by two selects (rejection), a > b with mirrored arms;
- FP-Neg chains: neg-of-neg (NaN payload preservation), arithmetic
  consumers, live-out;
- memfold corners: f32x4/f64x2/i16x8 streams, reversed sub (src2 fold),
  the register-only VEX shift-imm materialisation, two-stream 128-bit;
- scheduling: pressure, many-parameter prologues (ParamRef prefix), long
  live ranges.

The gate runs tri-config: SLP on, `CCC_NO_BB_SLP=1` (the same
compiler's scalar reference), and `gcc -O2 -march=x86-64-v3` — all
three outputs must be identical — plus scoped asm contracts (folded
memforms, the shift materialisation, the `.LCVEC` sign-mask, no scalar
compare degradation). Wired into `ci_local.sh` and `ci.yml` (the v6
gate was missing from `ci.yml` — added it too).

**ISA NOTE verified empirically (zero guesswork):** the VEX.128/256
immediate-shift encodings (66.0F 71/72/73 /digit ib) are REGISTER-ONLY.
GNU `as` assembles `vpslld $2,(%rax),%xmm0` to an **EVEX** encoding
(`62 ...`), and GCC 14.2 `-O3 -march=x86-64-v3` materialises the load
separately — the memory form exists solely under AVX-512, which the
x86-64-v3 baseline does not provide. The v6 removal of the v5-era shift
memfold admission was CORRECT; emitting the text would have produced
EVEX bytes (SIGILL on the AVX-512-less 14700KF) or an assembler error.

### 1.2 Mixed-run seed fallback (`collect_seed_candidates`)

A mixed-op store run (three uniform 4-lane groups — add/sub/xor — fused
into one 12-store stream, the `v7_pressure` shape) produced exactly ONE
8-lane seed whose lanes are non-uniform; the pack build failed and the
whole run stayed scalar. Every aligned 128-bit window of the run is now
offered as its own candidate (longest-first, one plan per scan; the
packed stores leave the scalar stream before the rescan, so overlap is
safe). p2 (the 3×4-group repro): 36 scalar instructions → 3 packed ops
+ 3 stores.

### 1.3 FP 128-bit select: VEX compare + one-instruction blendv

`emit_sse_cmp_128` gained the AVX2 discipline `emit_int_cmp` got in v6:
all-homed `vcmpps $imm, %src2, %src1, %dst` with zero staging (the
legacy 2-operand form copied src1 into the destination first), staged
VEX fallback through the reserved scratch pair. `emit_sse_blendv_128`
(FP domain) dispatches to the VEX lane select under SSE4.1+AVX2.
`v7_fsel_ge`: 16 → 11 instructions (`vcmpps` + `vblendvps`, the exact
GCC shape).

### 1.4 Sub-word SELECT demotion (the big one)

C promotes `q[i] = a[i] < b[i] ? x : y` on i8/i16/u8/u16 arrays to int
(compare, arms, select at I32); the store truncates back. The promotion
wall kept every such shape scalar (the v7 census: i8/i16/u8 selects and
min/max fully scalar while GCC vectorizes them all).

`build_pack` 1b (the v5 sub-word shift template applied to selects):

- lanes: truncating casts of promoted selects; the pack replaces only
  the truncs, so its result is bit-identical to `trunc(promoted
  select)` by construction — arm constants truncate exactly, for every
  value;
- the compare may already sit at the lane width (an earlier pass
  narrowed it) or at the promoted width over widening casts/consts;
  the predicate remaps by the promotion kind (`demote_cmp_pred`):
  zext+Slt → Ult (exactly C's unsigned promotion semantics — the zext
  values are non-negative), sext+Slt → Slt (sext is strictly monotone
  for the signed order), sext into an unsigned wider type and unsigned
  predicates over sext values REJECT (reordering);
  out-of-range compare constants REJECT, never truncate — the
  `a[i] < 40000`-on-int16 adversarial test caught exactly this
  miscompile during development (40000 "fit" the unsigned i16 range
  under a sign-flipped flag and truncated to -25536);
- the min/max spelling folds through `packed_int_minmax` (pminsw /
  pminub, SSE2) and the general shape through cmp+blendv, both with
  EMPTY cond_lanes — the promoted Select/Cmp chain stays in the IR for
  any wider uses and DCE retires it when the truncs were its only
  consumers (external uses of the compare are therefore legal here,
  unlike the at-width fold);
- the transitive dead-lane cleanup gained `Select` and `Cmp`: the
  surviving promotion scaffolding sits between the operand packs' load
  lanes and rule (a) read its uses as live in-block uses at or before
  the pack slot, rejecting every demoted plan before this.

Result: `s16` (int16 min, 8 lanes) emits GCC's exact 3-instruction
shape — `vmovdqu (%rsi), %xmm0; vpminsw (%rdi), %xmm1, %xmm0;
movdqu %xmm0, (%rdx)` — with the b-stream FOLDED into the min.
u8 mixed-arm selects, u16 unsigned compares (the Ult remap), and i8
blendv selects all pack.

### 1.5 Width-exact blendv granularity (latent v6 hazard, exposed + fixed)

The 128-bit VEX lane select is `vblendvps` only for DWORD-granular
masks (I32 lanes; FP compares produce per-dword or per-qword masks
whose dwords agree). Word lanes need `vpblendvb` (a word-compare mask
is byte-uniform — both bytes of every lane carry the same value — so
the per-byte select is word-atomic), and byte lanes need `vpblendvb`
(the per-byte sign select). The dispatch is now width-aware per family;
the previous I16x8/I8x16 routing through a vblendvps form was only
unreachable-before (no producer packed sub-word selects) — the demotion
made it reachable and the battery caught the miscompile
(`v6_sel_i16`: the dword blend read the HIGH word's mask into both
halves). The generic VEX path also stages the FALSE arm through
VEX.vvvv (REGISTER-ONLY slot) with the TRUE arm in the r/m slot — the
inverse placement emitted an unencodable instruction (assembler
rejection; the encoder's /is4 table has no memory-in-vvvv form).

### 1.6 Bit-identity cast stripping

`strip_bitidentity_casts`: same-width integer reinterprets
(`(int32_t)uint32_t`) preserve every bit, so they are strippable
wherever the consumer reasons about RAW BITS and takes its signedness
from the predicate — compare operands (the packed compare reads the
identical lanes), min/max arm identity. `sel_shape` accepts a
same-width integer compare type; `same_source` strips bit-identity on
both sides. The `(int32_t)a > (int32_t)b ? c : d`-on-uint32 spelling
now packs (pcmpgtd on the raw bits — exact).

## 2. Verification

- All 6 SLP gates (redteam, v3, v4, v5, v6, v7) PASS.
- Full regression suite: PASS=712 FAIL=3 SKIP=16, AB-diff 0 (the 3 are
  the pre-existing environmental i686 multilib-header failures).
- Benchmark output oracle: 204/204 PASS.
- `cargo test`: 2861 passed, 0 failed.
- SSE2-baseline (no -march) and explicit `-mavx2` parity runs of the
  v7 battery: identical, 0 fails.
- Zero compiler warnings; rustfmt and clippy (per-target, -D warnings)
  clean.
- `ci_local.sh --fast`: green (summary below in the session worklog).

## 3. Open follow-ups (priority order)

1. **adler32 DO8 auto-vectorization — the marquee feature nobody has.**
   GCC 14.2 and Clang do NOT vectorize the `s1 += b; s2 += s1`
   dependency (verified: gcc -O3 -march=x86-64-v3 emits pure scalar
   unroll). Design (derived, exactness proven mod 2^32):

   ```
   vs1 = [adler0, 0, 0, 0, 0, 0, 0, 0]   (dwords; adler0 = s1 init)
   vs2 = 0; vs3 = 0
   weights = rodata [32,31,...,1] as i8   (vpmaddubsw signed operand)
   loop (32 bytes/iter):
       vbuf = loadu 32 bytes
       vsad = vpsadbw(vbuf, 0)            # 4 qword partial sums
       vs3 += vs1                          # cross-term accumulator
       vshort = vpmaddubsw(vbuf, weights)  # pair-weighted (≤ 16320, no saturation)
       vsum2 = vpmaddwd(vshort, ones)      # quad-weighted dwords
       vs2 += vsum2
       vs1 += vsad
   epilogue:
       vs2 += vs3 << 5                     # 32 × Σ S_k, deferred
       s1 = hsum(vs1); s2 = hsum(vs2) + adler1
   ```

   Exactness: every term is +/× mod 2^32 and the regrouping is a ring
   homomorphism — the vector result equals the scalar WRAPPING result
   for every input, with NO chunking. The non-modular bounds hold:
   vpsadbw ≤ 2040/iter; vpmaddubsw pair ≤ 2·32·255 = 16320 < 32767
   (the weight cap 32 is exactly why zlib-ng's 64-byte unroll uses
   weights 64..33 — 2·64·255 = 32640, the saturation cliff). The
   per-lane accounting: hsum(vs1 at k) = S_k exactly (adler0 sits in
   lane 0; the 4 vpsadbw partials in lanes 0,2,4,6); hsum(vs2) =
   Σ_k [32·S_k + Σ_l (32−l) b_l] = the scalar Δs2. NMAX chunking is
   NOT needed for exactness-vs-scalar; it stays the SOURCE's business
   (adler32-spec correctness of the final `% BASE` is unchanged by the
   transform).

   Implementation plan: new intrinsics `VecMaddubsU8x32` (vpmaddubsw,
   non-commutative, args [u8, i8-weights]) and `VecMaddwdI16x16`
   (vpmaddwd, non-commutative) through `emit_avx_binary_256` +
   `memfold_consumer_256(Some(false))` for both (the weights fold from
   .rodata in the src2 slot — the vec_const_pool from v6's FP-neg
   work); recognition = extend the multi-reduction dependent-
   accumulator rejection (`sum2 += sum1`) in vectorize.rs to the exact
   adler shape (U32 wrapping accumulators, byte loads, no stores in the
   loop); reuse the existing reduction remainder machinery for the tail.
   Estimated 800–1200 lines with validation. Expected: ~6.8× on the
   checksum loop (7 vector ops / 32 bytes at ~3 ports vs 2 scalar ops /
   byte at 4 ALU ports), zlib/gzip/png workload class.
2. **Cross-block SLP, full**: phis as seeds (the loop-header phi packs)
   and call-argument seeds; v6's rule-(b) relax only services extracts
   in dominated successors.
3. **nbody scatter + dot** (loop epic; multi-store scatter with
   computed-invariant dot).
4. **Broadcast CSE across seeds**: the v7_pressure shape materialises
   the same k-splat once per seed (3× movl+movd+pshufd+movdqa); a
   post-pass GVN over identical `VecBroadcast(Const)` collapses them.
5. **Legacy-SSE/VEX mixing under AVX2**: the in-place 128-bit paths
   emit `psubd`/`movdqu` (legacy) next to `vpaddd` (VEX) — the
   AVX↔SSE transition penalty and the dirty-upper tracking cost. A
   VEX-everywhere policy under avx2_enabled would match GCC.
6. **Dead frame slots**: the demoted shapes still reserve frames they
   never touch (subq/addq with zero intervening stores) — the
   stack_layout shrink pass misses SLP-introduced slots.
7. **Duplicate b-load in the unhomed FP-select consumer** (the compare
   and the blend each load the same slot-homed operand once).
8. **Nested sub-word demotion** (`(a<b?a:b) + c` — the inner select's
   trunc feeding an arm cast of the outer) and the sub-word shifts
   feeding demoted selects.
9. **v7 gate asm contracts for the sub-word shapes** (vpminsw presence,
   vpblendvb granularity) once the shapes settle — the runtime +
   tri-config differential covers them today.

## 4. Reproducing

```
cargo build -j1 --profile fastbuild --locked
LCCC_BIN=target/fastbuild/lccc bash tests/regression/check_bb_slp_v7_codegen.sh
LCCC_BIN=target/fastbuild/lccc bash scripts/run_regression_suite.sh
bash scripts/ci_local.sh --fast
LCCC_DEBUG_SLP=1 ./target/fastbuild/lccc -O2 -march=x86-64-v3 -S <file.c>
```
