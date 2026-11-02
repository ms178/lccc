# FOLLOWUP 2026-09-17B — BB-SLP v8: struct-field streams, field-disjointness, packed FMA

Session 52. Base: upstream `main` = `8ad39bfe` (merge PR #548 — the
Adler-32 epic). The session was driven by a full re-rank of the advanced
benchmark corpus against the four Godbolt oracles (gcc16.2, clang 23.1,
icx-latest, icc — the CE API became reachable from this sandbox; the
survey lives in `engineering/evidence/godbolt/s52-rank/after.json` with
per-function asm artifacts under `s52-rank/<kernel>/`).

## 1. The worst-15 diagnosis (data-driven)

| # | kernel | gap | root cause (confirmed in asm) |
|---|--------|-----|-------------------------------|
| 1 | linux_rbtree | 435 | no store merging; `movq 80(%rsp),%rax` remat chains; regalloc spills |
| 2 | csv_field_sum | 386 | no branch-condition range fusion; phi-copy shuffling; hot-loop spills |
| 3 | zlib_ng_adler32 | 287 | epic landed (7× runtime) but spilled AND-mask constant, fat frame |
| 4 | nbody | 253 | pair loop fully scalar — struct-field streams never grouped |
| 5 | sha256_transform | 117 | message-schedule loop not SLP-packed (GCC: 2-wide vpsrld/vpxor) |
| 6 | chacha20_core | 104 | SLP'd but state round-trips a 208B frame (unhomed vectors) |
| 7 | moving_stats | 95 | sliding-window + fill loop not vectorized |
| 8 | i686_alu_chains | 90 | sp=75 regalloc on long ALU chains |
| 9 | glibc_strstr | 77 | no two-way needle idiom |
| 10 | struct_copy | 72 | no SROA of returned aggregates; no (x,y,z) FP SLP |
| 11 | strlen_bench | 69 | byte loops not vectorized/idiom-matched |
| 12 | expat_xml_scan | 65 | scan loops not vectorized |
| 13 | matmul | 63 | j-unroll ×4 vs GCC's k-unroll ×2 register-carried chain |
| 14 | loop_patterns | 53 | widening reductions 128-bit only; LCG init scalar |
| 15 | vecreg_new_ops | 53 | init arrays not const-folded to .rodata; dead VLFOLD slot store |

This session attacked #4 (the marquee) end-to-end and the cross-cutting
scalar contraction defect it exposed.

## 2. What this session implemented (all validated)

### 2.1 Struct-field stream composition (the unlock)

`eval_sym_addr` degraded any GEP base carrying an index variable to an
opaque intermediate value — `a[i].f2` (GEP(GEP(a, i·stride), field_off))
landed in a DIFFERENT stream key than `a[i].f1`, so adjacent struct
fields never grouped and every struct-array kernel (nbody bodies,
rbtree nodes, Particles) stayed scalar. Bases now COMPOSE: const
offsets fold into the root's offset, same-variable offsets compose
their scales (u64-checked, fail-closed), different variables keep the
historical opaque-anchor degradation. Composition keeps the ROOT symbol
as the base — strictly better for `RestrictBases::disjoint`.

### 2.2 The field-disjointness theorem

`field_disjoint(mult, off1, s1, off2, s2)`: two accesses over the same
base with the same byte stride but INDEPENDENT index variables never
alias when both are confined to single elements
(`(off mod m) + s ≤ m`) with disjoint element-relative windows. Full
proof in the source (the `d = i − j` progression analysis; the
in-element confinement bounds exclude `d = 0`, and `|v0 ± m|` excludes
`|d| ≥ 1`). This is what makes `bodies[i].vx` traffic independent of
`bodies[j].mass` for ALL i, j — including i == j — which no same-stream
offset comparison could establish. Wired into rules (c)/(d)/(e) and the
same-source proof.

### 2.3 Per-lane precision in rules (c)/(d)

Rule (c) now checks writes against each lane's OWN byte range over
that lane's own load-to-vector-load interval (not the whole pack window
over the whole span); rule (d) checks accesses against each lane's
bytes after that lane's OWN store (reads before a lane's store see the
same pre-store bytes; writes before it are overwritten by its own
commit either way). Both refinements are soundness-preserving
generalizations — the invariants are per-lane observationally.

