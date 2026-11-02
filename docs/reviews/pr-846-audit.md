# PR #846 — critical audit and implementation handoff

**Current recommendation: do not merge yet.** The original audit findings below are preserved as the review record. Follow-up work has since addressed F1–F4 locally and added an ICX CI lane, but the final worktree is not yet pushed or hosted-CI validated; live ICX provisioning remains unverified. The current performance measurement is still slower than GCC. See the dated follow-up status at the end for exact evidence and open risks.

Review date: 2026-10-17. Scope: the complete PR delta, surrounding pass-entry guards and flag analysis, regression scripts, oracle dispatch/identity, provisioning, and CI/local wiring. This is a source-level audit plus compiler-free tests, **not** an executed miscompilation reproduction or an exhaustive compiler proof. The initial exact-application sanity check established patch integrity; this deeper audit found issues that check missed.

## Executive scorecard

| Dimension | Weight | Score / 10 | Reason |
|---|---:|---:|---|
| Correctness and semantic containment | 30% | 4 | Two paths admit memory effects the packed loop does not preserve. |
| Validation and negative controls | 25% | 6 | Useful parity/kill-switch gates, but weak rejection coverage and a demonstrated fail-open trace check. |
| Design and readability | 20% | 8 | Read-only matching, staged lowering, bounded proofs, explicit policy and readable invariants. |
| Oracle identity and provisioning | 15% | 8 | Improved to authenticated APT selection, frozen manifests, pre-publication validation and offline tests; live install still unverified. |
| Scope and maintainability | 10% | 6 | Several independent subsystems in one large PR; main CLI integration and evidence remain incomplete. |
| **Weighted total** | **100%** | **6.1** | **Rounded recommendation: 6/10, changes required.** |

Qualitatively: code structure/readability **8/10**; overall engineering/release readiness **5/10**. These are review judgments, not measured defect probabilities. A clean CI run cannot average away a silent-miscompile blocker. A 10/10 below means satisfying this review's acceptance criteria, not guaranteeing that no compiler defect exists.

### Quantified observations

- The imported change touched **21 files**, with **2,309 additions / 26 deletions**.
- The new generator module has **942 lines and zero direct Rust unit tests**.
- Its C fixture has **7 kernels: 6 intended positive cases and 1 short-trip negative case**. The gate checks an aggregate firing count, not the identity of every accepted/refused loop.
- The imported Rust additions include **6 unit tests**: 3 paired-counter tests, 2 flag-liveness tests, and 1 loop-bound boundary test. The 3 paired-counter tests cover more than 3 scenarios.
- This audit records **8 open items: 2 P1 correctness blockers, 5 P2 validation/integration items, and 1 P3 performance-evidence item**. Some are established defects; others are explicitly labeled risks or missing evidence below.
- Compiler-free checks run during this follow-up: **104 unittest cases passed, 1 real-ICX test skipped by explicit opt-in policy** (24 oracle/provisioning cases + 7 oracle-delta cases + 73 gate-parity cases). The cache self-test also passed, reporting 18 checks.
- CI/local gate parity passed: **250 standalone commands, 24 registered invocation contracts**. An independent lane-pack model produced exactly `[0, 1, ..., 31]`.
- On the imported commit, GitHub reported **13 successful checks and 1 intentionally skipped research check** at the final pre-push inspection. Those results do not validate the follow-up commit; its checks must be assessed separately.

## What I agree with—and why

1. **Separate generator recognition from load-based map vectorization.** Pure `dst[i] = F(i)` loops need not pretend to have a source stream. Restricting the first implementation to AVX2 and simple integer operations is a sensible scope boundary.
2. **Build a plan without mutating the function; lower into temporary structures; commit only after success.** This is the right failure-atomic architecture for an optimizer. It needs stricter legality checks, not a wholesale redesign.
3. **Keep the original scalar loop as the tail.** This avoids a second implementation of remainder semantics. The lane-offset layout and masking before saturating packs are internally consistent; the independent lane-order model agrees.
4. **Resolve target/kill-switch policy once in the driver.** This avoids per-loop environment reads. Target permission remains distinct from whether the build host supports the ISA.
5. **Use a guarded sibling to bound a testless counter.** The nonnegative, same-type, zero-seeded unit-step subset, bounded recursion, and exclusion of observable exiting latch values make the new theorem reasonably auditable. The range proof should not be casually generalized.
6. **Scan flag liveness through a standalone label.** A label is not an instruction or a transfer on the fall-through path. The implementation retains readers/control transfers as barriers and tracks partial flag writes. I found no blocker in this particular change; the positive and negative tests are appropriate.
7. **Keep local ICX opt-in and record actual identity.** Default PR checks should not silently depend on a heavyweight vendor installation or a moving remote channel label.

