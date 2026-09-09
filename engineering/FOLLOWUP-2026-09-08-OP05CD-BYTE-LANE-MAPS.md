# FOLLOWUP 2026-09-08 — OP-05c..f: range fusion, i8-lane maps, if-combine, class union, runtime byte splats

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

### 1.4b OP-05e — short-circuit if-combine (loop-body scoped)

**Problem.** C's `&&`/`||` lower to a branch chain, and if-conversion only
recognises diamonds and triangles, so a chain was converted at most one level
deep.  `((c>='a'&&c<='z') || (c>='A'&&c<='Z')) ? 1 : 0` came out HALF
converted -- the second conjunction became two `Select`s while the first
stayed a branch chain -- and the leftover `CondBranch` made the map
vectorizer bail with `internal condbranch` at **every** element width.  The
loop stayed scalar while GCC vectorized it.

**Fix.** `detect_if_combine` / `apply_if_combine` in `passes/if_convert.rs`
fold a two-level chain into one branch on a bitwise `and`/`or` (GCC's
`tree-ssa-ifcombine`):

```text
  AND                              OR
  pred: br %a, inner, F            pred: br %a, T, inner
  inner: <insts> br %b, X, F       inner: <insts> br %b, T, Y
  =>                               =>
  pred: <insts> %c = and %a, %b    pred: <insts> %c = or %a, %b
        br %c, X, F                      br %c, T, Y
```

Six fail-closed clauses, each documented at the function: single
predecessor, side-effect-free and non-trapping `inner`, **both conditions
provably 0/1** (this is what makes the bitwise op the logical connective --
`3 && 4` is true but `3 & 4` is 0), no phis in `inner`, the shared target's
phis must agree on both edges, and the retargeted block's phi edges are
relabelled.

**Scoped to natural-loop bodies, deliberately.**  If-combine converts a
control dependence into a data dependence.  That pays where the straight-line
result can be vectorized or the branch is unpredictable -- inside loops.
Outside them it is a net loss, because it destroys the path-sensitivity that
correlated-value propagation needs: on the false edge of `var <= 0` a signed
`var` is known positive, which proves `(unsigned)(var-1) < UINT_MAX` and
deletes an unreachable `link_failure()`.  Merging the conditions erases the
edge carrying that fact and the call survives to link time.  **This is not
hypothetical -- the unrestricted version failed
`tests/regression/path_range_var_minus_one_uintmax.c`** (reduced from
`gcc.c-torture/execute/20041114-1.c`) on the first gate run.  GCC resolves
the same conflict by ordering (`ifcombine` after VRP); loop-body scoping is
the equivalent guarantee here and is robust to how often the pass pipeline
re-enters `if_convert`.

Because if-combine emits the conjunction as a bitwise `BinOp`, both map
parsers now canonicalise `and`/`or` of two MASKS into `MapExpr::MaskConj`.
The two forms are interchangeable in both consumers (the packed emitter
lowers `MaskConj` to the same `vpand`/`vpor`; the scalar mirror lowers it to
the same bitwise op on two 0/1 setcc values), and the canonical form is the
one the window fusion can see.

### 1.4c OP-05e — single-bit window union (the `isalpha` idiom)

**Identity.**  Let `B` be a power of two, `W1 = [lo, hi]` a window in which
bit `B` is clear at every point, and `W2 = [lo|B, hi|B]`.  Then

```text
    x in W1 U W2   <=>   (x | B) in W2
```

*Proof.* (=>) If `x in W1`, bit `B` is clear in `x`, so `x|B = x+B in W2`.
If `x in W2`, bit `B` is already set, so `x|B = x in W2`.
(<=) If `x|B in W2 = [lo+B, hi+B]`, then `y = (x|B) - B in [lo,hi] = W1`, and
`x` is either `y` (bit clear, so `x in W1`) or `y|B` (bit set, so
`x in W2`).  QED.

