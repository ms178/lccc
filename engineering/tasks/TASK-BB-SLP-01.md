# BB-SLP: Basic-Block Superword-Level-Parallelism Vectorizer — Design

Status: implementable spec (session 48). Owner: SLP epic (struct_copy +80 insns vs
clang, glibc_memcmp 4×u64 copy chains, unrolled map loops the loop vectorizer
rejects).

## 1. Problem

LCCC's vectorization is loop-centric: matmul/map loops (Phase 2b-vec), ARX
(2b-arx), interleave, const-trip maps. Straight-line isomorphic operation
groups — the SLP domain — are only covered by the narrow
`transform_fixed_distance_slp` fold (single-block squared-distance). The
classic SLP targets are missing:

1. `d[i..i+3] = a[i..i+3] OP b[i..i+3]` inside *unrolled* loops (aliasing
   unknown / tiny trip counts the loop vectorizer rejects).
2. Struct/aggregate copies by field: `p->x..w = q->x..w` (N adjacent loads +
   N adjacent stores) — the 4×u64 memcpy/memcmp shape.
3. Store-of-splat runs (`= 0`, `= c`) on adjacent fields.
4. Horizontal-ish trees (deferred: v2).

## 2. Non-goals (v1, explicit)

- Cross-block SLP (phIs, reductions), call-argument seeds, shuffle/permutation
  packs (lane reordering for commutative operand mismatch), FP FMA contraction
  (lane-exact only — no reassociation anywhere, ever), shifts/rotates,
  narrowing/widening casts, scalar FP min/max/select/cmp packs (v2), 256-bit
  gathers (128-bit Pack only), i64 lane multiplies (no SSE2/AVX2 pmulq),
  integer division lanes (gather only).

## 3. IR surface

No vector types exist in the IR; vector values are SSA values produced by
`Vec*` intrinsics, homed by the width-aware XMM/YMM allocator
(`vector_result_width`, `collect_x86_reduction_vector_values` web whitelist).

### 3.1 New intrinsics (all x86-only; AArch64/riscv/i686 backends never see them)

256-bit I64x4 family (completes the latent family — `VecLoadI64x4` currently
has NO x86 lowering and silently does nothing; fixing that is a precondition):
- `VecStoreI64x4`  — `vmovdqu %src, mem` (register-home aware, template
  `VecStoreI32x8`)
- `VecSubI64x4`    — `vpsubq` (`emit_avx_binary_256`, non-commutative)
- `VecAndI64x4` / `VecOrI64x4` / `VecXorI64x4` — `vpand/vpor/vpxor`
- `VecBroadcastI64x4` — `movq`+`vpbroadcastq`
- `VecLoadI64x4` — implement (vmovdqu, home-direct, sets dirty_upper_ymm)

128-bit I64x2 bitwise:
- `VecAndI64x2` / `VecOrI64x2` / `VecXorI64x2` — `pand/por/pxor`
  (`emit_sse_binary_128`)

128-bit I16x8 / I8x16 (byte/half lane copies — memcmp/struct char runs):
- `VecLoadI16x8` / `VecStoreI16x8` / `VecLoadI8x16` / `VecStoreI8x16` —
  `movdqu` (same lowering as I32x4 forms)
- `VecAndI8x16` / `VecOrI8x16` / `VecXorI8x16` / `VecAndI16x8` / `VecOrI16x8`
  / `VecXorI16x8` — `pand/por/pxor`
- (Add/Sub/Mul families already exist.)

Gather (vector build) — SSE2-exact, GPR-staged (scratch xmm0/xmm1 only):
- `VecPackI64x2` — 2 scalars: `movq a,xmm0; movq b,rax; movq rax,xmm1;
  punpcklqdq xmm1,xmm0`
- `VecPackF64x2` — same register choreography (bit-identical to I64 pack)

Extract (lane → scalar for external uses):
- `VecExtractLaneI64x2` — lane0 `movq xmm,rax`; lane1 `pshufd $0x0E` first
- `VecExtractLaneF64x2` — lane0 identity `movq xmm0,dst`; lane1 `pshufd`

### 3.2 Regalloc web whitelist (register homing — without it every value
pays a 32-byte protected-slot round trip and the epic dies)

- class 7 (I64x2): + `VecAndI64x2/VecOrI64x2/VecXorI64x2` producers;
  + consumers `VecStoreI64x2`, `VecExtractLaneI64x2`
