# PR835 review audit — S72 update, 2026-10-16

This is a dated investigation and local review record, not a hosted-CI acceptance
certificate. The earlier sections preserve the 2026-10-15 deep-audit findings;
section 7 records the fresh post-rebase evidence against upstream main at
`19be25d3856525974ec3032d0b16c7160e678331`. Historical performance numbers in
sections 2–5 are not substituted for the fresh S72 comparison. The tracked audit
update is included in the exact-tree FAST validation (**267 passed, 0 failed, 6
skipped**); final patch and exact-history proof are recorded in the S72 delivery
artifacts. Full/slow local CI is not authorized, and no hosted CI result is
claimed.

## Hosted baseline

All 13 checks reported for PR835 head
`4216f897460be1eef08cc0f24e8b6016cd738283` completed successfully before source
changes began. Main CI: https://github.com/ms178/lccc/actions/runs/38763960601 .
This includes the aggregate Test Suite, full benchmark corpus, codegen/runtime,
assembler/linker, frontend/ABI, SSA, assertion-enabled tests and toolchain jobs.
These results certify the uploaded baseline, not the subsequent local commits.

The preceding local delivery `3af371078042c941e07cd8018a21e2a6d7edbf05`
and uploaded PR differ only in ANSI escaping in one vendor evidence JSON.
Compiler sources, build definitions, scripts, tests and workflows are identical.
Consequently, the review's whole-tree provenance criticism is legitimate, but
claiming the compiler receipts describe different compiler code is not supported.

## 1. Explicit policy ownership

Replaced the crate-private `SIGNED_OVERFLOW_WRAPS` and
`FP_ENVIRONMENT_OBSERVABLE` thread locals with `IrFunction.pass_policy`.
`run_passes` installs the independent driver flags before any O0 early return;
cloning retains them and switch outlining inherits them explicitly. Backend DCE
now consults the function rather than the last TU on its worker thread.

No deprecated TLS shim is needed: these were crate-private implementation
interfaces and every use was migrated. Retaining a shim would retain two
competing sources of truth. This does not claim that unrelated target/pass
configuration elsewhere in the compiler has been made context-owned.

Executable tests cover all four policy pairs at each of O0..O3, a second TU on
the same thread, preservation of the first TU, and a cross-thread handoff.
`check_pass_policy_mutants.py` compiles and runs the original pure constructor
and two independent false-setter mutants. Both mutants must compile successfully
and fail the executable assertion; syntax/build failure is not accepted as a
mutation kill. This complements, rather than substitutes for, pipeline tests.

Initial targeted validation: 28 policy-filtered Rust tests, 61 IV-widening tests,
and the observable-FENV backedge-PRE test passed in fastbuild (Rust O1, jobs=2).
The original constructor passed and both independent setter mutants were killed.

## 2. Measured FENV conservatism (baseline, not an optimization result)

A mixed live exact-addition/dead unsigned-recurrence probe, executed under all
four rounding modes, produced identical checked output. Frozen PR compiler
Callgrind Ir: FENV off 4,155,651; on 13,155,665 (3.166×). GCC 16.2 produced
2,662,822 for both; Clang 23.1.3 produced 475,275 off / 1,412,849 on. GCC does
not implement the pragma like Clang; these are each compiler's observed policy,
not a claim of equivalent FENV support. The probe measures removable integer
work, not rounding correctness. Real inexact/exception/trap tests remain needed.

## 3. Local FENV optimization and missed interprocedural defects

DCE now roots FP-effect instructions and follows their operand dependencies,
while allowing unrelated dead integer computations to disappear. Constant
folding uses a positive integer-operation whitelist in mixed FENV functions;
FP loads, arithmetic, comparisons and both conversion directions remain runtime
operations. SCCP independently needed the same policy: its shared arithmetic
oracles previously folded `1.0 + 2^-53` to nearest even when runtime rounding
was observable. Its FP lattice values are now overdefined; integer propagation
continues normally.

