# The ten-PR red-team audit (session 574)

Session: 2026-09-22 · base `a8baa17d` (PR #573 merge — the madd VLFOLD
revival, tree-exact absorption of the session-572 delivery verified by
empty diff against the delivery branch).

Scope: the last ten merged PRs — #557, #559, #560, #563, #566, #567,
#569, #571, #572, #573 — audited AS A STACK: every per-PR gate re-run,
plus a fresh cross-PR interaction battery built for this audit
(`tests/regression/cross_pr_redteam{,2}.c` + the gate), four-oracle
surveys where a gap appeared, and code-level reads of the riskiest hunks
(copy deletion legality, ISA ceilings, peel grammars, NaN propagation).

## Method

The per-PR gates pin each PR's own contract; an audit that only re-runs
them re-proves nothing about INTERACTIONS. The battery stresses the
cross-products: FMA families × copy brackets × if-conversion ×
constant-array promotion × NonZero guards × wide SLP packs × the
AVX1+FMA target class × the SSE2 baseline — every FP edge value
(±0, ±inf, NaN, extremes) with bit-exact comparison against GCC at
matched march (the sign of zero and of infinities included: that is
what pins the FMA family selection). Canonical NaN patterns are
normalised on both sides under the C11 6.5p8 latitude — after this
session's fix, ZERO rows differ for any other reason.

## Verdicts

### #573 (madd VLFOLD revival + the P0 width fix) — AGREE, with one
### residual now measured

The revival itself is sound and the destructive-form dst-homing
(`vector_dying_values`: single total use + same-block def + live-regs
claim) is the right analysis — a plain use count is unsound for the
loop-invariant broadcast, and the live-regs conjunct is exactly what
excludes it. The P0 (width-4 F64 / width-8 F32 packs lowering through
128-bit families) was the audit's own find at delivery and the width
routing is correct. **Residual, now measured with the four oracles: the
phi-diamond shared-negation shape** (one `-b`/`-c` read by two fma
sites) stayed materialised — lccc 22 instructions vs gcc 10, clang 12,
icx 12 (icc 25 with ten spills). Root cause was in the #572 peel
grammar (below), fixed this session.

### #572 (FMA operand-negation peel) — AGREE on the design, DISAGREE
### on one grammar rule (fixed)

The IR-layer peel over peephole/generation alternatives is the
architecturally right choice — dominance soundness by construction, the
regalloc sees true live ranges, every backend gains the families
through one intrinsic arm. The CPU-verified 132/213/231 table and the
"round(−x) = −round(x)" value proof stand. **The single-use rule was
wrong** — and my own rule, delivered two sessions ago: it rejected every
multi-use Neg, so the shared-operand shapes (the phi diamond; GCC
contracts them) kept `vxorpd`+`vxorpd`+plain `vfmadd`, and
`fma(-a,-a,c)` (GCC folds it) was left alone as "a pathological
spelling". Fixed with the absorbability fixpoint: a Neg peels iff every
read of it disappears with the rewrite (fma argument slots all peel it;
chain-internal reads by other absorbed Negs) — computed as poison
propagation backward through chain links. The discipline that replaces
the old conservatism: the NaN-sign latitude the absorption takes (the
hardware propagates a source NaN's sign UNNEGATED through the negated
families — CPU-verified this session in every 132/213/231 source role,
which is why GCC's contracted forms and lccc's now agree bit-for-bit)
is only taken when the materialisation actually dies; a Neg with a
surviving reader keeps its exact materialised semantics and no site
peels it. Result: the phi shape is GCC's two `vfnmsub231sd`, the
edge-value matrix is bit-identical, and `fma(-a,-a,c)` cancels to the
plain family exactly like GCC.

### #571 (FMA form algebra, packed contraction) — AGREE

The rounding-parity bug it fixed (the scalar path contracts
`acc±a·b`, so a packed mulpd+addpd paid a third rounding the scalar
never takes) was real and the fix is the right one; the
register-alias discipline over the three dying operands is validated by
the differential. The map/SLP split (MapExpr::Fma for loops, PackKind::Fma
for stores) is a clean two-entry surface for the same algebra. No new
findings; the battery's H section re-pins all four widths bit-exactly.

### #569 (range-check fusion, peephole, NonZero, kernel boot) — AGREE

