# Follow-up: session 60 — the PR #564 red-team audit, the domain-algebra and callee-proof hardening, and the CI closure

Date: 2026-09-20 (session 60)
Base: `9d10546f` (= upstream main after PR #563; PR #564's squashed head
`48932d13` sits on it and this session's four commits continue that branch).
Series: `95a930f4` range_fold domain/CFG/restart · `729445eb` constarr
callee-proof + oracle discipline · the env-ratchet fix · the verifier-wired
tests. Deliverable: `ms178-1.patch` (APPLIES-CLEAN vs `9d10546f`,
tree-exact in a fresh clone).

## 1. What this session shipped

The user ordered a full red-team audit of PR #564 (range-fold +
const-array promotion) in response to the first CI failure, plus an
independent Review-AI report to adjudicate. Every claim in that report was
verified against the code and the live compiler before acting; the verdict
and the disposition of each finding follow in §3.

### 1.1 range_fold — three live miscompile classes closed (95a930f4)

Each was REPRODUCED FIRST on the unmodified PR-head compiler, then fixed,
then re-verified:

- **Narrow-domain aliasing (U8).** `x >= 250 && x <= 260` on an
  `unsigned char` in VALUE context folded to `(u8)(x - 250) <= 10`, which
  reads the low byte of the wide sub and wrongly accepts `x = 0..4`
  (repro: `V1`/`V1B`, five wrong results per sweep, `cmpb $10, %sil`
  after `subl $250, %esi`). The narrowing gate had checked only the SPAN;
  it must check BOTH BOUNDS against the pre-promotion source domain.
- **Wrapped unsigned ranges folded as windows (U64).** `x >= u64max-9 &&
  x <= 10` is EMPTY; the raw-`i64` `lo <= hi` check saw `-9 <= 10`, and
  the fold accepted `0..10` AND `u64max-9..u64max` (repro: `V2`, 15 wrong
  results, `subq $-9; cmpq $19`). U32/U16 survive only by
  constant-spelling accidents (positive `I64` spellings, mixed compare
  widths) — the fix is spelling-independent.
- **`i64` span overflow.** Full-range bounds made `hi - lo` overflow Rust
  `i64` (a panic under `debug-assertions`, a wrapped always-true span in
  release).

The fix is one shared `RangePlan`: every bound is normalized through
`i128` in the comparison's OWN domain (`dom_value`, masking unsigned
spellings to their width), ordered there, spanned there (no overflow), and
only then checked for representability. Narrowing additionally requires
`fits_domain(lo)` AND `fits_domain(hi)` at the source type. 128-bit
comparisons reject explicitly. All four emission paths (Select, Boolean
op, phi diamond, branch chain) consume the ONE plan, so the algebra cannot
diverge between them — the bug had been COPIED between paths, which is
how it survived the original battery.

### 1.2 range_fold — three structural unsoundnesses closed (95a930f4)

- **Canonical CFG everywhere.** Both CFG folds and the path-fact scan used
  a bespoke successor scan (`Branch`/`CondBranch`/`Switch` only) that is
  blind to `IndirectBranch::possible_targets` and instruction-level
  `InlineAsm::goto_labels`. A check block reachable through a computed
  goto or asm goto was "single-predecessor" to that scan; deleting it
  would dangle the target. All three sites now use
  `ir::analysis::build_label_map` + `build_cfg` (the canonical builder,
  which counts both edge kinds — and `Switch` predecessors for the
  path-fact check, which the old inline scan ALSO missed).
- **Analysis-restart discipline.** The old single-pass structure consumed
  `cmp_defs`/`cast_defs` AFTER `fold_phi_diamonds` had rewritten operands
  (a later fold could emit a `Sub` over a deleted phi's stale operand
  record), and indexed `block_known_pos_i32` by PRE-removal block
  positions — a fold that deleted an early block shifted the `x > 0`
  path facts onto unrelated blocks, letting the gcc-torture 20041114-1
  unsigned-overflow fold fire UNPROVEN. The CFG folds now apply ONE
  rewrite per call and `run_function` restarts them with freshly built
  maps and CFG until neither fires (termination: each iteration deletes a
  block); path facts are computed only after all removals.
- **Phi-arm discipline.** A branch-chain fold whose shared exit carries
  phis with DIFFERING `Bcond`/`Bcheck` arms now rejects (both failure
  paths survive through the fused branch; the surviving arm would
  misselect), as does a phi-diamond fold whose merge has sibling phis with
  differing arms (the old code left those phis with an arm for a removed
  predecessor = malformed IR). Equal arms collapse to the single
  surviving arm; missing arms (malformed) reject.