## Findings, ordered by importance

### F1 — P1, correctness blocker: unmatched stores are silently ignored

**Evidence:** `src/passes/vectorize_generator_map.rs:326–351` records only `Store { val: Value(...), volatile: false, ... }`. In the second body walk, line 367 accepts **every** `Store` with an empty arm. Lowering emits just the one recorded byte store.

An extra constant-valued store or a volatile store is not necessarily recorded or rejected. Thus “exactly one store” is not established. The generic volatile guard in `vectorize.rs:31786` does not rescue the general case: it tracks pointers rooted in volatile allocas and immediately returns false when none exist. Volatile accesses through a function parameter need no volatile alloca.

Concrete C shape for the implementation agent to turn into a regression:

```c
void generator(unsigned char *dst, volatile unsigned *tick) {
    for (unsigned i = 0; i < 32; ++i) {
        dst[i] = (unsigned char)(i * 3u);
        *tick = i;
    }
}
```

The corresponding valid IR body has one admitted byte-store chain and a second `Store { ptr: tick, val: iv, volatile: true }`. That second store passes the wildcard arm. A packed 32-element iteration must not replace 32 observable writes with zero. The scalar tail does not repair omitted effects in the packed prefix.

**Confidence:** high, from the matcher/lowering contract and the surrounding entry guard. No claim is made that this exact C fixture was compiled in this sandbox.

**Required fix:** inspect every Store. Reject all volatile/non-default-space stores and every store other than the unique selected byte store. Likewise do not blanket-accept an unmatched GEP. Match by instruction identity/index, not merely by opcode category. Preserve failure atomicity.

### F2 — P1, correctness blocker: segment-relative stores lose their address space

**Evidence:** the store pattern at `vectorize_generator_map.rs:328–334` ignores `seg_override`. The packed `Packuswb256` store at lines 798–803 has only an ordinary destination pointer and no FS/GS override.

The established `loop_touches_named_space` guard in `vectorize.rs` belongs to `vectorize_with_analysis_mode`. The generator transform is invoked **before** that analysis-mode call. Therefore that later guard cannot undo a generator rewrite already committed.

A byte generator whose single scalar store is `AddressSpace::SegFs` or `SegGs` can be admitted, then emitted as a default-space store to a different address.

**Confidence:** high, direct omission and call-order evidence. Not an executed segmented-memory repro.

**Required fix:** require `AddressSpace::Default` locally in the generator matcher before any mutation. Add separate FS and GS IR refusal cases plus default-space positive controls. Do not rely only on a broad outer guard.

### F3 — P2, established test defect: trace-firing checks can accept a failed command

**Evidence:** `tests/regression/check_generator_map.sh:45–48` uses `producer | grep -c ... || true`. The final `true` defeats `pipefail` for producer failures as well as the intended no-match case.

A compiler-free shell witness emitted six matching trace lines and returned 23. The helper reported `6`, and the gate accepted it. With the kill switch, a failed diagnostic invocation emitting no traces can also look like a successful zero-rewrite check. Separate runtime invocations do not validate the diagnostic invocation's exit status.

**Required fix:** capture the compiler's trace into a file and require its exit status to be zero first. Count matches afterward, allowing only grep's documented no-match status. Add mock-producer mutation tests covering success/zero, success/six, failure/zero, and failure/six.

### F4 — P2, coverage gap: positive parity is not a matcher-legality test

The generator has no direct Rust matcher tests, and its aggregate firing threshold does not prove that the short loop was rejected or that every intended kernel fired. There are no focused rejection tests for extra stores, address spaces, unsupported casts/shifts, or malformed CFG/value shapes.

For paired counters, runtime lower bounds generally cause the mirror proof to decline because the inferred sibling range includes negatives. Those edge cases are still useful regression inputs, but their passing checksums do not demonstrate that the new mirror theorem executed at those edges. The constant-threshold fixture is useful; merely requiring an assembly difference from the global IV-widen kill switch is less durable than a theorem-specific trace/IR assertion.

