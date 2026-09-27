# Follow-up: EVEX compare-completion + AVX512-FP16 assembler campaign (maps 5/6)

## What landed

### 1. EVEX compare-to-mask completion

* **Pseudo-op aliases**: every `vcmp*` predicate spelling × {ps,pd,ss,sd,ph,sh}
  now routes to the shared cmp-mask encoder with the predicate prepended as
  imm8. The GAS 2.47 alias table was probed exhaustively: the 12
  explicit-suffix spellings (eq_oq→0, lt_os→1, le_os→2, unord_q→3, neq_uq→4,
  nlt_us→5, nle_us→6, ord_q→7, false_oq→11, ge_os→13, gt_os→14, true_uq→15)
  are accepted for ALL six suffixes including FP16 ph/sh. GAS names predicates
  29/30 `ge_oq`/`gt_oq` — NOT the SDM's GE_OS/GT_OS (byte-probed:
  `vcmpge_oqpd` → predicate 29).
* **New imm forms**: `vcmpss`/`vcmpsd` EVEX (map1 F3/F2, W from suffix),
  `vcmpph`/`vcmpsh` (AVX512-FP16, map3 pp0/pp2), `vcmpbf16` (map3.F2;
  `{1toN}` IS legal with elem 2, `{sae}` never).
* **`encode_evex_cmp_mask` hardening** (each law byte-probed, GAS text):
  k-register destination required (a vector destination previously
  mis-encoded the vector register number in the k slot — false accept),
  unsigned imm range 0..=255 (`$-1`/`$256` → `operand type mismatch`),
  same-width register law, scalar Tuple1 memory law (N = element size,
  NOT VL/elem: `vcmpsd $0,-1024(%rdx),%xmm5,%k5` is disp8×8), scalar
  forms are xmm-only and reject `{1toN}`, packed `{sae}` is 512-bit only.

### 2. Systemic `{1toN}` count law

`check_decorators` now validates every packed broadcast against
`evex_packed_bcst_elem`: a **292-row `(map,pp,W,opcode) → element-size`
table distilled from the binutils 2.47 testsuite**. `{1toN}` requires
`count × elem == VL_bytes`; rows absent from the table have no broadcast
form. Before this, `vaddpd (%rcx){1to4}, %zmm30, %zmm31` silently encoded
as `{1to8}` — a **false accept class covering every EVEX binary/cmp
family** (12/12 probe matrix mismatched before, 12/12 after).

### 3. Packed SAE/ER width law

`vaddpd {rn-sae}, %ymm5, ...`, `vfmadd132ps {rn-sae}, %xmm5, ...`,
`vminps {sae}, %ymm5, ...` were false accepts (rounding bits encoded into
<512-bit forms). The binary/3src/unary paths now enforce packed →
512-bit-only and scalar → all-xmm (`operand size mismatch`).

### 4. Shuffle/unpack broadcast false-reject fix

`vshufpd/vshufps/vunpck*` (map1) and `vshuff32x4/f64x2/shufi*` (map3)
blanket-rejected `{1toN}` — but GAS 2.47 accepts it (the SDM's
"Full-mem only" claim is not what the oracle does; `vshufpd
$123,(%rcx){1to2},%xmm29,%xmm30` sets the broadcast bit, elem 8). The
forbid was removed; counts are validated by the central law.
`vpsllvw/vpsrlvw/vpsravw/vdbpsadbw` keep their forbid (probed rejects).

### 5. AVX512-FP16

GAS 2.47.20260726 places FP16 **packed arithmetic in EVEX map 5** and
**FMA/specials in map 6** (the 2026 binutils layout; NOT in the SDM —
every row byte-distilled from the oracle). Landed: packed ph arithmetic + vsqrtph + vcomish/vucomish;
scalar sh arithmetic + vmovsh (scalarmov gained map+elem params — the
W-derived tuple is wrong for half precision); all 24 FMA ph/sh rows
(vfmaddsub/vfmsubadd have **no scalar sh form** — probed; a wrong arm
would silently encode vfnmsub's opcode); complex FMA (scalar complex
rows take **4-byte** Tuple1 memory — a complex number is two halves);
specials (vgetexp/vrcp/vrsqrt/vscalef ph+sh); imm2 rows
vgetmantph/vreduceph/vrndscaleph; converts Same/Narrow(2:1)/Narrow4(4:1)/
Wide(1:2)/Wide4(1:4) with broadcast-disambiguated Narrow memory forms
(LL from the `{1toN}` count) and the Sae-class half→double/float
widenings. `evex_sae_class` is now keyed `(map,pp,opcode)` because ph/sh
rows share opcodes with different classes (vsqrtph is ER at 512-bit;
vgetexpph packed is Sae; vrcp/vrsqrt never).

## Measurement (isa_gap_sweep, x86-64, full corpus)