### 1.3 const_array — the callee proof and the caller rewrite (729445eb)

- **Weak definitions never license the proof** (a strong override
  replaces the symbol at static link — the same rule `inline.rs` and
  `ip_purity.rs` apply, with the kernel `vmemmap_set_pmd` failure
  documented there). The ELF-preemption policy for default-visibility
  bodies is now stated IN the code: lccc's executable-only output model
  makes the module's definitions the linked ones; if `-shared` lands, the
  gate tightens to non-preemptible definitions.
- **A duplicated `ParamRef` rejects** (the old scan audited whichever it
  saw last).
- **Intrinsic read side classified by op, not by elimination.** The new
  `IntrinsicOp::reads_pointer_arg` (load families + memory-foldable
  vector families) is the only allowlist through which a pointer may
  reach an intrinsic argument: `BuiltinSetjmp`/`BuiltinLongjmp`/
  `DoBuiltinApply`/`RestoreApplyResult` write their buffers through
  `args` with no `dest_ptr` and no `writes_memory_via_args` membership —
  a setjmp buffer promoted into `.rodata` was a SIGSEGV class — and
  scalar pure ops (`Crc32`) have no legitimate pointer argument. Future
  opcodes default to rejection.
- **The observed tier.** A pointer-to-integer cast moves the value into a
  tier whose only legal use is casting back to a pointer: promotion must
  not change what the program computes from the address. (A pure
  `ptr -> int -> ptr` round trip stays promotable — unit-tested both
  ways.)
- **Caller-side derivations now actually work.** The module doc claimed
  any address-dataflow chain may reach a permitted read, but the
  catch-all rejected Cast/Copy/Phi/Select mentions. They are now
  whitelisted mention positions AND rewritten by `apply_one` (the IR's
  untyped `IrType::Ptr` makes the substitution exact at every hop), so
  `const T *p = a; reader(p);` promotes instead of conservatively dying.
- **Engineering:** `.LCA_n` names probe the occupied-name set (sparse
  pre-existing labels can no longer collide with a count-based suffix);
  write-only objects (no load, no read-only call — initialization GEPs
  do not count) no longer promote into permanent dead `is_used`
  `.rodata`; dominators compute at most once per function; the callee
  proof caches by `(callee, argument index)` across the module audit.

### 1.4 The CI closure

Two independent reds, both closed:

1. **`const_array_promote` differential (the reported failure).** The
   corpus's `a8()` evaluated `p != 0` on the pointer returned from a
   dead stack frame — undefined behavior and an invalid oracle. GCC
   legitimately folds the comparison to 0 (its build's own self-check
   failed with `MISMATCH a8() ctx=89: got 4 want 13` while lccc's
   passed; the outputs diverged and the suite flagged it). The corpus
   keeps `leaky` compiled but uncalled (`used`), and the no-ICE /
   no-promotion contracts live in the structural gate. The gate's own
   `leak.c` main no longer evaluates the dangling pointer either.
2. **`env-test-hygiene` budget breach (latent, never reached by the
   arena run because the Test Suite step failed first).** The
   session-59 `CCC_DEBUG_CONSTARR` knob put the pass-pipeline
   environment-read count at 158 against the 157 ratchet. The knob is
   deleted (its audits are now unit tests + the gate), restoring
   157 = 157.

### 1.5 Test-validity fixes (the oracle discipline)

- `range_fold_branch.c`: every hand oracle subtracts in the UNSIGNED
  domain — `(unsigned)x - (unsigned)K`, never `(unsigned)(x - K)` —
  because the signed spelling overflows `int`/`long long` at the swept
  `INT_MIN`/`INT_MAX`/`LLONG_*` extremes, and a differentially compared
  oracle must itself be defined C.
- New adversarial shapes, both branch and value-context forms: U8
  domain-exceeding bounds (`[250, 260]`), wrapped U64/U32 empty ranges,
  I64 full-range spans (the panic class).
- `const_array_promote.c`: P4/A6 use UNION spellings so every access is
  defined C (the old `(unsigned short *)(buf + 2)` punning violated the
  effective-type rules and read uninitialized bytes); the gate helpers
  are `static noinline` (the body-inspection policy made explicit).
- The gate's alignment assertions are scoped to the promoted object's
  own `.align` directive (any unrelated `.align` in the TU can no longer
  satisfy them).