The new native witness also found two omissions in the original review's
praise of fail-closed FP fencing. IPCP treated a void callee containing unused
FP arithmetic as an empty side-effect-free call, and its constant-call
interpreter evaluated FP despite being documented as integer-only. Purity
inference likewise ignored FP state. Those paths now respect function policy;
known FP call attributes are invalidated before purity inference.

Frozen PR835 passes the new witness at O0/O1 but fails at O2/O3 (exit 2: lost
FE_DIVBYZERO). An intermediate candidate preserved the five tested exception
classes but failed upward rounding of constant addition (exit 14); the SCCP
fix closes that independently observed hole. Final candidate and Clang23.1.3
pass all O0..O3 cases: divide-by-zero, invalid division, overflow, underflow,
signaling-NaN comparison and dynamic/constant addition in all four modes.

Validated fastbuild O1/j2: 3 cross-pass FENV tests, 28 DCE tests, 32 folding tests,
3 purity tests and 39 SCCP tests. IPCP has no tests matching its own module filter;
its actual entry point is executed by the cross-pass FENV tests (not a claimed
zero-test pass). Prior native witnesses remain 94/94; nine packed-control arms
each pass 1,260 bitwise cases plus Memcheck. All 45 corpus Callgrind Ir totals
are exactly unchanged relative to frozen PR835. These bounded checks do not
establish universal absence of regressions.

The paired mixed-work run now measures 13,155,655 -> 4,155,639 Ir with FENV on
(68.41% fewer simulated instructions); both FENV-off arms are 4,155,641 Ir.
The tiny startup-level difference is not a hardware speed claim. Earlier
absolute totals differ slightly with the harness paths/environment; compare
within the recorded paired invocation, not across unrelated runs.

## 4. Recovering the sieve regression without unsafe widening

The sieve's final byte-count loop was excluded by three legitimate contracts:
nonzero IV start, inclusive comparison, and a select-based conditional increment.
A separate transactional adapter now proves a finite nonnegative 32/64-bit
trajectory including the failing header, rebases byte-stream pointers, and
normalizes the conditional update. It commits its private IR only if the existing
zero-based packed analyzer and emitter both accept. Runtime starts, truncating
indices, escaping IV/cast values, wrong branch polarity, wrapping endpoints,
unsupported effects and failed packed plans retain the original IR and IDs.
The narrow inner marking loop is not made unsound to recover the benchmark.

Executable scalar-IR interpretation covers the actual rewritten CFG, signed
and unsigned 32-bit address casts, true/reversed conditional increments,
non-Boolean bytes, wrapping counters, starts and vector/tail boundaries. Three
Rust tests pass, including transactional refusals. Seven native/vendor/ISA/
kill-switch arms pass with tight heap allocations and Memcheck; the normalized
large-count function contains packed SAD while its disabled counterpart does
not. Baseline ISA and no-AVX arms contain no AVX2 SAD. Prior witnesses stay 94/94.

In the complete 45-case paired Callgrind corpus, 44 Ir totals are exact ties and
sieve falls from 1,816,416 to 1,257,407 (ratio 0.69224616, 30.78% fewer). Geometric
ratio versus PR835 is 0.99185968. Sieve is now 0.77442991 versus main (22.56%
fewer Ir, reversing the previously reported +11.87%) and 0.72962527 versus
GCC16.2 (27.04% fewer). Disabling only `byte_count_normalize` makes candidate
and PR835 an exact tie at 1,816,753 Ir in that shared-environment control (the
extra environment setting changes startup totals relative to the main pair).
These are bounded simulated instruction counts,
not hardware timing or proof that every remaining main-relative regression
has been eliminated.

The separate compiler-work probe after the FENV/policy changes measured median
Ir ratios 0.99839932 (TLS), 0.99850382 (sieve), 1.00023505 (byte reassembly), each
three rounds. The small positive compiler-work change is disclosed, not hidden
behind target-code aggregate neutrality or called a wall-clock regression.

## 5. Scheduling, decomposition and evidence classification