For ASCII this is `c in ['A','Z'] or c in ['a','z']  ==  (c|32) in ['a','z']`
-- the classifier that pervades every parser (`expat`, `curl`, SQLite,
tokenizers).  The precondition (`lo & B == 0` and `lo | (B-1) >= hi`) is
checked, not assumed: `[30,34] U [62,66]` contains points whose bit 5 is
already set and must be refused.

Combined with the fusion of §1.1 the whole `isalpha` classifier becomes
`vpor` + `vpaddb` + `vpcmpgtb` -- **three** packed instructions where the
unfused two-window form needs five and the pre-session code needed a scalar
loop.

### 1.4d The CI failure, and what it says about the testing strategy

PR #452 went red on **"Differential correctness gate (lccc vs GCC oracle)"**
with two miscompiles: `vectorize_int_conditional_map` and
`vectorize_abs_fold_subtrahend`.

**Root cause, mine.**  The all-homed VEX compare fast path of §1.2 shipped
guarded by

```rust
if !unsigned && (eq || !invert) {          // WRONG
```

`emit_int_cmp`'s predicate encoding is `imm 0 = eq`, `1 = lt.s`,
`2 = le.s (invert)`, `4 = ne (eq AND invert)`.  So `ne` satisfies `eq`, fell
into the `eq` arm, and emitted a bare `vpcmpeqd` with **the inversion
dropped**: `v != 0 ? -v : v` silently computed `v == 0 ? -v : v`.  The
function's own comment said `le`/`ne` "keep the staging path" -- the code did
not implement its comment, and nothing checked.

The fix does not merely exclude the inverted predicates, it *handles* them:
all four signed forms now take the register-to-register path, with the
inversion built from the canonical all-ones idiom in the reserved `%ymm1`
scratch.  Three instructions where the staging path needed five, and no
`%ymm0` traffic.

**Why 722 regression tests missed it.**  Not one map kernel in the corpus
used a `!=` mask at AVX2.  The gap was not depth, it was *coverage of the
emitter's own vocabulary*: `emit_int_cmp` has six predicates x two lane
widths x {homed, staged} = 24 paths, and the corpus happened to hit about
half.

Two things changed as a result, and both are in the patch:

* **`tests/correctness/vectorize_cmp_predicate_matrix`** (+ its SSE2 twin,
  sharing one source so the two ISA baselines can never drift): every
  predicate, in both operand orders, against a constant AND against a second
  stream (which is what forces the all-homed path), at 8-bit and 32-bit
  lanes, over the signed extremes and the unsigned wrap boundary, with a
  0..40 trip-count sweep per emitter path.  It is **mutation-verified**:
  reintroducing the `(eq || !invert)` guard makes it fail, removing it makes
  it pass.
* **`scripts/ci_local.sh`** runs all fifteen gates of the GitHub `test` and
  `clippy` jobs, in order, with the same environment.  "The tests I
  remembered to run" is what let a red CI happen; the corpus is one gate of
  nine, and the differential oracle, the benchmark-output oracle and the
  emitted-assembly contracts each catch things the others cannot.

### 1.4e OP-05f — runtime byte invariants and signed byte min/max

* `VecBroadcastI8x{32,16}` (`vpbroadcastb`; SSE2 lowering
  `movd`+`punpcklbw`+`punpcklwd`+`pshufd`).  Constants still splat through
  the dword broadcast of `b * 0x01010101` -- one instruction fewer and it
  shares the constant-pool cache -- so `MapEmitCtx` picks per operand.
* The runtime invariant's **range is recovered through its integer
  promotion** (`invariant_byte_promotion`): a `zext u8 -> i32` parameter gets
  `[0,255]`, which is what lets it be a compare operand.  Without it,
  `c < threshold` with a runtime `unsigned char` threshold has unknown range,
  fails the compare side condition, and the loop stays scalar -- the exact
  shape image and codec code writes.  The narrow value is also the splat
  source, so the broadcast reads the byte directly.
