# PR #824: critical audit and implementation handoff

Audit date: 2026-10-14. Reviewed compiler-change commit: `4363e3efcc34888a0f4c35774fcb729bbff81b1a`.

> **Empirical follow-up:** The initial source-only audit below is historical.
> [PR824-EMPIRICAL.md](PR824-EMPIRICAL.md) records the executed baseline failures,
> safety repairs, Rust 1.99 tests, 45-workload execution, exact-version hosted
> Debian oracle, controlled static/scaling comparison, and remaining limitations.
> The later user instruction superseded the original no-local-build restriction.
> F1–F8 have implementation/test repairs; F9 remains partial; F10 helper mocks
> now cover both privilege paths and swap failures, with hosted i686 still required;
> F11's contradictory narrative is corrected. Read current-head CI, not this
> historical snapshot, for the latest checks. The original ratings below were
> source-review judgments, not new empirical measurements.

## Decision

**Do not merge the compiler changes as currently implemented.** The optimization direction is worthwhile, but source tracing identifies five high-priority safety-contract defects. Several are already expressed by regression tests added in this PR, without the corresponding implementation satisfying them. A successful lint, MSRV, or oracle-delta job does not discharge these contracts.

No compiler, assembler, Rust build, or compiler-dependent regression test was executed locally during this audit. Findings below distinguish source-proven behavior from unmeasured production reachability. In particular, I am not claiming to have reproduced a C-level miscompile or to have observed the predicted Rust test failures in CI.

The earlier integration sanity check was substantially narrower than this audit. Zero-fuzz application, result-blob fidelity, and syntax checks establish integration integrity, **not semantic correctness**. Its conclusions must not be read as approval to merge.

## Scope and quantification

The reviewed compiler-change commit contains:

| Measure | Value |
| --- | ---: |
| Changed files | 34 |
| Added/deleted lines | 10,438 / 1,922 |
| Net added lines | 8,516 |
| Changed Rust files | 13 |
| Rust added/deleted lines | 9,094 / 1,841 |
| Added Rust `#[test]` attributes | 84 |
| Added vectorizer test attributes | 41 |
| Vectorizer added/deleted lines | 6,101 / 1,712 |
| Final vectorizer file length | 34,190 lines |
| Final IVSR file length | 5,077 lines |

The vectorizer accounts for approximately 63.2% of all changed lines. Test attributes are an inventory, not a pass count or a coverage percentage.

**Findings: 11 actionable items — 5 P1, 5 P2, 1 P3.** P1 means fix before merge because a new pass violates a safety contract. P2 means substantive coverage, reliability, or maintainability work. P3 means documentation accuracy. No P0 production incident is claimed.

### Rating

These are judgment-based engineering scores, not measured probabilities of correctness.

| Dimension | Weight | Score / 10 | Reason |
| --- | ---: | ---: | --- |
| Correctness and ISA-contract fidelity | 35% | 3.5 | Several source-proven peephole hazards; strong mid-end preflight direction |
| Tests and evidence quality | 25% | 5.0 | 84 added test attributes, but contradictory and vacuous assertions remain |
| Maintainability and change isolation | 20% | 6.0 | Shared legality helpers improve structure; large monolithic transforms and repeated scans remain |
| CI and operational engineering | 10% | 7.0 | Better provisioning and diagnostics, duplicated provisioning and insufficient helper-specific tests |
| Documentation and reviewability | 10% | 6.0 | Extensive rationale, but stale ISA claims and performance evidence need qualification |
| **Weighted overall** | **100%** | **5.0** | Rounded from 4.975 |

**Code quality: 4.5/10. Engineering quality: 5.5/10.** The overall grade is not an average of those two headline assessments. It uses the weighted rubric above. Fixing the high-priority defects alone is necessary, not sufficient, for a 10/10 assessment.

## Choices I support

1. **Preflight before mutation.** Accumulator-seed validation, exact exit-comparison identities, remainder-reference planning, and unsupported multi-reduction rejection are the right architectural choices. They reduce partial-rewrite and SSA-dominance failure modes.
2. **Explicit mathematical domains.** Preserving signedness, narrowing constraints, positive floating zero, and bounds in widened arithmetic is much better than treating signed constant carriers as mathematical values.
3. **Complete address-use discovery.** Combining vector-memory triples with GEP uses, retaining vector displacement/store argument layout, and symbol-aware base identity addresses a real representation gap. The separate unsigned affine-add and product no-wrap obligations are important.
4. **Target-specific profitability.** Restricting the SIB cost rule to x86 rather than treating x86 addressing economics as a universal target property is correct. Keeping correctness and profitability separate is especially valuable.
5. **Conservative XMM destination-read handling.** Refusing propagation through destructive legacy shuffles is sound. A destination operand can be both read and written; text substitution alone cannot model this.
6. **Explicit oracles and evidence.** GCC selection, matching builtin include directories, full-state oracle comparison, retained optimization kill-switches, and surfaced check diagnostics improve engineering discipline.