### 2.4 Same-source splat (pack 1a)

Lanes that are same-address loads (symbolically equal addresses, no
intervening write that touches those bytes — checked with the full
disjointness battery) broadcast lane 0 and remove the redundant loads.
The pre-CSE frontend's per-component field re-loads
(`bodies[j].mass` × 3 around velocity stores) now feed ONE broadcast.

### 2.5 PackKind::Forward — chained seeds

A second seed whose operand tree reads a first seed's vector through
its extracts reuses the vector with ZERO new instructions (the
j-velocity seed forwards the i-velocity seed's (dx,dy) vsubpd instead
of re-gathering scalars). Extract lanes at consecutive indices of one
vector, THIS family's extract op only.

### 2.6 The packed FMA contraction (rounding parity as a correctness bug)

The scalar path contracts `acc ± a·b` (gap-fused vfmadd/vfnmadd), so
the packed mulpd+addpd/subpd alternative paid a THIRD rounding the
scalar code never takes — the tri-config differential (SLP on / off /
gcc) drifted on every nbody-class kernel (measured: …594 vs …617).
`PackKind::Fma` contracts Add/Sub(acc, Mul(x, uniform-s)) lanes into
ONE vfmadd/vfnmadd (new intrinsics VecFmaF64x2/VecFnmaF64x2/
VecFmaF32x4/VecFnmaF32x4, full regalloc/copy-coalescing/memfold
registration). The fp contract is threaded into the SLP entry
(`run_bb_slp_with_contract`) — the packed contraction honors exactly
the scalar detector's contract.

`emit_vec_fma_128` carries a complete register-alias discipline: three
dying operands (a, b, acc) may each be the register the allocator
reused for the dest; the 231 form (dst = accumulator) and the 132 form
(dst = multiplicand, acc as register/memory src1, product commutes) are
selected by the alias analysis. The naive load-acc-into-dst CLOBBERED
the coalesced mul result (caught by the differential: −0.00061 vs
0.00106 — exactly what the battery exists for). The VLFOLD fast path
folds the accumulator load as the 132 memory operand (the analysis
gained the `memfold_consumer_fma_128` class: only the args[2] position
folds).

### 2.7 Scalar gap-FMA Sub extension

`detect_gap_fma_fusions` now contracts Mul-[gap]-Sub
(vfnmadd231sd/vfmsub231sd), not just Add. The gap window tolerates the
full pure address chain (Cast/Shl/integer BinOp/GlobalAddr/GEP/Load,
bound 8) with the clash analysis extended to every gap destination —
the `p[i].v -= a*b*c` shapes with the accumulator loaded between the
multiply and the subtract previously never contracted (vmulsd+vmulsd+
vsubsd; now GCC's exact vmulsd+vfnmadd231sd). The loop-carried
accumulator exclusion policy is preserved.

### 2.8 Broadcast/extract quality (the runtime levers)

- `VecBroadcastF64x2` AVX fast paths: vmovddup reg→reg and mem→reg
  (one instruction; the SSE staging paid four, including a pure
  self-copy).
- Cross-seed SPLAT CSE: a second plan's splat of a bit-identical
  source FORWARDS the earlier plan's broadcast (the per-plan dedup
  cannot see across the fixpoint).
- `VecExtractLaneF64x2` register-sourced fast path: lane 0 IS the
  register's low double (direct store), lane 1 is one pshufd — the d²
  dependency chain of every vectorized pair loop no longer pays the
  movdqa+movapd staging and the slot round trip.

**nbody advance: 0.38s → 0.24s this session (≈0.30 fully scalar at
session start; gcc 0.209s)** on this box. The pair body per (i,j):
vsubpd (dx,dy), vmovddup mass, vmulpd, vfnmadd132pd, vmovupd store —
GCC's exact shape, bit-identical to lccc's own scalar-contracted form.

### 2.9 The red-team battery