* `VecMin/MaxI8x32` (`vpminsb`/`vpmaxsb`).  AVX2-only by construction: they
  are SSE4.1, so the 128-bit path keeps compare+blend rather than bailing the
  loop to scalar.

### 1.4f `fold_int_minmax` was not confluent (pre-existing, affects dwords too)

Enabling the min/max fold for signed bytes exposed a defect that had been
costing the **dword** clamp two instructions and a spill all along.

`fold_int_minmax` folded a `Select`'s ARMS but not its CONDITION's operands.
A nested clamp -- `if (v<lo) v=lo; if (v>hi) v=hi;` -- is
`Select(Cmp(P, hi, INNER), hi, INNER)`; folding only the arms rewrote the
`f` copy of `INNER` into a `MinMax` while the `cond` copy stayed a `Select`.
The outer pattern test then compared two different shapes and missed, and --
worse -- the emit cache keys on tree shape, so the body computed the inner
clamp **twice**, once as `vpmaxsb` and once as `vpcmpgt`+`vpblendv`, with the
extra live value spilling.

Folding the condition first makes the rewrite confluent.  Measured:

| clamp | before | after |
|---|---|---|
| `int` (I32, AVX2) | 9 insns / 32 B | **7** (`vpmaxsd (mem), %ymm3, %ymm5` ; `vpminsd`) |
| `signed char` (I8, AVX2) | 9 insns / 32 B (16 with the fold naively enabled) | **7** (`vpmaxsb (mem), …` ; `vpminsb`) |

### 1.4g Follow-up 3.1 — memfold-aware operand resolution

Last session's attempt to fold the load into the first ALU op was reverted
because consumers read the elided load's never-written home.  The correct fix
landed in two parts:

1. **The three home/memory resolvers consult `pending_vec_memfold` first.**
   `vec_home_256` and `vec_home_128` now report an elided value as *un-homed*
   (so callers route it through `avx_load_arg_to`, which re-issues the load
   into the requested scratch), and `vec_arg_mem` hands back the recorded
   source operand instead of the never-written slot.
2. **The homed elision is gated on an audited consumer set.**  Making *every*
   one of the 53 `reg_assignments` lookups in the emitter memfold-aware is an
   unbounded proof obligation that has now failed twice -- the FMA/madd family
   resolves its scale operand through its own lookup and was the second
   failure.  `compute_vector_memfold_homed_ok` therefore names only the loads
   whose consumer is a plain two-operand 256-bit binary, i.e. one emitted by
   `emit_avx_binary_256`, whose memfold path runs as its FIRST action.  That
   turns the obligation into "these consumers are audited", which stays
   checkable as emitters are added.

Result: `vpmaxub (%rsi,%r11), %ymm3, %ymm5` -- the byte clamp is 7
instructions per 32 bytes, and `vec_load_sink_memfold` hashes bit-identically
to GCC.

### 1.4h Follow-up 3.7 — a differential model of the two interpreters

`transform_map_vector` hands one `MapExpr` to two consumers that share no
code: `MapEmitCtx` (packed intrinsics) and `emit_map_scalar_tree` (the scalar
remainder).  **Three of the five defects in §1.5 were disagreements between
them** -- signedness, constant replication, element size -- and each produced
a program that is right for the first `n - n % VF` elements and wrong for the
tail.

`map_expr_interpreter_tests` models both interpreters semantically and
asserts they agree on 3000 randomly generated trees per lane width, over an
input domain that includes both signed extremes.  It also pins the invariant
that makes the agreement possible -- masks appear only in condition positions,
because a packed compare yields all-ones while a scalar one yields 0/1 -- and
includes a test that this invariant is **not vacuous** (a mask used as a value
provably diverges) plus one asserting that the strength reduction's output is
packed-only, since it deliberately violates the invariant.

### 1.4i OP-05g — 16-bit lanes (follow-up 3.6)