**Required fix:** add direct read-only matcher/transform tests, explicit per-case firing/refusal assertions, and a trace reason for the sibling-mirror proof. Unsupported cases must retain identical IR, not just return a zero rewrite count. Include memory canaries and nonzero sentinel state in executed CI fixtures.

### F5 — P2, functional integration gap: the main Godbolt CLI still routes local keys remotely

`godbolt.py:command_compile` and `command_compare` still call remote `compile_on_godbolt` for the requested compiler. Registering a name in `LOCAL_ORACLES` does not make `godbolt.py compile icx2026.2 ...` or `compare --oracles icx2026.2` use the local path. The new local path currently works in `codegen_oracle.py` and `oracle_profile.py` instead.

**Required fix:** either implement one shared local/remote dispatcher in all advertised entry points, including discoverability, or explicitly reject local keys in unsupported commands with a useful message and document the supported surface. Prefer the shared dispatcher. Mock remote APIs to raise in local-only tests; prove no remote call happens.

### F6 — P2, unclosed architecture contract: requested flags are not target validation

`codegen_oracle.py:_record_local_oracle` rejects `req.arch != 'x86'`, then forwards flags unchanged. It does not establish x86-64 bitness or validate target-changing flags such as `--target`, `-target`, or `-m32` against the channel's declared contract.

**Confidence:** high that validation is absent; whether a particular retargeting request succeeds depends on ICX. This is **not** a demonstrated wrong-target output. A vendor refusal would prevent that specific output, but should not be the tool's only architecture contract.

**Required fix:** define the contract explicitly. Either reject retargeting/32-bit flags for a host-x86-64-only channel, or record and validate the effective target/bitness. Add table-driven mock tests for accepted host flags and conflicting flags. Do not infer the effective target solely from the user-supplied architecture label.

### F7 — P3, missing performance evidence: SIMD structure is not a measured win

The implementation allocates two 32-byte scratch slots and performs intermediate pack stores/reloads. The legality threshold of one 32-element group is not a profitability model. The PR does not supply a controlled runtime/Callgrind A-B measurement showing where this tradeoff pays off, nor compiler-time measurements for repeated whole-function use scans.

**Required fix:** supply correctness-gated evidence for short, medium and large trips, aligned/misaligned destinations, representative expressions, and at least one adverse case. Report the exact metric, CPU/tool identity, variance where applicable, and regressions. Measure analysis cost on a large IR function; optimize repeated scans only if evidence warrants it. Do not claim universal speedups from a firing trace or instruction counts.

### F8 — P2, external validation prerequisite: the ICX upgrade has not been live-provisioned here

This sandbox's direct TLS access to Intel's APT host fails. No current Intel package filenames, build numbers or SHA-256 values were invented. The follow-up instead resolves the newest 2026.2.x compiler and matching support packages from authenticated APT metadata at installation time and freezes the exact selection in a manifest.

The selection, hashing and publication logic have offline tests. Those tests do **not** establish that Intel currently publishes the expected five-package coherent build, that the minimal extraction set has every runtime dependency on a clean supported host, or that real package layout matches expectations.

**Required fix:** on an allowed-network clean Linux host, run provisioning and the explicit identity probe without compiling any C/C++ code. Retain the selected manifest, APT signature-verification output, all five package identities/checksums, macro value, and a repeat-install result. Verify wrong-line and corrupted artifacts fail without publishing a prefix. If the package layout/dependency set changed, fix the resolver from observed vendor metadata, not guessed names.

## Changes made during this audit

These are targeted to the requested ICX upgrade; they do not fix or hide F1/F2.

- Replace the 2026.1.1 local channel with **`icx2026.2`**, recording the actual probed 2026.2.x patch release rather than pretending a release-line label is an exact compiler version.
- Replace hand-maintained package URLs/checksums with an **isolated authenticated APT client**. Select the latest candidate in the 2026.2 line, require the same exact build across the five compiler/support packages, verify archive digests and record exact provenance. No older-release fallback.
- Use a cross-process lock, private staging, validation **before** atomic publication, and cleanup on failure. Existing prefixes are frozen validated snapshots; explicit refresh uses a new prefix. No host APT sources, system keyrings or installed packages are modified.
- Bootstrap the Intel signing key through Intel HTTPS, as the vendor setup does. This is **not** an independently pinned fingerprint. That trust boundary is documented rather than overstated.
- Re-probe compiler identity rather than trusting a cached pathname, invalidate changed environment selections/symlinks, handle malformed provisioner output, and preserve actual identity in manifests.
- Remove the unnecessary Compiler Explorer catalogue request for local-only `codegen_oracle.py` batches.
- Make real ICX tests explicitly opt-in; a requested real probe can no longer silently turn a wrong compiler into a skip. Normal discovery has no provisioning/network side effects.
- Wire both offline oracle/provisioning test suites into GitHub CI and the local gate driver, and document usage/limitations.

