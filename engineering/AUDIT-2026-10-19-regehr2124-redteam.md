# AUDIT 2026-10-19: red-team review of the REGEHR-2124 patch stack

Scope: commits `ba39f8d0..HEAD` on base `2715bf90` (== `main`,
verified by `git ls-remote`: main had not moved; the rebase is a
no-op, and PR #871 merged today *is* the base). Method: re-read every
Rust hunk adversarially, probe each fix with cases its regression test
does not cover, re-run differentials, and measure with deterministic
tools (Cachegrind instruction counts; this VM has no PMU). Verdicts
are **KEEP** (sound as-is), **FIXED-DURING-AUDIT**, or **RISK**
(accepted, documented). Toolchains used: GCC 14.2 (+ multilib),
Clang 23.1.4, ICX 2026.2.0, godbolt pins GCC 16.2 / Clang 23.1.0 /
ICC 2021.10 (`godbolt.py audit`: all current).

## My delta, component by component

### `_Bool` constant-fold fix (4 sites) — KEEP

Adversarial probes beyond the regression: float sources
(`(_Bool)256.0`, `(_Bool)(-0.0)`, `(_Bool)1e300`, `(_Bool)(1.0/255.0)`),
`-(_Bool)256`, static `_Bool` from float — all match GCC at `-O0/-O2`.
Residual risk: the fold exists in **four** places (sema Cast eval,
sema `cast_i128_to_ctype`, both `eval_const_expr_as_bits`). I fixed
all four, but the duplication is the disease (see "other team" §1);
the next width/normalisation bug will reappear in the copy someone
forgets. Recommendation: one shared `cast_const_to_ctype` in
`common/const_eval.rs` that both evaluators call.

### Statement-expression conditional composite type — KEEP

Probed beyond the regression: pointer arms (`c ? p : (int*)0`,
`void*` vs `int*`, array-arm decay, GNU `p ?: p`) inside
declaration-bearing statement expressions — `sizeof` and values match
GCC. The fix reuses `CType::conditional_composite_type` with the same
NPC determination as `conditional_expr_ctype`, so sema and lowering
now agree by construction rather than by parallel reimplementation.
One deliberate asymmetry remains: the scope path cannot consult
sema's annotation to disambiguate NPC ties the way
`conditional_expr_ctype` does — but that path only runs where sema
has *not* annotated (unevaluated `typeof` operands), so there is no
annotation to consult. Accept.

### `-(unsigned)` fold modulo 2^32 (both folders) — KEEP

Class-swept before fixing: add/sub/mul/div/rem/shifts/bitnot on u32
constants were all already width-correct; only Neg lacked the
treatment its BitNot sibling had. Post-fix probes: negation through
casts (`-((unsigned)5000000000LL)`, `-((unsigned)vll)`),
`-(unsigned short)` promotion path, `-(4294967295U)` → 1, `-m32` —
all match GCC. Same duplication caveat as `_Bool` (two folders, two
identical fixes; a shared helper is the real cure).

### redundant_ext peephole (salvaged + hardened) — KEEP, one audit fix

The pass is sound as shipped: facts are only ever *cleared* on any
path the pattern matcher cannot parse exactly (that invariant is what
the classify_dst conversion bought, and the audit extended it to the
last two consumer guards and the movq producer guard). What the audit
found in *pre-existing upstream code* inside the same file:

* **cmov established `upper32_zero`** (FIXED-DURING-AUDIT, S10): the
  32-bit-ALU-write list included the whole cmov family, so a not-taken
  cmov left the pass believing in a zero-extended register and the
  `movq`→`movl` narrowing could drop a live upper half. Proven by a
  failing unit test before the fix. cmov now preserves a pre-existing
  fact through 32-bit forms and establishes nothing. The *other*
  cmov list (`candidate_uses_32bit_dest`, a profitability heuristic
  about the following line) is soundness-neutral and stays.
* Two `so > comma` consumer guards `continue`d past `self_update`,
  i.e. an unparsable movz line redefined its destination without
  updating facts (FIXED-DURING-AUDIT: run the producer update).

Known imprecision, accepted: 3-operand BMI forms (`shlx`…) sit in the
32-bit-write list but classify as `Unknown` → conservative clear.
Sound, merely unexploited (lccc rarely emits them).

### Python engines (mutpair, yarpgen_mutpair, slice_superopt) — KEEP

22 unit tests + both self-tests green on the final tree. The
execution filters are the soundness core and each has a
planted-collapse negative control. `yarpgen_mutpair` remains
*expensive* per seed (consensus GCC ×2 + lccc ×2−3 per pair on
50–250 KB TUs) and, like the classic engine, trips over lccc's
compile time on mega-functions before it finds deeper bugs; that is a
compiler problem (§"compile time"), not a harness problem.

### Bench kernels — KEEP, with measured (not claimed) status

Cachegrind, identical outputs, `-O3 -march=x86-64-v3`, hot function:
GCC-14 9.83M Ir, Clang-23 10.24M, ICX-2026.2 10.24M, **lccc
18.03M (1.83×)**. Godbolt statics agree: GCC-16.2 37 insns vs lccc
68. The gap decomposes (asm in the FOLLOWUP) into: (1) no
RIP-relative+index addressing for global arrays — `leaq tab(%rip)` +
`addq` re-materialised 4×/iteration where GCC folds
`movsbl tab(%rdx), %edx`; (2) byte extraction via movq/shrl/movzbl
where GCC/Clang store from `%dh`/low parts; (3) per-element `leaq
k(%i)` instead of pointer-bump/displaced loads; (4) I64 sign loads
(`movsbq`) where `movsbl` suffices. F-1 already removed the redundant
*second* extensions; the rest is isel/IR work that does not belong in
a peephole and was deliberately not forced into this patch. Ranked
follow-ups with exact shapes are in the FOLLOWUP.

## The other team's choices (upstream, incl. PR #871)

1. **Two constant evaluators** (sema + lowering) with parallel fold
   logics. Evidence this costs real bugs: both miscompiles I fixed
   this session existed in *every* copy (4 and 2 sites). I disagree
   with the duplication; the trend in recent commits (shared
   `common/const_eval.rs` helpers like `promote_sub_int`,
   `eval_const_expr_as_bits`) is the right direction and should be
   finished — one folder, two thin drivers.
2. **Textual peephole architecture** (LineInfo classification +
   liveness solves over asm text). It is impressively well-lawyered
   (width-exact kill rules, label-first clearing, pinned regions —
   comments cite the miscompile each rule exists for, and I verified
   several while hardening my pass). But it is **superlinear on
   mega-functions**: seed 101's single 63k-asm-line function costs
   580 s at `-O1`, of which `CCC_NO_PEEPHOLE=1` removes 486 s (84%);
   `-O0` is 15 s. My attempt to memoize line classification made it
   *worse* (718 s — measured, reverted): the cost is solve volume
   (many passes × fixpoints), not classification. Senior-grade fixes:
   batch passes per solve, window the solves per block with
   conservative live-at-boundary unknowns, or move the hot analyses
   to the structured IR. Until then, big-TU `-O1+` compiles
   (kernel-style files included) pay a real tax and fuzzing depth is
   capped by it. Verdict: agree with the *correctness* engineering,
   disagree with shipping more passes on this substrate before the
   cost model is fixed.
3. **`cmov*` in the unconditional-write list** — a latent miscompile
   of exactly the class the file's other comments defend against;
   shows the value of premise-style comments going stale. Fixed (S10).
4. **Kill-switch discipline** (`CCC_NO_*`, 214 switches): I agree
   strongly — it made my -O1 bisection mechanical, even though the
   answer turned out to be "none of them" (always-on lowering). Keep.
5. **Probe-gated provisioning scripts** (`ensure_gcc_torture.sh`,
   `ensure_icx_2026.sh`, manifest+digest discipline): agree; my
   `ensure_yarpgen.sh` deliberately mirrors them. The ICX provisioner
   gave me a working ICX 2026.2.0 on this host first-try.
6. **`lccc-snapshot.sh` runs `git add -A`**: it silently committed
   22k lines of generated repros into my S07 snapshot (caught by
   patch-size inspection, redone as S08). For a tool whose whole job
   is preventing work loss, ingesting *untracked fuzz output* into
   the deliverable is the wrong default; it should add tracked
   modifications (`-u`) or refuse on a dirty untracked tree.
   Disagree; documented in the FOLLOWUP for the owner.
7. **Unmerged branch `arena/01478eae`** (segment spaces on pointers,
   `-fshort-wchar`, "default x86-64 baseline", reviewed at
   stat/subject level only): changing the *default* x86-64 baseline
   is a consequential perf choice that needs the same
   multi-compiler A/B evidence demanded elsewhere in this repo
   before merge; I neither endorse nor block it from this patch.

## Process risks recorded

* The harness restored/tore `redundant_ext.rs` twice mid-session
  (duplicated `#[test]`, lost attribute, spliced EOF). Discipline
  that worked: read-before-edit, count-asserted replacements,
  count tests before/after (1813→1814 caught the lost attribute),
  commit+snapshot per validated step.
* VM timing deltas under ~10% are meaningless without an
  ASM-identity check (a −7% crc32 "regression" was identical
  assembly). All perf numbers here are Cachegrind-Ir or A/B with
  census, never bare wall-clock.

## 4b. The audit-census ratchet was already red on main (verified)

On the pristine rebase base (2715bf90), `scripts/audit_census.py --check`
already fails its own committed baseline: panic_sites 857 vs 854,
panic_sites_hot 110 vs 107, fns_over_300 232 vs 231. At the commit that
last touched the baseline (24a002c, inside PR #871's range) the census
matches exactly, so the drift came from PR #871 commits after it — the
named new panic sites are the five `expect("non-empty")` in
src/frontend/sema/init_cursor.rs and the `position(..).unwrap()` at
src/frontend/preprocessor/utils.rs:245 (net +3 after removals
elsewhere). These look invariant-backed, but by AUD-ICE-1's letter
every panic-shaped site in frontend/lowering/driver is a candidate
user-input ICE and deserves `let-else` treatment by their owners.

This delta re-baselines exactly those upstream numbers plus one
knob_literal for CCC_NO_SEXT (documented at its read site and in the
README; knobs_undocumented unchanged). The delta's own function-length
contributions were engineered to zero instead: the sign-extension
consumer and the movq/cmov producers of redundant_ext were extracted
into helpers (eliminate_redundant_zero_extend 567 -> 494 lines,
self_update 353 -> 286), and fns_over_500 stays at the baseline 93.

Host note: this VM additionally needs GCC 16.2 installed (Debian sid
gcc-16 16.2.0-3) before dozens of ci_local gates can run at all —
several check scripts hard-require `gcc-16` as oracle. 32-bit PIE
binaries built by that toolchain segfault at address 0x1 on this
host (kernel 6.1.158+); -no-pie and -static are fine. That is a host
quirk to know when triaging i686 gate failures here, not an lccc bug.

Rebase note (onto 4c0d36489, PRs #872/#873): upstream's own cleanup
dropped the counts further (panic_sites 852, hot 105, fns_over_300
231) — credit to the other team for moving the right direction. The
baseline shipped here is TIGHTENED to the measured values on the
rebased tree (ratchets should ratchet), with CCC_NO_SEXT the only
metric this delta raises (knob_literals 510, documented).

## 4c. The ext-vector near-miss: an oracle-provenance lesson (verified)

During final gating, clang->lccc -m32 ext-vector legs segfaulted
(provider abi_bool4 storing through a wild sret pointer), the ABI
grid showed 156 failing legs INCLUDING clang:clang self-legs, and a
pristine origin/main build reproduced every failure identically.
Root cause was NOT lccc and NOT upstream's code: Debian's
clang-23 1:23.1.3-1 build differs on i386 from the official LLVM
23.1.3 release binary (scripts/ci_install_clang_oracle.sh,
sha-verified). With the official oracle exported via $CLANG, all
954 grid legs and every ext-vector gate pass. The other team's
exact-oracle pinning (tests/regression/lib/clang_oracle.sh) is
vindicated by this incident: "same version string" is not "same
compiler". Verdict: their design choice here is RIGHT, and the
episode is a standing warning against distro oracles. It also
exonerates the merged i386 ext-vector work (PR-era convention
change), which this audit initially suspected.

## 4d. PRs #872/#873 (merged during this work) -- assessment

#872 (builtin folding vs shadowed library names): sound direction
(resolve against the identifier visible at the call site), with
regression coverage for the shadowed/nested cases and conservative
bounds on the new strcmp/memcmp sign folds. It also REDUCED the
census panic counts, letting this delta tighten the baseline.
#873 (full GNU TLS model set + GNU2 descriptors): the conservative
bits are right (psABI-safe default model choice; the descriptor
call carried as an opaque pseudo through the textual peephole so
no pass can mistake it for a SysV call -- the same conditional-
effect hygiene class as the cmov defect). These are careful
landings; no red verdicts. The remaining disagree-with-upstream
items from 4b stand (ratchet discipline), softened by their own
subsequent cleanup.

## 4e. Rebase onto e176a77a (PRs #874-#877) -- semantic audit

Rebase was textually clean (file overlap only in ci.yml/ci_local.sh,
disjoint regions). Semantic audit findings:

1. Ratchet discipline STILL upstream's weak spot: pristine e176a77a
   measures panic_sites 853 / fns_over_300 232 against its own
   committed baseline of 854/231 (fn300 red by one, again). This
   delta's contribution to both remains zero (measured identically
   on pristine main); the baseline here is re-tightened to the
   measured values.
2. Pristine main also failed ci-gate-parity: the hosted
   check_tls_runtime_oracle.sh step had no ci_local mirror. Fixed
   here by adding the gate; upstream's own bidirectional checker
   is the right design -- merging past it is not.
3. Volatile ALU fusion (378f9844) was merged with "No compilation
   or runtime tests run; execution is deferred to CI" in its own
   commit message -- a process red flag, subsequently remedied by
   d8bed1b1's hardening (mutation control included). The follow-through
   is what should be judged; it is adequate.
4. Interaction with this delta's pass: phase 9 re-runs the text
   passes on build_result-flattened text where pin/volatile
   metadata no longer exists, while load_op_fuse now depends on
   that metadata. eliminate_redundant_zero_extend is safe there
   only because every rewrite requires register operands; a unit
   test now latches that contract (memory operands are never
   rewritten or reordered). The same question is worth asking of
   every text pass in the phase-9 rerun set.
5. Post-rebase behavior is bit-stable where measured: full lib
   suite 5374/0/10, the three delta regressions match GCC 16.2,
   base64 hot-function Cachegrind Ir unchanged (141.075M lccc vs
   72.024M GCC-16.2). Validation this round is surgical (targeted
   gates listed in the delivery notes); the full --fast run was
   last green on 4c0d36489 and is NOT re-claimed for e176a77a.