The demotion argument is width-parametric, so 16-bit lanes reuse the byte
machinery verbatim rather than growing a second copy: `narrow_domains(n)`
returns the unsigned and signed domains of an `n`-byte lane, and every rule
that read `U8_RANGE`/`I8_RANGE` now reads those.  What is genuinely
width-SPECIFIC is small and all of it is encoded:

* **`Mul` is admitted at 16 bits and refused at 8.**  `pmullw` keeps the low
  half, which is a ring homomorphism exactly like the additive ops; x86 has
  no packed byte multiply at any width.
* **Signed word min/max are SSE2 BASELINE** (`pminsw`/`pmaxsw`) -- unlike
  both the byte forms (SSE4.1) and the dword forms (SSE4.1) -- so the word
  clamp folds at *both* vector widths.  The unsigned pair is SSE4.1, hence
  AVX2-only, and the 128-bit path keeps compare+blend rather than bailing.
* **`vpblendvb` is exact for word masks**: a word compare sets all 16 bits of
  its lane, so both bytes of every lane carry the same sign bit.
* Constants replicate as `w * 0x00010001`; runtime invariants splat with
  `vpbroadcastw` (SSE2: `movd`+`punpcklwd`+`pshufd`).

Everything else -- the unsigned range fusion, the single-bit window union,
the select strength reduction, the load memfold, the register-home classes --
carried over unchanged.  Measured, `-O3 -march=x86-64-v3`, 16 elements/YMM:

| kernel | packed body |
|---|---|
| `short` clamp | **7** insns / 32 B: `vpmaxsw (mem), %ymm3, %ymm5` ; `vpminsw` |
| `s[i]*3+7` | **7** insns / 32 B: `vpmullw (mem), %ymm2, %ymm5` ; `vpaddw` |
| `1000 <= c <= 2000` | **8** insns / 32 B: `vpaddw` bias ; `vpcmpgtw` ; `vpand` |

The range fusion firing at word width with no width-specific code is the
clearest evidence that the parameterisation is real and not three copies.

### 1.4j The SECOND CI failure: if-combine had no cost model

The codegen-quality gate in `bench.yml` — which `ci_local.sh` did not mirror —
failed with `expat_xml_scan: insns 203 -> 252 (+24.1%)`.

**Root cause.**  If-combine converts a control dependence into a data
dependence, and I shipped it with no profitability test at all: it fired on
every loop-body chain it could recognise.  `expat_utf8_name_length` is a
UTF-8 scanner whose loop body contains **five `break`s**, so it can never be
vectorized; predicating its
`(ptr[1] & 0xc0) != 0x80 || (width > 2 && ...) || (width > 3 && ...)` chain
made the common path — which exits on the first test — perform all the work
of the last.  58 -> 90 instructions in that function alone.

**Fix.**  A profitability gate expressing the transform's actual purpose:

* **exactly one exiting block.**  That block carries the loop control and its
  branch must survive; every other conditional branch in the body is a value
  computation predication can remove.  Several exiting blocks means
  data-dependent exits that predication cannot remove, so the branches stay
  and the combine is pure added work.  Note the exiting block is NOT required
  to be the header: after loop rotation the live exit is the LATCH, and
  requiring the header rejected every rotated loop — including the classifier
  this transform exists for.
* **no calls or inline asm in the body** — a barrier the vectorizer cannot
  cross, so straight-line-ness cannot pay for itself.

I also tried a stronger, "measure don't predict" gate: apply the combines on
a private clone, run the combine/diamond fixpoint, and keep them only if the
body became branch-free.  It is recorded here because it *failed* for an
instructive reason — the probe can only run `if_convert`'s own fixpoint,
while the real pipeline interleaves `cfg_simplify`, `copy_prop` and
`bool_thread` between rounds, and those are what expose the later diamonds.
The probe therefore under-approximated and rejected the classifier.  The
structural gate above is weaker in theory and correct in practice.