## Implementation handoff: acceptance criteria for a 10/10 review

**Work on the existing Arena branch and PR. Do not change the branch, silently disable the transform, weaken gates, or regenerate guessed vendor pins. Local compilation and runtime validation are authorized; report hosted checks separately and do not treat local results as hosted CI evidence.**

### Work package A — close memory-effect legality first

1. Start with red Rust IR tests for a legal 32-element generator plus: an extra constant store; an extra volatile store via a pointer parameter; a second byte store; an unmatched GEP; FS and GS stores.
2. Assert refusal and byte-for-byte/debug-structural equality of the function before/after every rejected transform. Include a fully accepted default-space one-store control.
3. Change the matcher to account for **every** body instruction and require the unique selected nonvolatile default-space byte store. Reject all other effects. Never fix this by relying exclusively on the outer volatile-alloca guard.
4. Keep the read-only match / pure lowering / commit split. Explain the complete effect-preservation invariant in the module documentation.
5. Add C regressions for CI, with a 32-iteration no-tail case so the scalar tail cannot mask omitted effects. Use nonzero sentinels and check all relevant objects/canaries.

**Done when:** deleting either the unique-store check or the address-space check makes a dedicated test fail, and valid kernels still transform under AVX2.

### Work package B — make tests fail closed and non-vacuous

1. Fix trace capture to preserve producer failures; add the four mock-producer cases in F3. This can be tested without a compiler.
2. Keep kernels named/stable enough for per-kernel trace assertions, or add direct IR trace identities. Check all six positives and the short-trip refusal independently.
3. Add rejection coverage for 64-bit logical right shifts, counts outside 0–31, unsupported operations/loads/calls, duplicate definitions, escaping values and noncanonical control flow. Inspect Switch/IndirectBranch edges as well as ordinary branches when testing the CFG contract; do not claim a proven CFG bug without a valid-IR witness.
4. Add theorem-specific mirror diagnostics and assert firing on the constant-bound shape, refusal on unsupported/escaping shapes, and both candidate orders. Explain why runtime-bound edge fixtures may exercise refusal instead of the theorem.
5. Add tests for signed/unsigned domain edges and keep the existing strict-header/observable-exit constraints. Constant non-strict comparisons normalized to strict thresholds should be documented accurately; do not equate every source-level `>=` with an unsafe proof.

**Done when:** disabling the new optimization fails its dedicated firing test, enabling it for a forbidden shape fails a refusal test, and a failed compiler diagnostic command can never produce a green gate.

### Work package C — finish oracle dispatch and architecture contracts

1. Use a shared dispatcher or explicit supported-command validation for local compiler names. Cover `compile`, `compare`, codegen per-function/whole-TU/rank modes and profile verification, as applicable.
2. In every local-only mock test make all remote catalogue/metadata/compile methods raise on use.
3. Validate effective target/bitness or reject contradictory flags; add table-driven tests.
4. Preserve structured errors for timeout, missing executable, malformed stdout, failed macro probes and malformed provisioning manifests. Record actual versions, never a fabricated current-release label.

**Done when:** each supported local-only entry point succeeds with remote access disabled and rejects a wrong-release/wrong-target fixture predictably.

### Work package D — close vendor-installation evidence

1. Run F8's provisioning-only verification on a clean allowed-network host. No source compilation is required or requested.
2. Exercise concurrent same-prefix requests, failed extraction/probe, retry after failure, an unmanaged prefix, digest tampering and a frozen-prefix repeat run.
3. Retain a small signed-metadata/manifest evidence record externally under repository conventions; do not commit vendor archives/binaries.
4. Confirm all selected package versions actually exist and the CPU-only dependency/layout assumptions hold. Keep authentication enabled. Consider separately pinning/verifying the Intel key fingerprint if reproducible supply-chain policy requires it.

**Done when:** a genuine 2026.2.x macro agrees with authenticated package metadata, failed installs leave no published prefix, and repeat measurements resolve the same frozen identity.

### Work package E — justify profitability and merge readiness