- class 8 (I64x4): + `VecLoadI64x4`, `VecSubI64x4`, `VecAnd/Or/XorI64x4`,
  `VecBroadcastI64x4` producers; + consumers `VecStoreI64x4`,
  `VecExtractLaneI64x4`(v2), horizontal ops already there
- class 4 (F64x2): + `VecPackF64x2` producer; + consumer `VecStoreF64x2`,
  `VecExtractLaneF64x2`
- class 7: + `VecPackI64x2` producer
- new class 9 (I8x16): `VecLoadI8x16`, `VecAddI8x16`, `VecSubI8x16`,
  `VecAnd/Or/XorI8x16`; consumer `VecStoreI8x16`
- new class 10 (I16x8): `VecLoadI16x8`, `VecAddI16x8`, `VecSubI16x8`,
  `VecMulI16x8`, `VecAnd/Or/XorI16x8`; consumer `VecStoreI16x8`
- class 6 (I32x4): + consumer `VecStoreI32x4` already present; `VecPackI32x4`
  producer + `VecExtractLaneI32x4` consumer

Scratch discipline: every new lowering confines staging to xmm0/xmm1
(ymm0/ymm1) and never writes a register home it does not own — the
verification fixpoint requires it.

## 4. The pass (`src/passes/slp_vectorizer.rs`)

Pipeline: Phase 2c-slp, x86-64 only, `-O2+`, not `-Os/-Oz`, iter 0, AFTER
post-vectorize unroll + its DCE (unrolled bodies are the feedstock) and
BEFORE gaddrcse. Pass name `slp` (CCC_DISABLE_PASSES), kill switch
`CCC_NO_BB_SLP`, debug `LCCC_DEBUG_SLP`. `verify::verify_after_pass` after
every function that changed. Fixpoint ≤ 8 rounds; DCE after.

### 4.1 Address model (symbolic affine)

`Addr = { base: Value, var: Option<Value>, mult: u64, off: i64 }` meaning
`base + var*mult + off` bytes, built by evaluating GEP/Load/Store pointer
chains (GEP(base, Const) folds; GEP(base, Add(var, Const)) → var; raw Value →
base; `Cast`/ Phi → opaque). Two addresses are *same-stream* iff base, var,
mult equal. Packable iff same-stream and offsets are consecutive with stride
== lane byte size. This catches: struct fields (const), unrolled `a[i+1]`
(Add(var, 1) after unroll const-substitution), and `p + k` GEP chains.

### 4.2 Seeds

Store seeds only (v1). Within a block:
1. every non-volatile, `AddressSpace::Default`, non-atomic Store whose type is
   a supported lane type and whose address evaluates to a symbolic Addr;
2. group by (base, var, mult, ty); sort by `off`; maximal consecutive runs
   (stride == size) are chains;
3. chain of length L ⇒ candidate widths W ∈ {4, 2} (128-bit) and {8, 4, 2}
   (AVX2 for ≤4-byte lanes), lanes = first W stores.

### 4.3 Pack graph (bottom-up)

`build_pack(lanes) -> Pack`:
- all lanes same Value ⇒ `Splat(v)` (broadcast leaf)
- all lanes Const ⇒ `Const` leaf; all-zero ⇒ Zero leaf; else Gather
- all lanes Loads with consecutive same-stream addrs ⇒ `MemLoad` (vector
  load; base operand = ANY lane's pointer VALUE — all lane GEPs remain
  computed until DCE, and the referenced one stays alive as an operand;
  offset arg = Const(delta from that lane))
- all lanes same-op BinOp with uniform supported (op, ty) ⇒ `Op` pack;
  recurse into lhs-lanes and rhs-lanes. If lhs pack is rejected but op is
  commutative (Add/Mul/And/Or/Xor) and the swapped build succeeds, build
  swapped (canonical operand order normalization)
- all lanes Copy{src} ⇒ recurse on src lanes
- else ⇒ `Gather` (cost leaf)
Dedup: identical lane tuples / (op, operand packs) reuse the same pack
(diamonds). Depth cap 64 packs.

Supported v1 lane ops (ty-uniform, both operands packed or leaves):
int: Add, Sub, Mul (no I64 lanes), And, Or, Xor; fp: Add, Sub, Mul, Div.
Two's-complement wraparound refinement argument documented per op. FP lanes
are lane-exact — no reassociation is performed or required.

### 4.4 Scheduling & legality (the red-team core)