Result: `expat_xml_scan` back to exactly its 203-instruction baseline, all
seven golden workloads PASS, and the classifier got BETTER (the load now
folds into the `vpor`).  `scripts/ci_local.sh` now mirrors `bench.yml` too —
a mirror that covers one workflow is not a mirror.

### 1.4k OP-06 — induction-variable fusion (follow-up 3.2)

Every packed map body carried two proportional induction variables: the
element counter (bounded by `floor(limit / VF)`) and the byte offset used for
addressing.  It incremented both and compared the one NOT used for
addressing:

```
addq $1,  %r9      ; element counter
addq $32, %r11     ; byte offset  == %r9 * 32
cmpq %r8, %r9
```

Two changes retire the counter:

1. **The exit test is retargeted onto the byte IV.**  The counter runs over
   `0 ..< T`, so the byte IV runs over `0, S, ..., (T-1)*S` with
   `S = VF * elem_size`, and the loop must continue exactly while
   `byte_iv < T*S`.  `T*S` is loop-invariant and is materialised in the
   preheader.
2. **The remainder's resume index is derived from the byte IV** by a shift
   (`byte_iv / elem_size`, and nothing at all for byte lanes) instead of a
   multiply of the counter.  Without this the counter still had a reader and
   survived DCE.

**Signedness is load-bearing.**  For a signed loop (`int i; i < n`) the trip
count `T = floor(n / VF)` is NEGATIVE when `n` is, and the loop must run zero
times.  A signed byte compare reproduces that exactly (`byte_iv >= 0 > T*S`
is false on the first test); the unsigned form reads the negative limit as an
enormous positive bound and runs off the end of the array.  My first version
used `Ult` unconditionally and segfaulted `affine_map_vectorization` — the
predicate now follows the source's signedness, and `<=` needs no adjustment
because `byte_iv` takes exactly the `T+1` values `0, S, ..., T*S`.

Measured on the casefold family, `-O3 -march=x86-64-v3`, hot-loop
instructions per input byte:

| kernel | LCCC before | **LCCC now** | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---|---|---|---|---|---|
| `fold_lower` / `fold_upper` | 0.3125 | **0.2812** (9 / 32 B) | 1.2500 | 0.2422 (4x unrolled) | 2.00 / 1.375 | 0.6250 |
| `classify_alpha` | 0.3125 | **0.2500** (8 / 32 B) | 0.3125 | 0.2109 (4x unrolled) | 4.2500 | 0.5625 |
| `clamp_bytes` | 0.2500 | **0.1875** (6 / 32 B) | 0.1875 | 0.1172 (4x unrolled) | 0.4375 | 0.3750 |

LCCC now beats GCC on the classifier and both case folds, and matches it
exactly on the clamp.  Every remaining gap to Clang is its 4x unrolling of
identical per-element work.

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
| `clamp_bytes` | 0.2500 (8 / 32 B) | 0.1875 | 0.1172 (4× unrolled) | 0.4375 | 0.3750 |
| `classify_alpha` | **0.3125** (10 / 32 B) — was scalar | 0.3125 | 0.2109 (4× unrolled) | 4.2500 | 0.5625 |

Against the **non-unrolled** competition LCCC wins or ties every kernel:
2.0× better than ICX and 4.0× better than GCC on the case folds, 1.8× better
than ICX and level with GCC on the classifier, 1.5× better than ICX on the
clamp.  Clang's remaining advantage everywhere is 4× unrolling of identical
per-element work — a scheduling change, not a lowering change (§3.3).

`classify_alpha` packed body after OP-05e (10 instructions / 32 bytes), the
whole `isalpha` predicate in three packed ops:

