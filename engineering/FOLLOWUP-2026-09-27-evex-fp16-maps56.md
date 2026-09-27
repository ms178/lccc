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