These are good choices, but they do not excuse inconsistent safety handling elsewhere in the same change.

## Findings and exact follow-up instructions

### F1 — P1: dead-FP elimination does not preserve observable VEX upper-lane zeroing

**Locations:** `src/backend/x86/codegen/peephole/passes/dead_code.rs:294-311`; `fp_liveness.rs:1233-1257` in the same directory.

The new pass deletes a move using `FpLiveness::xmm_dead_after`. That oracle has one bit per vector register family. A legacy XMM full definition kills that bit, even though it does not overwrite the upper YMM lanes. The new deletion consumer needs a stronger proof than low-128-bit death.

Counterexample already present in the PR's test fixture:

```asm
vmovapd %xmm6, %xmm7
pshufd $0x1b, %xmm5, %xmm7
vmovupd %ymm7, (%rdi)
```

The first instruction zeros YMM7's upper 128 bits. The legacy shuffle replaces the low 128 bits but preserves the upper bits. The final store observes all 256 bits. The oracle kills the family at the shuffle; the new pass can consequently delete an observable zeroing operation. The fixture also includes legacy `pshufhw`, `movapd`, and `pxor` overwrites.

**Implementation:** Model low and upper-lane facts separately, or initially refuse deletion of VEX/EVEX XMM moves whenever a same-family wider observation can survive a legacy overwrite. A whole-function conservative veto is acceptable as an immediate correctness repair, but document its cost. Do not fix this by claiming every legacy XMM write kills the whole vector register. Treat upper-lane and mask semantics explicitly.

**Acceptance:** In GitHub CI, all four existing overwrite cases retain the VEX zeroing move. Add a positive twin where a later full-width VEX definition genuinely kills all relevant state, and branch/backedge cases. Store the full vector so the result is observable. Test the pass directly and the full pipeline.

### F2 — P1: symbol-LEA CSE treats the assembler location counter as a stable symbol

**Locations:** `relay_and_lea.rs:1888-1902`, `1195-1263`.

`parse_rip_symbol_lea` trusts `parse_disp`'s symbol classification. `is_plain_symbol` accepts `.`; consequently `.`, `.+8`, and `.-8` all pass. The assembler location counter changes at each instruction. Two identical textual expressions are not the same address.

```asm
leaq .(%rip), %r9
movq %r9, (%rdi)
leaq .(%rip), %r9
movq %r9, (%rsi)
```

CSE must not delete the second materialization. The added current-location regression expects rejection, but the implementation has no such exclusion.

**Implementation:** Positively recognize stable relocatable symbol expressions and explicitly reject the location-counter symbol. Keep ordinary local names such as `.LC0` legal. Also retain rejection of numeric RIP displacements and numeric-label relative spellings unless their stability is independently proven. Review the older RIP-relative LEA-folding parser for the same distinction; that is a pre-existing adjacent risk, not another counted PR defect.

**Acceptance:** The three current-location cases report no change and retain both LEAs. Ordinary symbol plus/minus constant cases still fold. Add decimal/hex and relative-label negative cases. Preserve pinned behavior separately; it is not a parser fix.

### F3 — P1: symbol-LEA CSE can rewrite a pinned instruction and discard its pin

**Locations:** `relay_and_lea.rs:1924-1953`; `types.rs:243-270`, `1176-1204`.

`cse_symbol_lea` checks `is_barrier`, but `is_barrier` does **not** include `pinned`. If a duplicate symbol LEA targets another register and is pinned, `replace_line` changes it to a move and reconstructs its metadata without the pin. The added pinned-materialization test directly covers this exact case. The same-register case instead calls `mark_nop`, which refuses deletion, while the pass still claims a change.

**Implementation:** Treat a pinned line as an explicit boundary in this pass: clear tracked homes and do not edit the line. Audit every new multi-line fold for pre-commit `all_editable` checks. Do not globally change `is_barrier` without reviewing all callers; the project's existing API separates metadata pinning from structural barriers.