Replaced-set R = every instruction that is a lane of any Op/MemLoad pack or a
seed store. For packs replacing instructions:
`sched(p) = max(lane def positions)`. Gathers/splats are new code:
`sched = min over graph consumers of sched(consumer)` (insert before the
earliest consumer; always legal — their scalar inputs are defined earlier by
SSA-within-block).

Rules (all must hold or the SEED is rejected — no partial eviction in v1):
1. **External-use rule**: every use of every replaced lane value is either
   (a) an instruction in R, or (b) positioned strictly after sched(its pack).
   Extracts are emitted right after the vector op for the surviving external
   uses (extract is pure; placing it early is fine).
2. **Load-range rule**: between min and max lane-load positions of a MemLoad
   pack there must be NO memory-writing instruction (store/memcpy/call/
   atomic/inline-asm/vaarg) other than members of R positioned after the
   loads. Conservative; blocks with interleaved writes keep scalar loads.
3. **Store-range rule**: between min and max seed-store positions there must
   be no memory access (read OR write) at all other than the packed stores
   themselves and pack members — the vector store commits all lanes at
   max-store-pos, so an interleaved reader could observe a different
   half-stored state. (Alias refinement is v2.)
4. No volatile/atomic/`__seg_fs`/`__seg_gs` anywhere in a pack. Calls are
   never packed (gather leaves only).
5. Over-read proof: consecutive stride==size lanes tile the vector access
   range exactly — the vector load/store touches the union of the scalar
   accesses, never more.

### 4.5 Cost model

`benefit = Σ_{Op,MulPacks} (W-1) + (seed lanes - 1) − Σ gather_cost − Σ extract_count`
with gather_cost: Splat 1, Zero 1, Pack2 3, Pack4 7, Const-pool 1; extract 1
each. Vectorize iff benefit ≥ 1. Pure copies: 2×u64 ⇒ +2; 4×u64 ⇒ +6.
Splat-store 2-lane ⇒ 0 (rejected — correct, it is not profitable);
4-lane zero ⇒ +2.

### 4.6 Rewrite mechanics

Apply removals + insertions in one pass over the block: for each original
index i (ascending): emit pending inserts scheduled at i (packs sorted
topologically within the same slot: gathers before consumers, vector op
before its extracts), then keep/drop instruction i. New dests from
`func.next_value_id`. VecLoad args = [Value(ptr of any lane), Const(delta)];
VecStore args = [vec, Value(ptr), Const(delta)]. After each round:
`dce::eliminate_dead_code` reclaims the orphaned GEPs/lane ops.

## 5. Testing

1. `tests/regression/bb_slp_*.c` + `check_bb_slp_codegen.sh`:
   struct copy 4×u64/2×f64/4×f32/8×i8 chains, ALU trees, splat/zero runs,
   gather cost gates, kill-switch control, AVX2 gate control, external-use
   rejection, interleaved-store rejection, volatile rejection.
2. Differential: `tests/benchmark/run_benchmarks.py --only struct_copy`
   LCCC vs GCC.
3. Full regression corpus + benchmark-output oracle + cargo test.
4. CE oracle hard data on the epic shapes (recorded in
   engineering/evidence/).

## 6. Red-team checklist (must be all-green before delivery)

- [ ] MemLoad over-read impossible (stride tiling proof) — fuzz with ASAN
- [ ] External-use rule: hand-built early-use case must stay scalar
- [ ] Store-range rule: interleaved reader must stay scalar
- [ ] Lane order: non-commutative Sub/Div operand order preserved bit-exact
- [ ] FP: no reassociation (strict -ffp-contract=off/-fast-math identical
      per-lane results; test fenv/rounding invariance not required — same
      instructions per lane)
- [ ] signed overflow wrap refinement legal (UB in C; kernel -fwrapv test:
      i64 add packs under -fwrapv — vwrap identical either way: paddq wraps)
- [ ] Alignment: movdqu/movupd everywhere — no aligned-only instruction
- [ ] i686/ARM/RISCV never see the intrinsics (pass gated x86-64)
- [ ] VecLoadI64x4 no-op fix does not change any existing lowering
- [ ] regalloc fixpoint: new ops legal consumers; no protected-slot regression
      on the existing corpus
- [ ] vzeroupper/dirty_upper_ymm set by every new 256-bit path
- [ ] peephole passes (store_forwarding, dead_code) treat new intrinsics
      conservatively (unknown intrinsics are barriers by default — verify)
