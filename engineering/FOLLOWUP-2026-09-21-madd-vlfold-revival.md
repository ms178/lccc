# Follow-up: PR #572 audit adjudication — the madd memory-fold revival

Session: 2026-09-21 (PR #572 follow-up, third of the FMA series)
Base: `d4b60ff0` (PR #572 merge) → 6 commits, all validated on this tree.

## The Review AI's audit, adjudicated

**F1 (VDEFER/VLFOLD asymmetry for the packed signed families) — AGREE on
the finding, DISAGREE on the mechanism and the fix.** The audit proposed a
one-line gate extension (`memfold_consumer_madd_256` += Signed) with the
claim that "nothing else needs changing". Deep-diving the code proved both
halves of that claim wrong:

* The map-FMA VLFOLD arm has been **entirely dead since 60f3cd17** (the v6
  safety-net tightening): `emit_intrinsic_impl`'s width-matched consumer
  set (`memfold_consumer_256/128`) never admitted the madd family, so every
  elided load was materialised as "unexpected consumer" *before*
  `emit_avx_map_fma`'s VLFOLD arm could run — for the PLAIN family too.
  Verified empirically: `[VLFOLD-EMIT] elide load %60` followed by
  `materialising %60 (unexpected consumer)`. The same gap dead-pathed the
  128-bit BB-SLP acc-fold in `emit_vec_fma_128`.
* `compute_vector_memfold_homed_ok` excluded the madd family, so the
  COMMON shape (the RA homes every map intermediate — both stream loads
  carry register homes) never elided even with the gate fixed.
* `is_vec_ssa_producer` lacked the Signed variants (their results could
  not defer).
* The audit's own acceptance criterion ("new assertion green") is
  unachievable with the proposed fix: the revived fold would have
  materialised at the safety net before reaching the emitter.

What the audit got right, verified independently: `memfold_consumer_madd_256`
missing the Signed variants, and the emitter-side sign-correctness of the
132→213/231 re-encode (re-derived from the SDM's digit convention; see the
form-semantics note below).

**F2 (string-based family re-encode) — AGREE.** Replaced with the closed
8-entry × 2-target table, `unreachable!` on anything else, unit-tested in
both directions.

**F3 (mnemonic-table duplication) — AGREE on the dedupe** (extracted
`x86_common::fma231_mnemonic`; both backends route through it; output
byte-identical). **DISAGREE on the sweep-comment nit:** the audit read
`dce::sweep_block`'s live-slice guard (which aborts) as its spans handling
(which *clears* on length mismatch — exactly what the peel does). The
comment was accurate; it now preempts the conflation explicitly.

## The miscompile the audit's testing would have walked past

Adjudicating F1 empirically (compiling the proposed test shape) surfaced a
**P0 predating PR #572**: the BB-SLP `Fma` pack's op table matched only
`(negate, ty)`, so a width-4 F64 (or width-8 F32) pack — `r[i] = x[i]*s +
z[i]` over four doubles — lowered through the 128-bit `VecFmaF64x2`
family: half the lanes computed, the store's upper half publishing the
multiplier pack raw. `r[2], r[3]` came out as `0 0` where GCC computes
`31.5 42` (and the 8-float twin broke lanes 4..7 identically). Every
existing packed-FMA test shape was 128-bit, so no differential had ever
exercised the wide packs. Fixed with width-exact op selection routing the
256-bit packs through the affine-map madd families (`VecMaddF{64x4,32x8}`
/ their `Signed(true, false)` spellings for `acc − a·b`), whose
[input, scale, bias] contract the pack's [a, b, acc] args already satisfy
exactly. New gate section D3 pins all four widths by runtime equality
against the reference compiler plus register-width shape assertions.

## The FMA3 form semantics, CPU-verified

The revival forced a re-derivation of the 132/213/231 operand roles, and
the CPU settled a session-long ambiguity: canonical AT&T
`(src3/r/m, src2/vvvv, dst)` and

* **132**: `dst = dst × src3 + src2` — r/m is the SECOND MULTIPLICAND,
  VVVV the ADDEND.
* **213**: `dst = src2 × dst + src3` — r/m is the ADDEND.
* **231**: `dst = src2 × src3 + dst` — r/m is the second multiplier, dst
  the accumulator.

(Verified with distinct A/B/C operands through GCC-assembled inline asm:
`vfmadd132pd C_mem, B, A` computes A×C+B, not A×B+C.) Consequences:

* `emit_vec_fma_128`'s acc-fold — spelled `vfmadd132 %S2, MEM_acc, %dst`
  — had its multiplier/addend roles inverted, and had **never been
  assembled**: it was born after the v6 safety net, dead its whole life.
  It now emits the canonical 213 form (`vfmadd213 MEM_acc, %S2, %dst`),
  the one form whose r/m slot is the addend.
* The assembler's "gas-style middle-memory" arms (W=1 since the scalar-FMA
  era, W=0 added then removed during this session) accepted a spelling GAS
  itself rejects (`operand size mismatch`) with a guessed role assignment
  — exactly the mechanism that let the never-tested acc-fold silently
  encode the wrong instruction instead of failing loudly. Both arms are
  gone; an operand-order mistake must now fail assembly.

## The revival (commit d601984c)

Safety net admits the madd (width 32) and packed-FMA (width 16) families at
their exact widths; the Signed variants join `memfold_consumer_madd_256`,
`is_vec_ssa_producer`, and (with the emitter audit) the homed-ok set.
`emit_avx_map_fma_inner` now follows the audited binary-emitter discipline
(`emit_avx_binary_256_inner`): memfold-aware `home()` (the live fold's
phantom RA home is never reported), a memfold-aware fast path and
`operand_reg_source`, and a 2XX arm resolving each operand via its RA home,
its deferred-store scratch (consumed in place), or a staged load. The
result computes into the DYING operand's register when the new
`vector_dying_values` analysis proves it dead: single total use, same block
as the def, live-regs claim on that home. That last conjunct is what the
loop-invariant broadcast fails — one STATIC use site, read by every
iteration, claimed only in the preheader — and it is why a plain
use-count is unsound for loops. The IR matcher gains the madd/fma128
alias-free gates (`fma(x,x,c)` shapes keep the ordinary path: one fold
cannot serve memory and register roles at once).

### Result

Saxpy-shaped loops (`r[i] = __builtin_fma(±x[i], s, c[i])`, the SLP
contraction, f32 and f64, all four sign families) — all bit-exact vs GCC:

```asm
.LBB8:
    vmovupd (%rsi,%r9), %ymm8
    vfnmadd213pd (%rdx,%r9), %ymm3, %ymm8
    vmovupd %ymm8, (%r8,%r9)
```

5 instructions → 3 per iteration, GCC's exact body, with the FMA result
computed straight into the streamed input's dying register (the
destructive-form reuse). All six packed D2 kernels now fold a load into
the FMA memory operand (new ratchet: 6 sites). The 128-bit SLP acc-fold
computes bit-exact at both widths for the first time ever.