**Acceptance:** Pin both same-register and different-register duplicates. Require unchanged text, unchanged pin/volatile metadata, and a false change result when no other rewrite happens. Add a pinned intervening line case. Never weaken or remove the existing test.

### F4 — P1: symbol-LEA homes survive writes to the first operand of `xchg`

**Locations:** `relay_and_lea.rs:1960-1967`; `helpers.rs:478-499`; `types.rs:classify_implicit_operands`.

The pass promises invalidation at any home write, but delegates to `writes_family`. That helper sees the trailing explicit destination plus fixed implicit writes. `xchg` writes both explicit operands; its first operand is neither covered by the trailing-destination rule nor by the implicit-write table.

```asm
leaq bins(%rip), %rdi
xchgq %rdi, %rsi
leaq bins(%rip), %rdi
movq %rdi, (%rdx)
```

The home for `bins` incorrectly remains `%rdi`, so the second LEA is considered redundant even though `%rdi` now holds the former `%rsi` value. This is not exclusively an inline-assembly concern: `memory.rs:3038` emits an unpinned `xchgq %rdi, %rsi` for parallel-copy staging. A complete C-level reproduction combining that emission and CSE has not been executed here.

**Implementation:** As an immediate repair, clear all CSE homes at instructions with opaque/multiple register effects (`has_implicit_reg_usage` already includes `xchg`). For the long-term model, use a conservative complete register-write set that includes both `xchg` outputs and every explicit destination of `mulx`. Unknown effects must retire homes. Review other consumers if repairing the shared effect model.

**Acceptance:** Add direct-pass and observable pipeline cases with the home in either operand, including partial-width swaps. Add `mulx` with a home in the non-trailing destination. The fresh LEA must remain, and stored addresses must match in CI.

### F5 — P1: the relaxed forwarding barrier misses `vmaskmovdqu`'s implicit memory write

**Locations:** `dead_writes.rs:1064-1104`; `store_forwarding.rs:1255-1282`, `2041-2063`.

The new forwarding logic relies on `line_writes_memory`. Its implicit-writer list recognizes `maskmov*`, but not `vmaskmovdqu`. For `vmaskmovdqu %xmm0, %xmm1`, the final operand is a register and there are no parentheses; the fallback returns false. This instruction writes memory through the implicit RDI address. A slot mapping backed by XMM15 can survive and a following reload can be replaced by a stale register value.

The added implicit-writer regression includes this exact VEX spelling. Ordinary user inline asm is pinned, reducing one production path to this hazard; it does not make the classifier or the admitted regression fragment correct. No emitted-C reproducer is claimed.

The new comment that unrecognized shapes report true is also inaccurate: the implementation is a last-operand heuristic, not a fail-closed effect allowlist.

**Implementation:** Decode the mnemonic and distinguish known read-only effects from possible/implicit writes. Add the exact VEX implicit-store spelling; do not indiscriminately strip `v` from arbitrary mnemonics. Prefer `ReadOnly`, `MayWrite`, `Unknown` effect classes and make `Unknown` a forwarding barrier. Audit implicit-store operations such as `movdir64b` when defining the admitted instruction set.

**Acceptance:** Direct classifier assertions for legacy and VEX implicit stores, RMW forms, register-only arithmetic, known memory reads, and unknown effects. Verify active reloads remain for every writer and still forward across supported read-only loads. Exercise both GP- and XMM-backed mappings in CI.

### F6 — P2: pinned dead-FP candidates report successful changes without changing anything

**Locations:** `dead_code.rs:298-311`, `1503-1507`; `types.rs:1176-1179`.

The new pass does not skip pinned candidates. `mark_nop` refuses to delete them, yet `changed` becomes true. The pinned-dead-FP test expects false. This pollutes change accounting and can cause unnecessary cleanup iterations; the driver is bounded, so an infinite loop is not claimed.

**Implementation:** Skip pinned candidates before analysis and set `changed` only for an actual edit. Add an idempotence assertion: a second pass over the stabilized result returns false.

**Acceptance:** The existing pinned fixture returns false and preserves metadata. Unpinned positive-control deletions return true only once.

### F7 — P2: a new preservation test inspects the unchanged text store rather than active instructions

**Locations:** `dead_code.rs:1476-1481`; `types.rs:1176-1189`.