1. Produce correctness-gated A-B evidence for F7; state limitations and adverse cases. Keep performance work separate from semantic fixes.
2. Obtain green required GitHub checks on the **final** head, including the new adversarial fixtures and offline gate tests. A green earlier commit is not sufficient.
3. Re-audit the final diff for dropped effects, target/address-space drift, false-positive proof firing and fail-open checks. Verify mutation tests actually detect removed guards.
4. Update this report and the PR body with resolved findings and evidence links. Prefer small, separately reviewable commits for legality, gate hardening, dispatch and evidence.

**Merge decision:** fix F1/F2 and their negative tests first; close F3/F4 and verify the real ICX upgrade before treating the PR as ready. F5/F6 need a deliberate supported-surface/target decision. Performance claims must remain qualified until F7 is evidenced. Do not merge the current branch merely because its existing corpus checks are green.

## Follow-up status — 2026-10-17 (initial fixes pushed; final validation in progress)

The following updates supersede the original open/required-fix descriptions where stated; the sections above remain the historical audit record.

- **F1/F2 addressed locally:** the matcher now scans/account for body effects, matches the selected store and GEP by instruction identity, requires a nonvolatile default-address-space store, checks CFG edges through shared CFG construction, and rejects unsupported side effects/shapes. Direct Rust tests cover additional value/constant/volatile stores, FS and GS, unmatched GEPs, escaping/duplicate definitions, unsupported effects, malformed control flow, tails, and failure-atomic rejection. Focused generator-map Rust tests passed **21/21**; the compiler-built runtime/trace gate passed six named positives, the kill-switch arm, GCC output parity, and refusal checks for `gen_g`, `gen_volatile`, and `gen_constant`.
- **F3/F4 addressed locally:** trace capture checks compiler exit status before counting; six mutation tests passed. Per-kernel firing/refusal assertions and a theorem-specific paired-counter trace are wired into regression gates. The paired-counter gate passed all optimization-level and kill-switch cases, including proof firing on the constant-threshold shape.
- **Pack-register follow-up:** made the 256-bit signed-word pack a pure vector SSA value and admitted the final `Packuswb256` sink as a legal homed consumer. On the exact same local benchmark harness, 3-round/1,000-repetition medians changed from 0.04576s to 0.04269s for LCCC, with generator instructions 92→89, memory loads 10→8, stores 8→6 and stack references 12→8. GCC measured 0.03058s in the after run. This short local sample is not a robust performance win claim; LCCC remains slower. Full baseline/optimized measurements and variance review remain required.
- **Oracle tests:** the local oracle/provisioning unittest suite passed **24 tests, 2 skipped**. It does not prove real Intel package availability or a successful live install. The hosted ICX job is added but has not run on the final head.
- **Local CI result:** `scripts/ci_local.sh --fast` ended **241 passed, 31 failed, 13 skipped** before the latest small edits. Multiple failures are confirmed environment limitations (missing i386 libc/libgcc, unavailable qemu/binutils downloads under sandbox network restrictions). Some targeted 32-bit checks have a valid test prerequisite failure rather than a demonstrated source regression. The artifact-DAG layout test did fail because the new `icx-oracle` job was not included in its expected lane set; the test has since been updated and passed in isolation. The whole local suite has not yet been rerun after that correction.
- **Hosted checks on initial fix commit `602adb9`:** unit/build, Clippy, MSRV, assertion-enabled tests/corpus, Debian GCC 16.2/Rust 1.99 corpus/contracts, frontend/ABI, SSA, codegen/runtime, complete benchmark corpus, and authenticated ICX integration all passed. The assembler/linker lane failed in its remaining fast local-contract Python unittest suite; follow-up diagnosis isolated the failure to the newly-added CI-layout test expecting `LCCC_TEST_REAL_ICX` in the `run` text rather than the step's `env` map. The assertion was corrected in `01d51d9` and the 759-case unittest suite passed locally except for that now-fixed assertion. Hosted validation on the corrected head is running; no final-head green claim is made yet.
- **Still open:** final green hosted checks on the current pushed SHA, longer final benchmark with adequate samples/variance review, final diff review, and any remaining target-specific failures. Authenticated live ICX provisioning, frozen-prefix repeat, macro/target probe and actual compiler test **passed in the hosted ICX job on `602adb9`** (29 seconds); the job's provenance artifact was retained by Actions. The current final head still requires its own green check. No merge recommendation upgrade is made.