`loop_bound` now owns its scheduling predicate independently of the signed
`affine_fold` knob. Its legality remains target-independent; the x86-64,
O2/O3, non-size, first-iteration rollout is a measured profitability boundary,
not an ISA proof requirement. Dedicated disable control remains. Six tests pass:
actual CFG interpretation covers all eight strict/inclusive/swapped/reversed
spellings, every unsigned width, both wrap policies, nonidentity block labels,
zero-trip cases, near-maximum carriers and cyclic refusals. The interpreter
compares body and final-header trajectories and does not reuse the proof helper.
Native witnesses remain 94/94.

Strict packed DAG planning and its seven executable unit tests now live in
`vectorize_strict_recip.rs`. Immutable input plus an owned complete plan retains
transactionality and identifier rollback. Native register-home/ISA/guard-page
controls and nine 1,260-case bitwise arms with Memcheck pass after extraction.

The uploaded evidence delta is 118 additions plus one modified README. Seven
identical redundancies are represented by authenticated canonical aliases.
The follow-up inventory covers 128 actual files; scoped discovery rejects new
unlisted files, removed entries, reintroduced aliases, corrupt bytes and unsafe
paths. Seven Python integrity tests pass. All three inventories are checked by
the existing required local/hosted integrity gate. Native FENV O0–O3 and byte-count
boundary/ISA/disable controls are now registered on both sides of CI.

Textual contracts establish source/opcode shape, not execution correctness.
IR interpretation tests rewrite semantics on bounded trajectories; native
vendor comparisons test compiled execution; Memcheck checks memory accesses;
Callgrind measures simulated work; hashing authenticates retained bytes. None
alone is a universal correctness or performance proof.

## 6. Hosted research ownership and remaining dispatch block

Operational owner: repository maintainer `ms178` (the repository owner, not a
claim that a new assignment/issue was created). Research jobs now allow 45
minutes with bounded build/measurement steps and retained failure reports;
sieve joins the focused Callgrind set. A timeout or external-oracle outage must
be escalated with the job URL, compiler identities and partial artifact to the
maintainer before enlarging corpora or declaring success. Do not refresh a
baseline from a failed/partial run. A 30-to-45-minute allowance is mitigation,
not evidence of a successful dispatch.

Manual dispatch is still blocked here: no authenticated GitHub client/token is
available and Arena reports its GitHub connector unsupported. No remote run URL
or issue is invented. After publishing the reviewed branch, the maintainer can
run `gh workflow run compiler-research.yml --repo ms178/lccc --ref BRANCH -f baseline_ref=main`
and `gh workflow run oracle-delta.yml --repo ms178/lccc --ref BRANCH`, then record
the resulting run URLs and final conclusions. These two outstanding research
runs are distinct from the baseline's already-green 13 hosted checks.

## 7. S72 refreshed-main validation and measured results (2026-10-16)

### Base refresh and acceptance scope

A fresh `git fetch origin main` on 2026-10-16 returned the same commit,
`19be25d3856525974ec3032d0b16c7160e678331`; no additional rebase was needed.
The 17 PR commits replayed onto that base without conflicts. Four small follow-up
commits repaired evidence/test contracts found by post-rebase gates; they do not
change the pointer conversion policy. An exact-history recovery checkpoint was
saved before the refresh.

The final local FAST validation after this audit update passed **267 tests, 0
failures, 6 intentionally skipped slow gates**. Rustfmt, MSRV check and strict
Clippy passed. This is the required `--fast` scope, not a full/slow CI run. The
exact tree/stamp and log are retained with the S72 delivery artifacts. The FAST
run is a local check on Debian 13; it is not a hosted Ubuntu or GitHub result.

Targeted post-rebase checks also passed: PR824 cross-target boundary gate **369
PASS / 0 FAIL**, affine-loop-fold gate, signed-address harness tests **2/2**,
evidence-integrity tests **13/13**, candidate pointer Memcheck **90/90**, and
latest-main pointer-i128 Memcheck **30/30**. The six skipped FAST gates remain
for hosted CI; no GitHub workflow dispatch or push is claimed.

### Fresh performance evidence against latest main