`shufps_reads_its_destination_so_the_feeding_move_stays` searches `store.get(...)`. Deletion only marks `infos` as NOP; it does not erase the stored text. Thus the assertion passes even if the feeding move was deleted. This is a confirmed vacuous preservation assertion, not an allegation that all added tests are vacuous.

**Implementation:** Locate the fixture line before the pass and assert `!infos[index].is_nop()`, or use a helper that renders/filters active instructions. Audit preservation assertions in the other new fixtures. Keep downstream stores/returns observable where using the full optimizer.

**Acceptance:** Intentionally marking the relevant line NOP must make the assertion fail. Perform that mutation check in a supported CI environment; do not merge the mutation. Make direct and pipeline tests independent.

### F8 — P2: the nbody mass-load regression gate is representation-dependent and weaker than its claim

**Locations:** `tests/regression/check_nbody_perf_shapes.sh:43-45`.

Changing `exactly 2` to `at most 2` displacement-form loads is understandable after pointer hoisting, but `grep 'movsd +48\('` cannot count a mass read through a hoisted field pointer, a folded arithmetic memory operand, or a different instruction spelling. Zero matches now passes. A regression reloading via `(%reg)` can therefore evade the gate. The new struct-walk shape gate checks multiplies/base materializations, not equivalent mass-load counts.

**Implementation:** Track the mass-field address into the hot pair loop and count all equivalent accesses, or pin an IR semantic count before lowering plus a separate hot-loop machine-code metric. Require recognition of a nonempty function/hot loop and positive-control accesses. Do not merely restore the obsolete exact displacement spelling.

**Acceptance:** Synthetic assembly fixtures with hoisted-pointer duplicate reads, folded reads, and no recognized workload must fail. Legitimate hoisting must pass. A deliberate duplicate mass read must fail even if its textual displacement is zero.

### F9 — P2: repeated instruction scans increase compile-time and review risk

**Locations:** `iv_strength_reduce.rs:find_derived_exprs`, especially `find_affine` around 1338-1409; `vectorize.rs` legality/range helpers and AVX2/SSE2 reduction transforms.

The new affine walker repeatedly scans loop instructions for each frontier value and candidate; use discovery rescans the loop for derived expressions. This has quadratic components rather than a reusable indexed def/use model. The vectorizer also grows by 4,389 net lines to 34,190 lines and retains substantially duplicated target transforms. Source complexity is confirmed; actual compiler slowdown is unmeasured.

**Implementation:** Build and reuse immutable definition/use indexes during analysis; bound alias traversal and cache successful/failed affine recognition. Invalidate/rebuild indexes only at a documented mutation boundary. Extract a target-neutral validated reduction plan shared by AVX2/SSE2/NEON where semantics genuinely match. Avoid a huge speculative rewrite: separate correctness repairs from mechanical refactoring.

**Acceptance:** Preserve deterministic IR/output on existing fixtures. In CI or the user's supported environment, measure compile time on synthetic chains/diamonds with doubled instruction counts, plus large real workloads. Report scaling, not just one wall-clock sample. Explain any material regression and keep refusal paths mutation-free.

### F10 — P2: GCC provisioning is duplicated and helper behavior is under-tested

**Locations:** `.github/workflows/ci.yml:52-55`, `81-94`; `scripts/ci_install_gcc16.sh`; `scripts/ci_enable_swap.sh`.

The new helper installs GCC 16 early; the later original verdict-oracle step still adds the PPA and reinstalls GCC 16. This defeats the helper's stated distribution-first fallback and creates a second provisioning path. Selecting major version 16 and printing versions improves observability but does not make patchlevel/package identity reproducible. No helper-specific regression fixtures were found.

**Implementation:** Consolidate provisioning once, preserving QEMU, system and GCC16 multilib, G++, and any explicit development-package requirements. Replace the duplicate installer with verification if needed. Define whether the policy is rolling major-version compatibility or exact package reproducibility; do not call the former deterministic across time. Add PATH-stubbed shell tests for distribution/PPA selection, multilib package sets, invalid modes, sudo/root handling, already-active swap, and command failures. Use consumers that drain version-report output and do not mask producer failures inside `printf` substitutions.

**Acceptance:** A mock command log shows one provisioning sequence and all required packages. Both setup modes and idempotent swap behavior are covered without root/network side effects. Hosted CI confirms the real environment, including i686 execution.

### F11 — P3: historical narrative contradicts corrected ISA behavior and overstates reference independence

**Locations:** `engineering/SESSION-PEEPHOLE-RELAY-2026-10-13.md`, section 2; `tests/regression/check_ivsr_vecaddr_struct_walk.sh`, reference comment.