## Gate repairs (commit 00478f70)

Two gates failed at the upstream tip (identical at the PR #572 parent —
pre-existing staleness, not regressions): `affine_map` pinned the
uncontracted `vmulpd`+`vaddpd` spelling against C99-legal default
contraction (GCC's own default behavior) and "no %ymm anywhere" against
the runtime versioning design (verified bit-exact vs scalar semantics and
GCC for shifts 0..200); `reduction_vecreg` pinned the pre-VLFOLD `%ymm0`
stream spelling against the memory-folded in-place accumulator. Both now
pin the current design with real controls (a new `-ffp-contract=off`
compile that must retain the separate mul/add; the runtime distance
guard's exact prologue).

## Residuals

* The aliased-operand shapes (`fma(x, x, c)`, `fma(x, s, s)` and the
  128-bit twins) keep the ordinary (correct, one-load-slower) path — the
  alias-free gate. Folding one of them would need the fold to serve two
  operand roles at once.
* The SSE-forced map path does not contract `a*x + b` (no 128-bit map
  madd family); GCC contracts under FMA3-capable `-march` at 128 bits.
  A future `VecMaddF64x2`-class family would close it.
* The 213-form dst preference when the streamed operand is neither dying
  nor held stages one `vmovdqa` (4 instructions, not 3) — only reachable
  when the streamed multiplicand is multi-use, which the map bodies'
  single-use loads make rare.

## Validation

cargo 3071/3071 · vector gate 46/46 (D3 width battery + memory-fold
ratchets) · all nine vector-family codegen gates OK · corpus 723 PASS /
0 real failures / 0 AB-differential (3 i686 build failures are
missing-multilib on this host, identical at the upstream tip) ·
rustfmt + clippy clean · `ci_local --fast` (see the session log for the
final tally).
