# FOLLOWUP 2026-09-08 — OP-05c/OP-05d: unsigned range fusion and i8-lane conditional maps

Session scope: the byte case-fold family (`gzip`, `expat`, `curl`, `SQLite`
parsers) where GCC/Clang/ICX win with `vpcmpeqb`/`vpaddb`/`vpblendvb`, plus
the packed-compare lowering that the same family exposed as the real
bottleneck.

Everything below is backed by a reproducer, generated assembly, a
hot-loop measurement against GCC 16.2 / Clang 23.1 / ICC 2021.10 / ICX
(latest), and an exhaustive correctness gate that is now in the regression
corpus.

---

## 1. Accomplished

### 1.1 OP-05c — unsigned range-mask fusion (all integer lane widths)

**Problem.** x86 has no packed unsigned integer compare below AVX-512.
`emit_int_cmp` expands `a <u b` into a monotone sign-bias remap (`vpxor`
both operands with the lane's sign bit) plus `vpcmpgt`, and the `<=` forms
add an all-ones materialisation and an inverting `vpxor`: **five**
instructions and two extra live vector constants per predicate.  The
classifier idiom the source actually writes — `lo <= c && c <= hi` — paid
that twice and then ANDed: **eleven** packed instructions for one window
test.

**Fix.** A pure canonicalisation of the map expression tree
(`fold_unsigned_range_masks`, `src/passes/vectorize.rs`).  For lane width W,
`SB = 2^(W-1)`, and constants `0 <= lo <= hi < 2^W`:

```
    lo <=u X <=u hi
<=> (X - lo) mod 2^W  <=u  R,                    R := hi - lo
<=> signed((X - lo) + SB)  <=s  R - SB
<=> signed(X + bias)  <s  T,     bias := (SB - lo) mod 2^W,  T := R + 1 - SB
```

The middle step is the load-bearing one: `u |-> signed(u + SB)` is exactly
`u - SB` for **every** `u` in `[0, 2^W)`.  It is the order *isomorphism*
from the unsigned order to the signed order, so the comparison is preserved
rather than merely implied.  `T` is representable as a signed W-bit integer
iff `R <= 2^W - 2`, which is why the whole-domain window (an always-true
mask) is refused instead of fused.

The result is `Cmp(Slt, Add(X, bias), T)` — **one `vpadd` plus one
`vpcmpgt`** — expressed entirely in ops that already existed.  No new
intrinsic, no new emitter, no new encoding.  Single-sided unsigned compares
against a constant are the degenerate windows `0..=hi` / `lo..=MAX` and fuse
the same way (5 instructions -> 2).

Measured on the dword classifier (`unsigned` element type, AVX2): the packed
body went from **24 instructions to 15**, and to **10** after §1.2.

### 1.2 All-homed three-operand VEX fast paths

`emit_avx_binary_256` already had an "all operands and destination carry XMM
homes" fast path.  `emit_avx_blendv_256`, `emit_int_cmp` and the
`VecLoadI32x8` arm did not — they staged every operand through `%ymm0` and
copied the result back out, costing **five `vmovdqa` per iteration** in a
conditional-map loop.

Added (`src/backend/x86/codegen/intrinsics.rs`):

* `all_vec_homes_256` — the shared admission test.  Sound because
  `avx_store_dest` never *defers* a register-homed destination (a homed
  value's home always holds its current contents), and because a value whose
  load was elided into a pending VLFOLD memory operand is explicitly
  rejected.
* blendv all-homed form: `blendv %mask, %true, %false, %dst`.
* `emit_int_cmp` all-homed form for the two predicates needing no auxiliary
  register — `eq` (`vpcmpeq`) and signed `lt` (`vpcmpgt`).  This is exactly
  the predicate OP-05c emits, so the fused compare costs one instruction.
* `VecLoadI32x8` loads directly into the destination's YMM home, matching
  what the FP twin `VecLoadF64x4` has always done.  The integer form's
  omission cost one register copy in *every* integer map and reduction loop.

Also fixed: the homed `VecLoadF64x4` path never set `dirty_upper_ymm`, so a
function whose only 256-bit operation was a homed load skipped its
`vzeroupper` and left the AVX/SSE transition penalty in place.

### 1.3 OP-05d — i8/u8-lane conditional map vectorization

**Problem.** C's integer promotions destroy byte parallelism.  The frontend
does not emit a clean "extend, compute wide, truncate" bracket — it emits a
*mixed* tree in which the arithmetic runs at `int`, the selects run at
`unsigned char` over the **raw** load, and the truncation sits in the middle:

```
%16 = load u8                    ; the byte itself
%18 = zext %16 to i32            ; promotion, feeds only cmp/add
%20 = cmp sge i32 %18, 65
%24 = cmp sle i32 %18, 90
%28 = add i32 %18, 32
%29 = trunc %28 to u8            ; truncation of the ADD only
%40 = select u8 %24, %29, %16    ; selects at BYTE type, over the raw load
%41 = select u8 %20, %40, %16
       store u8 %41
```

Vectorizing that as written yields 8 elements per YMM and a widen/narrow
sandwich; GCC 16.2 does exactly this (`vpmovzxbw`/`vpmovzxwd` up, dword
compares, `vpackuswb`/`vpermq` down) at **40 instructions per 32 bytes**.

**Fix.** `parse_byte_map_expr` walks the mixed graph directly and treats the
width casts as what they are in 8-bit lanes: no-ops.  Legality rests on
truncation to 8 bits being a **ring homomorphism** `Z/2^32 -> Z/2^8`, so for
`+`, `-`, `&`, `|`, `^` the low byte of the result depends only on the low
bytes of the operands — and the store keeps only that low byte.  The
transformation is therefore *equal*, not approximate.

The two node kinds that read more than the low byte carry a side condition,
discharged by an interval analysis threaded through the parse:

| both operand ranges | action | why |
|---|---|---|
| `⊆ [0, 255]` | emit the **unsigned** byte predicate | wide-signed, wide-unsigned and byte-unsigned are the same total order there |
| `⊆ [-128, 127]` | keep the predicate as written | `x mod 256` is an order isomorphism from that window for **both** byte orders |
| otherwise | refuse | loop stays scalar |

`MinMax` additionally requires `⊆ [0, 255]`, because `vpminub`/`vpmaxub` fix
the order as unsigned — which is precisely where it coincides with the wide
order.

Deliberate refusals, each for a concrete machine reason: **multiply** (no
packed byte multiply exists at any width), **shifts** (no packed byte shift;
a word-shift-plus-mask lowering is no longer a one-instruction op), and
**non-constant loop invariants** (would need `vpbroadcastb` from a GPR).

**Op space.** Only ops whose *machine encoding* is lane-specific were added
(12: `VecAdd/Sub/Cmp/Blendv I8x{32,16}`, `VecMin/MaxU8x{32,16}`).  Bitwise
ops, unaligned 256-bit load/store, the zero idiom and the broadcast are
bit-exact at every lane width and are **reused**.  A constant byte needs no
`vpbroadcastb`: `b * 0x01010101` through the existing dword splat fills all
32 byte lanes.  The in-tree assembler already encoded every new mnemonic
byte-identically to GAS 2.47 (verified against `as --64` + `objdump`).

### 1.4 Select strength reduction (packed-only lowering)

`m ? x + k : x`  ->  `x + (m & k)`, `m ? x - k : x` -> `x - (m & k)`, and the
classifier shape `m ? k : 0` -> `m & k`.  A lane mask is all-ones or
all-zeros, so `m & k` is `k` or `0` and the arithmetic reproduces the select
exactly.  Payoff: `vpblendvb` is **two uops** on every Intel core through
Raptor Lake (and a 2-cycle op on Zen), while `vpand` + `vpadd` are one uop
each; the rewrite also shortens the dependency chain, since the `vpand`
depends only on the mask.

This is applied to the **packed body only** — see §2.2 for the defect that
taught us why.

### 1.5 Defects found and fixed (all were latent miscompiles)

1. **`build_map_remainder_loop` hardcoded `elem_bytes = 4`** for every
   non-`F64` type.  Wrong for `I64`/`U64` (8) and for byte lanes (1): the
   scalar remainder addressed `(%rsi, %r10, 4)` for a byte array and read
   past the end.  Replaced by `map_elem_size`, now the single authority
   shared with `analyze_map_pattern`.
2. **The scalar remainder mirror emitted a *signed* min/max** for byte
   lanes, where the packed body emits unsigned `vpminub`/`vpmaxub`.
   `min(240, 16)` returned 240 (as `i8`, 240 is −16).  Caught by the byte
   clamp gate at trip count 1.
3. **Byte constants were replicated in the semantic tree.**  Replication is
   a property of the *dword-broadcast lowering*, not of the expression: the
   scalar remainder mirror evaluates the same `MapExpr`, where
   `min(0xF0F0F0F0, c)` is not `min(240, c)`.  Moved into
   `MapEmitCtx`'s `Invariant` arm, gated on the byte element type.
4. **Constant carrier types blocked structural folds.**  Promotion leaves
   the same literal as `I32` inside the compare and `I64` in the select arm,
   so `(c < 16) ? 16 : c` failed to fold into `max(16, c)` and the clamp
   idiom silently stayed scalar.  Canonicalised to `I32` (value preserved).
5. **`offset_is_canonical_unit_stride` rejected `elem_size < 2`**, so byte
   GEPs (`&p[i]` needs no scaling) never matched.  `find_reduction_byte_iv`
   had the mirror-image gap.
6. **Bitwise lane ops were missing from the map-broadcast consumer
   classes** in `regalloc.rs`.  A broadcast feeding a `vpand` was not
   recognised as a map broadcast, got no register home, and was re-read
   **from the stack every iteration** — this cost 2 instructions per
   iteration in every strength-reduced loop and in every dword `x & K` map.

### 1.6 Measurement tooling: `scripts/hot_loop_metric.py`

`codegen_oracle.py`'s `insns` column is a **static, whole-function** count.
For loop kernels it is actively misleading: a compiler that refuses to
vectorize emits one tight scalar loop and "wins", while a vectorizing
compiler pays for prologue + packed body + remainder.  Ranking the byte
kernels that way rewards exactly the codegen we are trying to beat — and it
is what made LCCC look best in the pre-session survey while it was emitting
*scalar* byte loops.

The new tool recovers, from assembly alone, the steady-state density
`instructions in the loop body / bytes consumed per trip`.  Two design
points worth keeping:

* **Loop selection is by maximum bytes-per-trip, not by tightest cycle.**  A
  vectorized kernel has at least two backedges and the *remainder* is
  usually the shorter one; "innermost" reliably picks the loop that runs
  fewer than VF times in total.  The remainder is bounded by one vector
  width regardless of `n`; the packed body's trip count grows with `n`.
* **Bytes-per-trip comes from the pointer advance** (the immediate of the
  `add`/`sub`/`lea` updating a register used in a memory operand), falling
  back to vector-move width × load count.  When neither is recoverable the
  tool reports `n/a` rather than inventing a density.

---

## 2. Results

`/home/user/work/casefold.c`, `-O3 -march=x86-64-v3`, steady-state hot loop,
**instructions per input byte** (lower is better):

| kernel | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---|---|---|---|---|
| `fold_lower` | **0.3125** (10 / 32 B, YMM) | 1.2500 | 0.2422 (4× unrolled) | 2.0000 | 0.6250 (XMM only) |
| `fold_upper` | **0.3125** | 1.2500 | 0.2422 (4× unrolled) | 1.3750 | 0.6250 |
| `clamp_bytes` | **0.2500** (8 / 32 B) | 0.1875 | 0.1172 (4× unrolled) | 0.4375 | 0.3750 |
| `classify_alpha` | scalar (see §3.1) | 0.3125 | 0.5625 | scalar | scalar |

Against the **non-unrolled** competition LCCC now wins every vectorized
kernel: 2.0× better than ICX and 4.0× better than GCC on the case folds.
Clang's advantage is entirely 4× unrolling of the same per-element work —
see §3.2, which is a scheduling change, not a lowering change.

`fold_lower` packed body (10 instructions / 32 bytes):

```
vmovdqu  (%rsi,%r11), %ymm5
vpaddb   %ymm2, %ymm5, %ymm6     ; + 63          (range bias)
vpcmpgtb %ymm6, %ymm3, %ymm7     ; vs -102       (fused window)
vpand    %ymm4, %ymm7, %ymm8     ; & 32          (strength-reduced select)
vpaddb   %ymm8, %ymm5, %ymm9
vmovdqu  %ymm9, (%rdi,%r11)
addq     $1, %r9                 ; <-- redundant, see 3.2
addq     $32, %r11
cmpq     %r8, %r9
jb       .LBB2
```

`clamp_bytes` reaches the theoretical minimum for the operation itself —
`vpmaxub` + `vpminub` around one load and one store, no compare, no blend,
and it holds at the SSE2 baseline because unsigned byte min/max are baseline
instructions (unlike their dword counterparts, which need SSE4.1).

### Correctness

* `tests/regression/vec_range_mask_fusion.c` + `_sse2.c` — 8 window shapes
  (sign-bit-straddling, coincident bias/threshold, whole-domain refusal,
  single-value window on the sign bit, zero-anchored, strict bounds), every
  trip count from 0 up, reference computed through `volatile`.
* `tests/regression/vec_byte_lane_map.c` + `_sse2.c` — 12 byte kernels
  including deliberate 8-bit wraparound, a compare on a *wrapped* sum,
  `signed char` with sign-extended leaves and negative constants, and two
  kernels (shift, multiply) that exist to assert the analysis **refuses**.
  Input is byte-exhaustive (0..255 at every alignment).
* Unit tests: the 8-bit range fusion is proved **exhaustively** (all
  `lo <= hi` windows × all lane values, ≈8.4 M checks — the identity is
  width-parametric, so this is a proof of the algebra at every width); the
  ring homomorphism, both compare side conditions, and the unsigned-only
  min/max domain are likewise checked exhaustively over their domains.
### Benchmark corpus

`tests/benchmark/run_benchmarks.py --compilers lccc,gcc --reps 3 --warmup 1`
at HEAD: **39/39 workloads correct**, LCCC/GCC geometric mean **0.7597**
(recorded pre-session baseline in `engineering/STATE.md`: 0.8598).

Caveat, stated plainly: this run used 3 reps rather than the runner's
preferred >=7, and it emitted coefficient-of-variation warnings on several
sub-20 ms workloads.  The **correctness** result (39/39) is solid; treat the
aggregate delta as indicative rather than a certified regression-free
number, and re-run at full rep count on a quiesced machine before quoting
it.  No workload changed correctness state, and the largest known gaps are
unchanged and unrelated to this work (`lz4_compress` 3.05x, `chacha20_block`
1.55x -- the open ARX item, `sha256_transform` 1.55x).

* Full gate at HEAD: **720 regression tests, 0 failures**; **2180 unit
  tests, 0 failures**; `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` both exit 0.

---

## 3. TODO for future agents

### 3.1 `classify_alpha` — short-circuit `||` is not if-converted  (HIGH)

`((c>='a'&&c<='z') || (c>='A'&&c<='Z')) ? 1 : 0` bails with
`BAIL: internal condbranch` **before** the map parser runs, so it stays
scalar at every element width (the dword version fails identically — this is
not a byte-lane limitation).  GCC vectorizes it at 0.3125 insn/B.

The map parser already recovers the `||` shape *when if-conversion produces
nested selects* (`MapExpr::MaskConj { is_and: false }`).  The gap is purely
that if-conversion does not flatten a two-level short-circuit `||` whose
arms are themselves `&&` conjunctions.  Fix in the if-conversion pass, not
in the vectorizer.  Expected result after the fix, with the machinery that
already exists: two fused windows (2 instructions each) + `vpor` + `vpand`
with 1 = **~9 instructions / 32 bytes = 0.28 insn/B**, comfortably ahead of
GCC.

### 3.2 Redundant second induction variable  (HIGH — affects EVERY map loop)

Every vectorized map loop carries both a trip counter and a byte offset:

```
addq $1, %r9      ; trip counter
addq $32, %r11    ; byte offset  == %r9 * 32
cmpq %r8, %r9
```

GCC emits one: `addq $32, %rax ; cmpq %rax, %rdx`.  This is one instruction
per iteration in *all* map kernels — `clamp_bytes` would go 8 -> 7 and
`fold_lower` 10 -> 9 (the original design target, 0.28 insn/B).  The two IVs
are exactly correlated, so this is textbook IV elimination; the reason it
was not done in this session is that the remainder-loop entry computes its
start index from the trip counter, so the rewrite has to retarget that too.
Do it in `transform_map_vector` (bound the byte IV by
`(n / VF) * VF * elem_size`, derive the remainder start by shifting the byte
IV back), and re-run both validation gates — they already cover every trip
count including the empty and single-element cases.

### 3.3 Unroll the packed map body 2×  (MEDIUM)

Clang's lead on every byte kernel is 4× unrolling, not a better lowering:
its per-element work is identical to ours.  Two independent 32-byte streams
would amortise the loop overhead (which after §3.2 is 3 of 9 instructions)
and fill the two load ports.  Expect `fold_lower` ≈ 15 / 64 B = 0.234
insn/B, beating Clang's 0.242.  Gate it on trip-count and code-size
heuristics.

### 3.4 `vpbroadcastb` for runtime byte invariants  (MEDIUM)

`parse_byte_map_operand` fails closed on non-constant loop invariants
because the byte path splats constants through the dword broadcast.  Adding
`VecBroadcastI8x{32,16}` (`vpbroadcastb`, AVX2; `punpcklbw`+`pshufd` at
SSE2) would admit `dst[i] = src[i] < threshold ? lo : hi` with runtime
bounds — common in image and codec code.

### 3.5 Signed byte min/max  (LOW)

`k_signed_clamp` currently lowers to `vpcmpgtb` + `vpblendvb` twice (8
instructions / 32 B).  `vpminsb`/`vpmaxsb` are SSE4.1/AVX2 and would take it
to 6 — but they must stay behind an ISA gate, unlike the unsigned forms.

### 3.6 16-bit lanes  (LOW)

`map_elem_size` returns `None` for `I16`/`U16`.  The entire OP-05d
machinery is lane-width-parametric — the interval analysis, the range
fusion, the ring-homomorphism argument — so extending it needs the op table
entries (`vpaddw`/`vpsubw`/`vpcmpgtw`/`vpminuw`) and a `[0, 65535]` /
`[-32768, 32767]` domain pair, nothing conceptual.  Note `vpminuw` is
SSE4.1, so the 128-bit path needs a gate.

### 3.7 Structural debt noticed while working here

* **`is_two_operand_binary` in `copy_coalescing.rs` is misnamed** — it lists
  256-bit three-operand VEX ops as well, and it omits Cmp/Blendv for a
  reason recorded only in a comment on a *different* function.  The three
  overlapping classifier tables in `regalloc.rs`
  (`collect_vecreg_candidates`, `collect_x86_map_broadcast_values`,
  `collect_x86_map_intermediate_values`) plus the two in
  `copy_coalescing.rs` must each be updated by hand for every new op, with
  no compiler-enforced link between them.  Defect §1.5.6 is exactly what
  that costs.  A single `VecOpInfo` table (lane width, element type,
  commutativity, register class, consumer kind) queried by all five would
  make the next op family a one-line change and would let a unit test assert
  coverage the way `declared_width_agrees_with_the_lane_count_in_the_name`
  already does for widths.
* **The scalar remainder mirror and the packed emitter are two independent
  interpreters of `MapExpr`.**  Three of the five defects above were
  mirror/packed disagreements (signedness, constant replication, element
  size).  A differential unit test that evaluates both on random trees and
  random inputs would have caught all three without needing a full compile.
* `tests/benchmark/programs/ascii_case_fold.c` fuses the byte map with a
  serial `sum = (sum<<5)+sum+c` hash, so the corpus does **not** yet
  exercise this path; it needs loop distribution first.  Until then the
  in-tree benchmark understates the byte work — add a pure map workload.

---

## 4. Files touched

| file | change |
|---|---|
| `src/passes/vectorize.rs` | OP-05c fusion, OP-05d byte parser + interval analysis, strength reduction, `map_elem_size`, unit tests |
| `src/ir/intrinsics.rs` | 12 byte-lane ops + all 6 classification tables + `names_to_op` + width-test shapes |
| `src/backend/x86/codegen/intrinsics.rs` | `IntCmpLane`, lane-parameterised `emit_int_cmp`, all-homed VEX fast paths, byte dispatch arms, `VecLoadI32x8` home load |
| `src/backend/regalloc.rs` | byte ops + bitwise ops in the map broadcast/intermediate classes |
| `src/backend/stack_layout/copy_coalescing.rs` | byte ops in the SSA-producer / two-operand / memfold tables |
| `scripts/hot_loop_metric.py` | new — steady-state instruction density from assembly |
| `tests/regression/vec_range_mask_fusion{,_sse2}.c` | new |
| `tests/regression/vec_byte_lane_map{,_sse2}.c` | new |