| | Baseline | +compare/broadcast | +FP16 |
|---|---:|---:|---:|
| PASS | 42547 | 48826 | **51206** |
| MISSING | 19285 | 13006 | **10617** |
| FALSEACC | 0 | 0 | **0** |

Session delta: **+8659 PASS / −8659 MISSING (45% of all gaps)** with zero
false accepts at every checkpoint. Correctness batteries: compare 230/230,
FP16 263/263, converts 576/576 byte-exact vs GAS 2.47; asmdiff 1060/1060
(x86-64) and 568/568 (i686) — the i686 numbers ride the shared-encoder
delegation (vector-only `v*` mnemonics without GP operands delegate to
the x86-64 core), so the FP16 work is already i686-complete for the
delegated rows.

## Deferred work (ranked, with the probes already recorded)

1. **x/y/z-pinned convert spellings** (~120 lines): `vcvtdq2phx/y`,
   `vcvtqq2phz`, `vcvtneps2bf16x/y/z` etc. — register forms are
   same-encoding aliases; the MEMORY forms pin the source LL (probe:
   `vcvtdq2phy (%rcx), %xmm30` = LL=01 m256). Needs a
   pinned-LL variant of `encode_evex_vcvt` + suffix validation.
2. **vfpclass family** (~90 lines incl. hidden vfpclassps/pd that the
   sweep SKIPs because the corpus lines use (%eax) addressing GAS rejects
   standalone): `(imm, src, kdst)` with vvvv=1, ModRM.reg=k. Register
   forms only; memory needs the x/y/z pins.
3. **FP16 scalar converts** (~120 lines): vcvtsd2sh/vcvtss2sh/
   vcvtsi2sh/vcvtusi2sh (3-op, GP-source rows need the i686 GP guard).
4. **Remaining map-3 imm rows**: vgetmantsh/vreducesh/vrndscalesh
   (3src+imm, Tuple1 4? probe disp8), vminmaxph/sh/bf16 (~90 lines,
   map3 pp0/pp3 0x52/0x53), vcvt2ps2phx (3-src), the BF16 converts
   (vbcstne*/vcvtne*/vdpbf16ps ~150 lines), vdpphps.
5. **SM4** (64 lines), **AVX512ER** vrcp28/vrsqrt28/vexp2 (~250), **GFNI
   EVEX** (~104), **VBMI2** vpexpandb/w/vpcompressb/w/vpshldvd/vq/
   vpshrdvd/vq (~266), **v4fmaddps/v4fnmaddps** (~53), vbroadcastf32x2
   (35), vmovntdq EVEX (29), vcvtsd2si64-family (~112).
6. **TBM** (~980 incl. uppercase): blci/blcic/blcmsk/blcs/blcfill/
   blsfill/blsic/bextr/t1mskc/tzmsk (0F38 W0 rows; i686 needs local arms
   — legacy encoding, no delegation).
7. **XOP leftovers** (~440): vpcom* (8 mnemonics), vpmadcswd/vpmadcsswd,
   llwpcb/slwpcb, vprot* uppercase spellings.
8. **vextractf32x4/f64x2/vinsertf*** (~270): the FLOAT insert/extract
   rows (integer variants exist; add map3 0x19/0x1B/0x38/0x3A float arms
   + the 2-source vinsertf64x2 check).
9. **vreduceps/vrndscaleps-style {sae} on remaining families**: done for
   the six rows; verify vfixupimm/vrange reject parity (4-op shapes).
10. **mov/lea/addr32 corpus lines** (~390): GAS testsuite quoting edges
    (`mov "x(y", %eax`), `lea symbol(%eip)`, addr32-prefixed branches —
    low value, high eccentricity; classify before touching.

---

# Audit response (2026-09-27, second round — Review AI verdict on PR #643)

Every audit claim was re-probed against the GAS 2.47.20260726 oracle
(cross-checked on the system GAS 2.44; the two agree on all disputed
behaviors). Verdicts, with the oracle as the only authority:

1. **FP16 scalar non-XMM operands — AGREE.** `vaddsh/vfmadd*sh/vfmaddc*sh/
   vmovsh` with ymm/zmm spellings were false accepts. The shared-boundary
   XMM gate the audit proposed is exactly right, and the oracle shows it
   must cover the ss/sd families too (`vaddss/vfmadd132ss/vmovss %ymm`
   are equally illegal — pre-existing false accepts in the VEX row, fixed
   by the same law at `encode_evex_binary_impl`, `encode_evex_scalarmov`,
   and the VEX-row helpers via `check_avx_scalar_xmm`).
2. **vcomish/vucomish gate — AGREE, but the dispatcher comment was wrong
   in the other direction.** GAS ACCEPTS a bare `{sae}` on the register
   form (`62 f5 7c 18 2f d1`, b'=1, L'L=00); masks, broadcasts, non-XMM
   widths, memory+`{sae}` and rounding tokens are rejected. lccc was both
   false-accepting (masks, ymm) AND false-rejecting (`{sae}`). New
   dedicated encoder `encode_evex_comis` for all six mnemonics (the map-1
   vcomiss/d family gained the same EVEX row: `{sae}`, xmm16+, EGPR).
   The audit's "instruction-specific validation gate" is implemented;
   packed unary operations are untouched.