The two miscompiles it fixed (NonZero guard-select collapse speculating
BSR/BSF garbage above its proof; the phi-diamond hoist faulting loads
across branches) were verified live at delivery, and today's battery
adds independent coverage: the guarded clz/ctz over promoted constant
arrays (E section, values 0 and 1 in the table) and hoist shapes in the
phi-FMA composition are bit-exact. The regalloc delta is two lines
(i686 scratch hazards for the NonZero bitcount ops) — consistent with
the audited NonZero work. The range-fusion domain corners measured
here: empty domain, point domain, INT_MIN, unsigned OR-wrap, FP NaN
non-fusion, reversed operands — all bit-exact vs GCC. The parity
checker's execution semantics (PyYAML run-body extraction) survived its
negative controls and now guards 48 commands.

### #567 (vector copy elimination + EVEX aliasing) — AGREE

The core is the best-engineered peephole in the tree: copy forms carry
their upper-bit semantics (`movsd` legacy preserves, `vmovsd` VEX
zeroes — the widen rule's soundness precondition), compare-class
instructions answer "no vector write" for their last operand, the
three-operand merge reads its middle operand's upper lane, and
`parse_vec_copy` declines exactly the forms whose lane algebra it
cannot state. Its gate (46/46 today) covers every legality rule with
runtime differentials. **Residual, measured this session:** the copy
elimination is block-local, so the phi-diamond's entry staging
(`movsd %xmm1, %xmm3` family) survives across the branch — that plus
the destructive-form dst preference for multi-use accumulators is the
remaining 10-instruction gap on the phi shape (lccc 20 vs gcc 10 after
this session's family fix). Designed follow-up: cross-block bracket
coalescing.

### #566 (range_fold hardening, constant-array promotion) — AGREE

The domain-ordered emptiness and no-overflow spans were re-derived at
the pr568 adjudication and the battery adds the aliasing corner: a
const table written through a cast-away-const pointer is OBSERVED by
the promoted form (bit-exact vs GCC). The red-team regression battery
it shipped is the reason the empty/point/wrap corners were already
covered.

### #563 (whitespace-invariant peephole store, ISA-gated vectorizer,
### hoisted env knobs) — AGREE

The whitespace invariance was the right fix for a matcher that silently
depended on emitter spacing; the ISA gate closed the kernel-TU #UD
class; the env hoisting killed eight environ scans per function. The
gate re-ran green today, including at -O3.

### #560 (docs + evidence refresh) — AGREE

310 files but 128,890 of the insertions are evidence artifacts (oracle
asm dumps, manifests); the claims it refreshed are the ones the oracle
surveys re-derive. Spot-checked the performance tables against today's
oracle numbers: no contradiction.

### #559 (relocation addends, branch relax, kernel codegen) — AGREE

The i686 asmdiff corpus (21/21 objects, byte-exact vs GAS) re-verified
at the pr568 session stands; the corpus re-ran green today (723 PASS /
3 known-env i686 multilib / 0 AB-diff).

### #557 (codegen hardening: .code16 routing, hex-constant labels, rcx
### liveness, typed-call homes) — AGREE (it is this tree's most-audited
### PR; the fixes folded into it were verified with negative controls
### at delivery and the realmode corpus re-runs green)

## The defects this record covers (all fixed, all gated)

0. **The fixpoint's poison direction (found by the post-merge review,
   PR #574 follow-up)**: the multi-use rule shipped with badness
   propagating from a Neg's SOURCE to its reader — the exact inverse of
   the invariant the same patch documented. A Neg read by an fma site
   AND by a surviving outer Neg was classified absorbable, rewritten,
   and deleted while the outer link still named it: a live ICE on
   `t=-x; u=-t; r=fma(t,b,c); return r+u` at -O1/-O2/-O3 (x86 codegen:
   `value has no register, stack slot, Copy, or GlobalAddr definition`),
   invisible to the armed corpus because nothing verified after the
   peel (it was the last IR transform). The one-line edge flip the
   review note proposed is NOT sufficient (its own model enumeration:
   90/4545 shapes still unsafe — a surviving reader that no fma chain
   reaches); the correct rule is the GREATEST FIXPOINT over the reader
   closure and the fma-reachability TOGETHER (model: 0/4545 unsafe,
   0/162009 at width 4; and it folds MORE than the shipped rule —
   1752 vs 1722 shapes — because the source-poisoned direction also
   over-pinned chains whose inner link alone was external). Fix landed
   with: the closure fixpoint (`tools/ir_shape_check.py` keeps both
   failure modes pinned as negative controls); a self-enforcing
   pre-sweep assert on the ACTUAL decision set; `verify_after_pass`
   wired after the peel at both call sites; the peel extended to
   already-Signed sites (families compose by XOR, full cancel
   normalises to plain) so every `FmaArg`-classified read truly
   disappears — the classification is true by construction, not by the
   coincidence of `VecMadd*` reads pinning the Neg; 5 new unit tests
   (16 → 21); the chain shapes in the batteries + the ISA-gate asm
   pins. The verify wiring immediately caught a SECOND latent
   malformation: `vector_temp_promote` leaves `Alloca` defs below the
   intrinsics that use them as `dest_ptr` (inlined-callee param slots
   positioned after the caller's producers) — benign for codegen but
   an IR-contract violation; fixed with a block-head alloca hoist
   (Alloca reads no operand, so the move is semantics-preserving).

1. **The multi-use peel rejection** (root cause in #572's grammar,
   measured via #573's residual): fixed with the absorbability
   fixpoint; `fma(-a,-a,c)` now cancels like GCC; the phi shape went
   from `vxorpd+vxorpd+vfmadd+vfmadd` to two `vfnmsub231sd`; the full
   edge matrix is bit-identical to GCC at matched march. (The fixpoint's
   own direction bug is defect 0 above.)
2. **`-mno-avx2` killed the VEX encoding, not the 256-bit class**: the
   flag set `avx_explicitly_disabled`, so `x86_isa().avx` went false
   and `normalized()`'s `fma && avx` declined the FMA fold —
   `fma(-a,b,-c)` at `-mno-avx2 -mfma` compiled to two `xorpd` and a
   libm call where GCC emits one `vfnmsub132sd`. Fixed with a dedicated
   `avx2_explicitly_disabled` consulted only for the `ymm` bit; the
   AVX1+FMA target class (GCC's `-march=corei7-avx` relatives) now
   keeps every VEX.128 form. The ISA matrix is pinned in
   `check_vectorize_isa_gate.sh` §4b and the value contract in the new
   cross-PR gate.

## Design divergences adjudicated (no code change — upstream's call)

* **The absent-`-march` default is x86-64-v3** (documented in
  `pipeline.rs`): every other compiler defaults to the generic baseline.
  This is a portability trap for any build system that compiles with
  bare `lccc -O2` — my own first battery run fell into it (the NaN
  rows "differed" only because lccc was emitting FMA3 the baseline gcc
  run could not). It is deliberate, documented, and `-march=x86-64`
  restores GCC-exact behavior; the audit flags the risk, not a defect.
  Worth considering upstream: a one-line note in `--help` output, since
  the divergence is otherwise invisible until deployment.
* **`ymm` is one bit for both AVX1 256-bit FP and AVX2 integer**: lccc
  cannot express "AVX1 exactly" (`-mavx -mno-avx2` keeps VEX.128 but
  drops 256-bit FP loads/stores too). Conservative and sound; GCC
  distinguishes. A future split (ymm_fp vs ymm_int) would close it.

## Residuals, priority-ordered

1. **Cross-block copy coalescing + the destructive-form dst preference
   for multi-use accumulators** (the phi shape's remaining 10-insn gap,
   2.0x → 1.0x): the entry staging survives because the #567 machinery
   is block-local, and each fma stages its own accumulator copy when
   the accumulator is multi-use. Design: extend bracket coalescing
   across single-predecessor chains; teach the 213/231 dst choice to
   prefer the register that needs no staging (GCC's b→xmm3 move is the
   model — ONE staging copy for the whole diamond).
2. **The map (loop) side of the multi-use peel**: `parse_fma_arg` still
   requires the loop-local count == 1, so two map-FMA nodes sharing one
   `-x[i]` parse the negation as a broadcast. Same absorbability
   discipline applies (all loop-uses are fma-arg reads), but the
   cross-parse coordination needs the loop-body whitelist argument
   first.
3. `-march=x86-64-v2 -mfma` declines the families (normalized():
   fma requires avx, and v2 has no avx): GCC enables FMA there (the
   user vouches for the target). Conservative-decline is sound; a
   `fma implies vex` rule in the ceiling would match GCC.
4. The staging-`movsd` discipline generally (the fresh-temporary
   routing) — the #567 follow-up list stands.

## Verification (this session, at the fixes)

* New cross-PR gate: 4 build configurations × 2 batteries, bit-exact vs
  GCC (canonical NaN normalisation only) — PASS.
* ISA gate (extended §4b): PASS. Vector-copy gate 46/46. Adler gate
  PASS. Env hygiene, parity (48 commands) PASS.
* Regression corpus: 723 PASS / 3 known-env i686 multilib / 17 skip,
  AB-diff 0. Benchmark output oracle: 204/204.
* cargo test: the peel's 21/21 — 12 pre-fix tests carried over, of
  which the two-position pin was REWRITTEN in place
  (`same_value_in_two_positions_is_not_absorbed` → `..._cancels_to_plain`),
  plus 4 new multi-use/chain/cross-block/external-reader tests and (with
  the fixpoint correction) 5 more: the shipped defect's shape, the
  dead-outer-reader control, the pinned-inner fold, and the two
  already-Signed-site XOR compositions.
* Oracle: phi_fma 22 → 20 insns (gcc 10, clang/icx 12) — the family
  selection is now GCC-exact; the remainder is the staging residual #1.

## Post-merge follow-up (2026-09-23, the fixpoint correction)

The post-merge review correctly flagged the multi-use fixpoint's poison
direction (defect 0 above). Adjudication summary, with every claim
re-verified against the tree and empirically:

* CONFIRMED live: the chain shape ICEs the shipped compiler at
  -O1/-O2/-O3 (x86 emit guard); the IR dump shows the deleted inner Neg
  under the surviving outer read. -O0 is unaffected (GCC-matched gate).
* CONFIRMED: the review's own one-line remedy is insufficient — its
  harness (landed here as tools/ir_shape_check.py, 4545 shapes at width
  3, 162009 at width 4) reproduces 738/4545 unsafe for the shipped rule,
  90/4545 for the edge flip, 0 for the reader-closure fixpoint.
* DEVIATED from the review's work order where it was wrong: its STEP 4a
  and 4c test sketches are the SAFE MIRRORS of the defect shapes (the
  site reading the OUTER link), which pass on the shipped rule too —
  false greens. The landed tests use the discriminating shapes (site
  reads the INNER link; pinned-inner fold). The review's "5 added" test
  count was also wrong (git: 4 added + 1 rewritten); the doc
  parenthetical is now the git-verified decomposition.
* EXTENDED beyond the work order: the peel now also absorbs Negs off
  already-Signed FmaScalar sites (XOR composition, cancel normalises to
  plain). Rationale: the FmaArg classification claimed Signed-site reads
  disappear, but Pass 2 never rewrote Signed sites — sound today only
  because the vector main lanes pin the Neg (VecMadd reads classify as
  Other). The audit's residual #2 (the map-side multi-use peel) removes
  exactly that pin; the class is closed by making the classification
  true instead of coincidentally conservative.
* The new verify hooks (both peel call sites) caught a second latent
  IR-contract violation on their first armed corpus run:
  vector_temp_promote rewrites producer dest_ptrs to slots whose Alloca
  sits below them (inlined-callee param slots after the caller's
  producers). Fixed with a block-head Alloca hoist (no operands, no
  semantics change; spans in lockstep).
* Suite integration gaps closed: the batteries now carry matched-arch
  .flags (the suite's raw oracle was comparing lccc's default-v3
  contractions against the reference compiler's default no-FMA
  double-rounding — the C11 6.5p8 latitude, bit-visible in phi_fma's
  1-ulp rows and bracket_signed's NaN signs; the matched oracle is
  bit-exact RAW, NaN normalisation kept at print time for the
  dedicated gate's cross-baseline configs), and the ISA gate's
  shared-negation xorpd pin was vacuous for VEX (`\bxorpd\b` can never
  match `vxorpd` — no word boundary between v and x) — now `v?xorpd`,
  with the counts re-verified by hand.

Validation at the follow-up tip: cargo 3080/0; peel 21/21; harness
fixtures ok + closure 0 unsafe (both widths) + controls ok; corpus 725
PASS / 3 known-env i686 / 17 skip / AB-diff 0 with CCC_VERIFY_IR=abort
armed; cross-PR gate 4x2 bit-exact; ISA gate with the new chain pins
(chain_live: exactly 2 sign masks, 0 negated families; chain_pin: 1
mask + 1 vfnmadd; shared_neg: 0 masks + 2 vfnmsub); ci_local --fast
55/0/4; rustfmt + clippy (lib/tests/bins) clean; CCC_VERIFY_IR=abort
matrix 30/30 (batteries + chain repro, -O1/-O2/-O3, with/without
-mno-avx2 -mfma); the live repro computes GCC's exact -5 at every
optimisation level.