- **31 new direct unit tests** (22 range, 9 constarr) including the two
  live repros, the goto/asm-goto predecessor rejections, the
  differing-arm phi rejections (&& and ||), nested-chain restart, the
  index-shift path-fact regression, the stale-def-map consumer rewrite,
  and — per the audit — **the repository SSA/CFG verifier runs on every
  transformed test IR** (`verify_after_func_pass` / `verify_after_pass`).
- `check_range_fold_branch.sh` is wired into BOTH `ci.yml` and
  `ci_local.sh` (it was registered in neither).

## 2. Verification (final tree, `405556e2`)

- `ci_local.sh --fast`: **49 passed, 0 failed** (range-fold-branch gate
  included; peephole-whitespace run separately: PASS).
- Full regression suite (`CCC_VALIDATE_SSA=1`): 763 passed, 3 failed —
  the documented environmental i686 multilib-header set
  (`bits/libc-header-start.h` missing; identical on the pristine base),
  11 skipped-compare, 9 skipped-run. The CI-failing
  `const_array_promote` now passes AND matches GCC output exactly.
- Benchmark output oracle: **204/204 PASS**. Codegen-quality gate:
  within tolerance (the hardening's conservatism does not touch the
  measured shapes — their promotes are pinned by the structural gates).
- `cargo test`: **2976 passed, 0 failed** (7 pre-existing ignores).
- `cargo fmt --check` green; `cargo clippy --all-targets -- -D warnings`
  green. Zero warnings.
- Fresh-clone verification: the deliverable applies clean to `9d10546f`
  and produces the identical tree SHA.

## 3. The Review-AI report adjudicated

**Agreed and fixed (verified live before fixing):** the U8-narrow and
U64-wrap miscompiles (reproduced: 15+5 wrong values), the `i64` span
overflow, the incomplete CFG scans, the stale-analysis consumption
(including the path-fact index shift), the weak-callee trust, the
BuiltinSetjmp and crc32-pointer-identity holes, the a8/leak UB oracle
(reproduced the exact CI divergence), the missing range-gate CI wiring,
the `.LCA_n` count-based naming, the dead write-only `.rodata`, the
dominator/cache recomputation, the doc-vs-code mismatch on caller-side
derivations, the `Ule` doc typo, and the missing direct IR tests.

**Agreed in substance, severity corrected:** the differing-Bexit-phi-arms
fold. The demanded check is implemented (and unit-tested both ways), but
the report's claim that "a source shape that distinguishes below-range
from above-range can be miscompiled" is NOT reachable through today's
frontend: a differing arm value must be defined between the two tests
(i.e. in Bcheck), and Bcheck's purity plus the whole-function use counts
(which DO count phi-arm uses — verified against
`Instruction::for_each_used_value`) reject every such construction. The
class was defense-in-depth against other-pass-produced IR and against the
fold's own arm retargeting — worth closing, and closed.

**Disagreed:** the report's U32 wrapped-range example survives today by
accident (the positive `I64` constant spelling trips the signed `lo > hi`
check), which is exactly why the fix is domain math rather than a
spellings patch. The "convert a8 to compile-only" instruction was adopted
with the refinement that `leaky` stays EMITTED (`used`) so compile
coverage (no-ICE) is retained. The one-commit process complaint cannot be
remedied without the force-push the same report forbids; the follow-ups
are focused commits.

**PR #562 (unmerged) audited:** the if_convert clone-ID reservation fix
(reserve before recursion — the self-shift doubling class), the
Clz/CtzNonZero split with fail-closed constant folding, and the
dominance-scoped cvp specialization are sound; no interaction with the
range folds beyond providing them better-shaped input.

## 4. Follow-ups

1. `-shared` output support, when it lands, must tighten
   `callee_readonly_through` to non-preemptible definitions (the policy
   note marks the exact site).
2. The memfold read-side allowlist (`reads_pointer_arg`) should grow
   per-ARGUMENT-INDEX precision if a future op mixes pointer and
   non-pointer args (today the families are homogeneous).
3. Full-domain range folds (`span == 2^width - 1` → constant true /
   unconditional branch) are deliberately still rejected (status-quo
   threshold; GCC folds them). A follow-up could fold to the
   unconditional form via the plan's all-ones span with a backend
   `Ule(v, -1)` → true simplification.
4. The `set_membership` pass consumes range-fold output; its own domain
   arithmetic deserves the same `i128` normalization audit.
5. Cross-PR: once #562 lands, re-run the range battery (its if_convert
   SSA fix changes the shapes the phi-diamond fold sees).
