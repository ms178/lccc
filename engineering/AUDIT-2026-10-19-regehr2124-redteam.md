# AUDIT 2026-10-19: red-team review of the REGEHR-2124 patch stack

Scope: the whole REGEHR-2124 change set on top of the then-current
`main` (`2715bf90`, the merge of PR #871). Stated as a base commit, not
as a commit range: the change landed squashed, so the individual stages
never existed as standalone commits in this repository. Method: re-read every
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
  count tests before/after (one lost `#[test]` attribute shows up
  immediately as a count drop),
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

## 5. Round 2 -- the implicit-operand model had three copies, and one
##    of them was not consulted at all

The first round fixed `cltd` in `helpers::has_implicit_reg_usage` and added a
private `implicit_write_families` table for the redundant-extension pass.
Re-auditing that work against the tree found the shape of the problem was
different from the shape of the patch:

* **`types::classify_implicit_operands` is the real model.** It is 200 lines,
  covers ~90 mnemonics and forms (one-operand `imul`, `mulx`, `popa`/`pushf`,
  `rsm`, `sysenter`, the `xsave` family, `rep` string ops, `cpuid`, `rdtsc`,
  `int3`, hypervisor calls, ...), and is pinned by `tests/fixtures/
  implicit_operands.tsv` plus `scripts/check_implicit_operand_matrix.py`.
  The table the first round added was a 20-line subset of it.
* **`redundant_ext` had no implicit-write invalidation at all.** The PR-tip
  file handled `call` and nothing else, so a fact about `%rax`/`%rdx`
  survived `cltd`, `idiv`, `cpuid`, `rep movsb` and `syscall`. The private
  table was the first round's fix for that; the correct fix is to consume
  the shared model, which is what the pass now does
  (`implicit_r_w_full(..).1` -> `ExtFacts::clear_implicit(mask)`).

Measured consequences, all reproducible:

| claim | evidence |
|---|---|
| the private table was unsound for forms the model knows | mutation restoring the old four-mnemonic table makes `implicit_writes_reset_the_ledger_for_every_modelled_opcode` fail on `cqo` (fact for family 2 survived) |
| the refactor changes no emitted code | `lccc -O2 -S` over all 1000 corpus programs: **0 differing `.s` files** vs the pre-refactor binary |
| the fact ledger cannot depend on a value the instruction destroys | property test asserts, for every family in the model's write mask, `ledger(start=all-true) == ledger(start=all-false)`; 38 sample forms, non-vacuity floor of 30 |
| the veto is a superset of the model's write half | `every_modelled_implicit_write_is_vetoed`; this **passes at HEAD already**, because the tail of `has_implicit_reg_usage` consults `has_bare_implicit_gp_effect` -> `implicit_write_refs`. The first round's `cltd` entry in that union was therefore redundant |

The last row is the useful correction. Making `has_implicit_reg_usage` consult
the model *first* — the obvious "single source of truth" move — measured
**+5.1 % compiler wall time** (corpus `-S` sweep: 45.5 s mean vs 43.3 s mean,
interleaved runs) for a semantic change of zero, because the model was already
consulted on the slow path. It was reverted; the only change left in
`helpers.rs` is a `trim_start()` that makes the veto total (an untrimmed line
used to model as an empty mnemonic and answer "safe"), plus the contract tests.

`scripts/check_implicit_operand_matrix.py` had never run in CI: it extracted
`implicit_full_write_refs` without the `_from_write_mask` helper it calls, so
the probe did not compile, and no workflow referenced it. Fixed and wired into
both `.github/workflows/ci.yml` and `scripts/ci_local.sh` (parity now 315
commands).