For the default Sieve (`N=10,000,000`), all four native arms—rebased candidate,
`19be25d` LCCC main, GCC 16.2.0 and Clang 23.1.3—produced exactly
`primes up to 10000000: 664579`. They used `-O2 -march=x86-64-v3 -fwrapv`; native
measurements used two warm-ups followed by **31 randomized paired rounds**,
pinned to CPU 0 on a two-vCPU KVM Xeon VM.

- Candidate median: **46.618 ms**; latest-main LCCC median: **52.103 ms**.
- Paired candidate/main median ratio: **0.898258**, 95% paired-bootstrap
  interval **[0.891457, 0.908052]** (about **10.2% lower elapsed time** in this
  run).
- Candidate/GCC16 ratio: **0.945054** [0.929180, 0.951443]. Candidate/Clang23
  ratio: **1.019600** [1.016753, 1.024835]; candidate is about 2% slower than
  Clang in this workload.
- The VM's absolute per-compiler timing CVs were high (candidate 13.6%, main
  12.8%); outliers were retained and the paired-ratio CV was 4.7%. `perf` is
  absent, so there are no PMU counters.

Correctness-gated Callgrind on the same default input recorded **119,009,169
Ir** for the candidate vs **174,946,334 Ir** for main (ratio **0.680261**). This
is 32.0% fewer *simulated instructions*, not cycles or hardware speed. A bounded
six-workload Callgrind screen found exact Ir ties in `arith_loop`, `tce_sum`,
`zlib_ng_adler32`, and `sha256_transform`; `loop_patterns` differed by seven Ir;
Sieve was the only material change. The focused `affine_countdown` workload was
an exact tie at 95,730,281 Ir each. The 45-case S68/S69 corpus numbers in
section 4 are historical comparisons against other bases and are not a fresh
45-case sweep against `19be25d`.

The measured Sieve candidate's final byte-count loop uses AVX2 vector operations
(`vpcmpeqb`, `vpsadbw`, `vpaddq`); the latest-main baseline's loop is scalar.
This disassembly supports code-shape attribution, not timing or cycle claims.
Compiler Explorer returned GCC 16.2, Clang 23.1.0 and ICX's moving `cicxlatest`
channel under the same flags. The API reported ICX version only as `(latest)`,
with no numeric release. Static function instruction counts were 85 for local
LCCC, 36 for GCC16.2, 170 for Clang23.1 and 183 for `cicxlatest`; these counts
are assembly inventory only. A local ICX compiler/runtime was unavailable.

The raw native report, Callgrind manifests, Godbolt manifest, FAST log and
exact-history recovery proof are retained under `/home/user/artifacts/pr835/`.
The consolidated audit is `/home/user/artifacts/pr835/S72-audit.md`.

### Remaining boundaries

This S72 run demonstrates a scoped runtime improvement over rebased LCCC main on
one deterministic Sieve benchmark in a noisy VM. It does not establish an
application-wide speedup, superiority to every vendor compiler, hardware-counter
improvement, or that every historical main-relative regression is resolved.
The current-main Callgrind screen is six workloads plus focused Sieve and affine
runs, not the full 45-case corpus. Full/slow local CI is explicitly not run.
Hosted PR CI and the two manual research workflows still require a maintainer to
dispatch them; this environment has not produced a hosted result.

## Open work and delivery boundary

- Preserve the exact-history checkpoint before any future rebase. The pre-refresh
  recovery proof passed; final patch/recovery identities are regenerated after
  this document's exact-tree FAST pass.
- Keep the original reviewable commit history; no broad squash/rewrite was made.
- The remaining external acceptance is GitHub CI and the manual research workflow
  dispatch by a maintainer. No push, authenticated dispatch, or hosted pass is
  performed or implied here.
- Do not generalize the one-workload timing or Callgrind Ir result. Do not claim
  all performance regressions fixed; the fresh current-main screen is bounded.

Callgrind Ir is not hardware cycles, IPC, uops or EPYC emulation. Aggregate
neutrality does not erase individual regressions. No universal superiority or
perfect-score claim is justified.