The session narrative groups legacy `shufps`/`shufpd` with source-only full definitions. The implementation correctly excludes them because they read the destination. Preserve the historical account if useful, but label the old model as superseded. The pairwalk reference comment says it has no struct-array walking even though it walks `ref[i]`/`ref[j]`. The independent GCC executable, not the same-TU reference alone, supplies the independent oracle.

**Implementation/acceptance:** Correct these statements; qualify imported benchmark ratios with compiler versions, workload, metric, flags, and provenance. Instruction-count improvements are not automatic native-runtime improvements. Do not present imported measurements as rerun results.

## Additional risks to investigate, not counted as confirmed defects

- `build_alias_map` deliberately treats same-width non-floating casts as bit identity, while affine/IV recognition uses mathematical offset-value preservation. Review narrow signed/unsigned reinterpretations at address uses and all target pointer widths; add explicit negative tests. I have not demonstrated a new valid-C miscompile from this distinction.
- Positive signed-overflow assumptions in IVSR depend on the driver's documented language contract. Verify that no supported wrapping option or internal modular integer transform bypasses the assumptions.
- Profitability evidence is benchmark-driven. Quantify spill pressure and hot-loop work on x86-64 and i686, and prevent x86 cost decisions from perturbing non-x86 code generation unexpectedly.
- Inspect invalid-input/refusal tests for byte-for-byte IR stability, `next_value_id` consistency, and source-span accounting. An added guard is only as good as the state it leaves on rejection.

## Handoff sequence for the implementing agent

1. **Correctness commit:** repair F1-F5 with direct-pass negative tests and observable pipeline positive twins. Prefer safe refusal over a partially modeled optimization. Preserve all existing negative tests.
2. **Test integrity commit:** repair F6-F8, add active-instruction assertion helpers and mutation checks, and strengthen semantic load recognition.
3. **Infrastructure commit:** repair F10 with mock-only helper tests; document the oracle policy and retain all current runtime dependencies.
4. **Refactor/measurement commit:** address F9 with indexed analysis and a shared validated plan; measure complexity, target codegen, and runtime/compile-time tradeoffs on supported CI or external hardware.
5. **Documentation commit:** repair F11 and update PR claims to the actual final results.

Historical handoff constraint (superseded by the subsequent explicit testing request): do not compile in this Arena harness. Do not weaken tests to obtain green CI. Push changes on the session branch and use GitHub CI for compilation, unit tests, SSA-validation regressions, differential/runtime checks, lint, and target validation. External native performance work requires the user's supported environment.

### Target criteria for a 10/10 re-review

- Zero unresolved P1 findings; all preservation/deletion decisions account for complete architectural state.
- All five currently contradictory test functions pass for the right reason, and their negative controls demonstrably detect a broken implementation.
- Preservation tests inspect active output; shape gates cannot pass on an unrecognized/empty workload.
- Refused transforms leave valid, unchanged IR; successful transforms have checked dominance, integer domains, exact update identities, and metadata invariants.
- Shared effect models are conservatively complete for admitted instructions; opaque cases are explicit barriers.
- Full required CI green on the final compiler-changing head, with no skipped/softened correctness gates.
- Reproducible performance and compile-time evidence: same workload/tool identities, flags, target, repeated runs, correctness-checked outputs, and reported variance. No unqualified speed claims.
- Provisioning has a single ownership path and mock-tested failures/idempotence.
- Reviewable, isolated commits and documentation consistent with implementation.

This checklist defines what would justify a top grade; it is not a promise that the eventual implementation will earn one without re-review.

## Validation and CI snapshot

Local non-compiling checks passed: CI/local parity (216 commands, 22 invocation contracts), conflict markers, diff whitespace, and the repository's pipefail/SIGPIPE static checker (164 wired shell scripts). The earlier integration also checked changed Python ASTs, shell syntax, and workflow YAML/embedded shell syntax. Small independent arithmetic/grammar witnesses were inspected; they are not execution of the Rust passes.

At the audit snapshot for the reviewed compiler-change commit, Clippy, MSRV, and Compiler Explorer oracle delta had succeeded. Test Suite and LCCC-vs-GCC benchmarks were still in progress. The Test Suite was still at its Build step when detailed job steps were inspected. No successful full correctness run is claimed. Documentation-only follow-up commits may trigger additional runs; check final required statuses before any merge decision.