`tests/regression/bb_slp_v8.c` + `check_bb_slp_v8_codegen.sh` + CI
wiring ("bb-slp-v8"): field-pair packing, field-disjointness edges
(self-pairs, abutting elements, spanning accesses, overlapping
stores), same-source splats (disjoint/same-stream/volatile), Forward
chains, packed-FMA parity (dual-accept references — the fused OR the
split form, both legal C; a gross miscompile fails both), orientation
flips, non-uniform splats, gap-FMA spellings, the loop-carried
discipline, denormal/NaN/±0/inf lanes, live-out phis. Tri-config +
SSE2-baseline fail-closed differential.

## 3. Verification (final)

- All 9 SLP/adler gates PASS (redteam, v3..v8, distances, adler).
- Full regression suite: PASS=714 FAIL=3 SKIP=16, AB-diff 0 (the 3 =
  the pre-existing environmental i686 multilib-header failures).
- cargo test: 2862 passed, 0 failed.
- Benchmark output oracle: 204/204 PASS.
- `ci_local.sh --fast`: 43 passed, 0 failed, 3 skipped. rustfmt green;
  clippy (lib/tests/bins, -D warnings) green; zero warnings.
- Differential: pair_step bit-identical over 3000 randomized rounds;
  tri-config parity exact (SLP-on == SLP-off bit-for-bit).

## 4. Open follow-ups (priority order)

1. **csv_field_sum (gap 386)**: the branch-CONDITION range fusion —
   `if (c >= '0' && c <= '9')` with side-effect arms keeps the CFG form
   (the value-form fold already exists in range_check.rs). Design:
   extend range_check with a short-circuit-diamond arm — B1's
   true-edge is B2, both false-edges share E, B1 contains only the cmp
   + branch; rewrite B1's branch to the fused unsigned compare
   (edge-rewrite the phis). Plus the phi-copy coalescing gap.
2. **sha256_transform (gap 117)**: the message-schedule loop unroll ×2
   + SLP pack of the w[i]/w[i+1] chains (GCC's 2-wide vpsrld/vpxor
   form). The recurrence stays within the unrolled body; overlapping
   shifted loads pack as MemLoads.
3. **Register allocation quality** (rbtree 63sp, csv 62sp, sha 63sp,
   alu 75sp, vecreg 52sp, struct_copy 68sp): rematerialization of
   cheap address materializations, spill-slot coalescing.
4. **chacha20 homing (gap 104)**: the state's 208-byte frame
   round-trip — the SLP'd vectors need homes through the state-copy
   chain; ICX's vpshufb-rotation form is the structural bar.
5. **struct_copy SROA (gap 72)**: returned-aggregate scalar
   replacement; then the (x,y,z) field pairs pack via 2.1.
6. **matmul k-unroll ×2 (gap 63)**: unroll-and-jam the k loop after
   vectorization — the register-carried two-FMA chain.
7. **loop_patterns widening at 256-bit + conditional/max reductions
   (gap 53)**; moving_stats sliding windows; LCG recurrence
   vectorization.
8. **vecreg_new_ops**: .rodata folding of fixed-trip init loops
   (GCC's .LC0 vectors); the dead VLFOLD slot store in sat_kernel.
9. **Adler DO32 nested-loop spelling** (the §4.1 follow-up from the
   epic session, unchanged).
10. **nbody remaining 13%**: the d² chain still stages through xmm0/
    slots (the scalar muls' operand discipline); the memfold adjacency
    for the SLP FMA's accumulator (the acc MemLoad is ≥3 instructions
    from the FMA — the VLFOLD analysis only folds j−1/j−2).

## 5. Reproducing

```
cargo build -j1 --profile fastbuild --locked
LCCC_BIN=target/fastbuild/lccc bash tests/regression/check_bb_slp_v8_codegen.sh
LCCC_BIN=target/fastbuild/lccc bash scripts/run_regression_suite.sh
bash scripts/ci_local.sh --fast
LCCC_DEBUG_SLP=1 ./target/fastbuild/lccc -O2 -march=x86-64-v3 -S <file.c>
python3 scripts/codegen_oracle.py --rank tests/benchmark/programs/*.c \
  --local target/fastbuild/lccc --json <out.json>   # vs gcc16.2/clang23.1/icx/icc
```