3. **Decorator legality contradictions — RESOLVED IN FAVOR OF THE CODE.**
   The oracle accepts `{rn-sae}..{rz-sae}` on `vsqrtph` at 512-bit (Er,
   identical to vsqrtps) and bare `{sae}` on `vgetmantph/vreduceph/
   vrndscaleph` zmm register forms (Sae, identical to ps/pd). The
   dispatcher comments claiming "probed: none" were hallucinated
   documentation, not code bugs; comments corrected and the behavior is
   now pinned by `tests/asm-diff/fp16-evex.casefile`.
4. **Conversion policy — SPLIT RESOLUTION, both sides partial.** The
   oracle: `vcvtps2pd`/`vcvtph2pd`/`vcvtph2psx` take bare `{sae}` at
   512-bit only; `vcvtdq2pd`/`vcvtudq2pd`/`vcvtneps2bf16` take NOTHING;
   all other converts are Er (rounding tokens) at 512-bit only. lccc had
   the classes half-right and lacked the universal LL=10 gate; both are
   fixed, with shape-before-decorator error ordering matching GAS.
5. **Testability — AGREE.** 181-casefile-entry regression net added
   (`fp16-evex.casefile`: every fixed false-accept/false-reject plus the
   complete SAE matrix, broadcast-count matrix for maps 1/2/3/5/6,
   shuffle/unpack and widening/narrowing counts);
   `scripts/distill_bcst_elem.py` upstreamed with a `--check` mode that
   re-derives the table from the pinned binutils testsuite and verifies
   row-for-row (302/302 keys, 0 ambiguous, 0 drift); `th.s` removed.

## Measurement reconciliation (the 9-line corpus delta)

The original table omitted the ENCDIFF column. Re-counting the cached
sweeps: baseline PASS 42547 / MISSING 19285 / ENCDIFF 612; final
PASS 51206 / MISSING 10617 / ENCDIFF 621. The FP16 patch moved 2389
MISSING lines: 2380 to PASS and **9 to ENCDIFF** — all nine were
`vcomish/vucomish disp8` lines (`vcomish -256(%edx),%xmm6` etc.),
byte-divergent because lccc scaled the Tuple1 memory operand by Full VL
(N=16) instead of the element (N=2). This round found the same bug via
the disp-boundary differential, fixed it (all nine now PASS), and the
summary now reports every bucket. Post-audit figures, one pinned corpus
(122508 lines): **PASS 52471, ENCDIFF 618, MISSING 9355, FALSEACC 0.**

## Additional gaps found and fixed by the audit-round differential
(47 baseline divergences on the audit battery, then extended batteries;
all oracle-verified, all pinned in the casefile)

- `vcvttph2{dq,udq,qq,uqq,w,uw}`: the entire packed truncating-FP16
  convert family was undispatched (params + arms + evex_only routing).
- `vsqrtss/vsqrtsd/vcvtss2sd/vcvtsd2ss`: EVEX rows for `{sae}`/`{r*-sae}`
  /xmm16+/masked spellings (`vcvtsd2ss` is Er, `vcvtss2sd` is Sae-only).
- The 24 scalar ss/sd FMA EVEX rows (masked/ER/high-register spellings).
- `vminmaxps/pd/ph/bf16` (AVX10.2): undispatched; now complete with the
  unsigned-imm8 law (GAS rejects `$-1` here but accepts it on vpternlog).
- `vpternlogd $256` silently truncated to imm 0x00 — now rejected
  (-128..=255 enforced on the whole 3src-imm family).
- `vpermilpd/vpermilps` variable-index broadcasts: distilled-table rows
  were missing (count law false-rejected legal `{1toN}` forms).
- AVX512ER packed `vexp2/vrcp28/vrsqrt28 ps/pd`: zmm-only rows with bare
  `{sae}`, masks, memory and `{1toN}` — complete, plus their 6 table rows.
- `vpermq/vpermpd` imm-form broadcasts: 2 table rows were missing.
- Convert broadcast-count law: `count x src_elem` windows per
  VcvtKind (the xmm-dst Narrow/Narrow4 leniency is the pinned-row zoo,
  byte-probed across every family x width x count).

## Validation (all on this tree, GAS 2.47.20260726 oracle)

asmdiff x86-64 **1243/1243**, i686 **568/568**; audit differential
battery **441/441** (268 valid byte-exact + 173 reject-parity);
cargo-test **3612/0** (+debug-assertions run); rustfmt + clippy clean;
`distill_bcst_elem.py --check`: **302/302 table rows verified**.