```
vmovdqu  (%rsi,%r11), %ymm6
vpor     %ymm2, %ymm6, %ymm7     ; c | 32      (single-bit window union)
vpaddb   %ymm3, %ymm7, %ymm8     ; + bias      (range fusion)
vpcmpgtb %ymm8, %ymm4, %ymm9     ; vs threshold
vpand    %ymm5, %ymm9, %ymm10    ; & 1         (select strength reduction)
vmovdqu  %ymm10, (%rdi,%r11)
addq     $1, %r9                 ; <-- redundant, see 3.2
addq     $32, %r11
cmpq     %r8, %r9
jb       .LBB2
```

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

Follow-ups 3.1 (memfold-aware resolution), 3.2 (IV fusion), 3.4
(`vpbroadcastb` + promotion range recovery), 3.5 (`vpminsb`/`vpmaxsb`), 3.6
(16-bit lanes) and the testing half of 3.7 are all **DONE** -- see 1.4e
through 1.4k.  Two items remain.

### 3.1 Unroll the packed map body 2x  (MEDIUM — the only gap left to Clang)

Clang's remaining lead on every byte kernel is 4x unrolling of identical
per-element work; after OP-06 the loop overhead is 3 instructions out of
6-9, so it now DOMINATES the tight kernels.  Two independent 32-byte streams
would halve that share and fill both load ports:

| kernel | now | 2x unrolled (projected) | Clang (4x) |
|---|---|---|---|
| `clamp_bytes` | 6 / 32 B = 0.1875 | 9 / 64 B = 0.1406 | 0.1172 |
| `fold_lower` | 9 / 32 B = 0.2812 | 15 / 64 B = 0.2344 | 0.2422 |
| `classify_alpha` | 8 / 32 B = 0.2500 | 13 / 64 B = 0.2031 | 0.2109 |

i.e. 2x unrolling alone would take the case fold and the classifier past
Clang and bring the clamp within 20%.

Not attempted here, for a reason worth recording: after OP-06 the remainder
loop must cover up to `2*VF - 1` elements instead of `VF - 1`, so the
`T = floor(limit / (2*VF))` computation, the resume-index shift and the
remainder's own trip bound all change together — the same three-way coupling
that made OP-06 segfault on the first attempt.  It needs its own session with
the trip-count sweeps (which already run every n from 0 at four element
widths) as the primary gate, plus a code-size heuristic so small-trip loops
do not pay for a body they never execute.

### 3.2 A shift rule for the demotion  (LOW)

Byte lanes have no packed shift at all, so `>>`/`<<` must stay refused there.
WORD lanes do have `psllw`/`psrlw`/`psraw`, so `vec_word_lane_map.c`'s
`w_shift_refuse` could become `w_shift_folds`.  The demotion needs a rule:
a left shift by a constant `< 16` is low-16-exact; a logical right shift is
exact only when the operand's range is already inside `[0, 65535]` (the wide
value's high bits would otherwise shift in), and an arithmetic right shift
only inside `[-32768, 32767]`.

### 3.3 Remaining structural debt

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
  interpreters of `MapExpr`** -- now covered by
  `map_expr_interpreter_tests` (1.4h).  The remaining half of this item is
  the *unified op table*: five overlapping classifier lists must still be
  edited by hand for every new op, and this session lost three build cycles
  to exactly that (a byte op missing from the map-broadcast consumer class
  spilled a constant to the stack; a word op missing from the same list did
  it again one width up).  A single `VecOpInfo` table -- lane width, element
  type, commutativity, register class, consumer kind -- queried by all five
  would make the next op family a one-line change, and a unit test could
  assert coverage the way `declared_width_agrees_with_the_lane_count_in_the_name`
  already does for widths.
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
| `src/passes/if_convert.rs` | OP-05e if-combine (loop-body scoped) + the diamond phase split out so the context can be rebuilt |
| `scripts/hot_loop_metric.py` | new — steady-state instruction density from assembly |
| `tests/regression/vec_range_mask_fusion{,_sse2}.c` | new |
| `tests/regression/vec_byte_lane_map{,_sse2}.c` | new |
| `tests/regression/vec_class_union_ifcombine{,_sse2}.c` | new — union folds AND the two deliberate refusals |
