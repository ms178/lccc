# PR831 follow-up audit — 2026-10-15

**Follow-through:** current main63e783 / PR831acd48b0 integration, oracle repairs,
fresh vendor counterexamples and validation are in [PR831-ORACLE-RECHECK.md](PR831-ORACLE-RECHECK.md).
The hosted-run identities below describe the earlier audit, not the latest upstream.

## Scope and honesty boundary

Reviewed hosted head: `9881506686a270497d635917dde75297ed805a5f`.
Integration base: `7f9eb225def23bda9370d3349304b9b785554fea`.
Dedicated follow-up branch: `audit/pr831-redteam`.

The previous S41 local FAST pass was not a hosted/full pass. The PR831 source
also removed ten historical plaintext evidence logs; its compiler source was
otherwise the delivered revision. This follow-up preserves those logs losslessly
as gzip with raw-content hashes in `engineering/evidence/pr828/RAW-LOGS.json`.
It does not remove their tests or silently rewrite historical results.

No external expert approval, universal bug-freedom, exhaustive review of every
repository function, or optimality claim is made. The review lenses are ISA/ABI
semantics, dataflow proof soundness, arithmetic domains, reproducibility, and
performance controls; they are not fictitious independent human reviewers.

## Hosted failures: evidence before hypotheses

The completed PR831 run reports two failed execution lanes and a correctly
failed aggregate. Its build, Clippy, MSRV, assertion-enabled units/corpus, SSA
corpus, and codegen/runtime lanes report success. These are results of the OLD
hosted revision, not acceptance of this follow-up.

1. **Assembler/linker:** downloaded `linker-results.json` contains 302 PASS,
   zero FAIL/WARN, and one SKIP: `plain_gotpcrel_relaxation`, because the fixture's
   assembler rejects MOVRS. Strict mode correctly refuses this skip.
2. **Frontend/ABI:** installation of GCC16/multilib exits 1 before the tests.
   Public job metadata identifies the failing step but not the package-manager
   diagnostic. Authenticated log downloads were unavailable; the user selected
   continuing without supplying that log. **The precise cause remains open.**
   The independently found C++-version check gap below is NOT claimed to explain
   or repair that hosted installation failure.

## Attributable repairs and alternatives

### A. Select the pinned assembler for the assembly fixture

`_plain_gotpcrel_test` used `CC -Wa,...` while the sibling MOVRS fixture invoked
`as` directly. Putting GAS2.47 on PATH does not override a compiler configured
with a private/absolute assembler. Both `pg.s` and `sc.s` now invoke
`as --64 -mrelax-relocations=no` directly; C compilation and linker shims remain
unchanged. The existing exact relocation, byte, runtime, PIE/static/shared and
negative script-link assertions remain intact.

Rejected: weakening `--strict`, deleting MOVRS, or applying a global compiler
`-B` that can also change CRT/library/linker selection. No compiler production
change is needed for this harness defect.

Evidence: new controller tests fail before the fix, pass afterwards, and are
wired into hosted and local FAST. A real GNU as2.40 private-driver control
reproduces the reported diagnostic before repair and passes the complete
four-mode fixture after repair. System GAS2.44 passes BOTH versions: the
original local environment was insufficient to expose the selection defect.
The old-GAS control is a reproduction of the mechanism, not an Ubuntu VM replay.

### B. Exact byte-range endpoints beyond signed displacement limits

The inherited shared `ranges_overlap` helper added i32 displacements and widths
in i32. A valid memory range can extend past i32::MAX even though its start is
an encodable displacement. With overflow checks this panics; without them it
can wrongly say two overlapping regions do not overlap. Empty ranges also
incorrectly overlapped in some cases.

The helper now excludes empty/nonpositive ranges and adds endpoints in i64.
Two i32 inputs cannot overflow that domain. This preserves exact overlap rather
than saturating/wrapping or treating every boundary case as an opaque alias.
The changes affect compiler-side reasoning, not inserted target instructions.
All callers were located: dead-code/store forwarding, FP slot liveness and
frame compaction. An FP consumer test checks a live read spanning the signed
endpoint and a genuinely disjoint negative-displacement positive control.

Evidence: the original helper fails two extracted-source tests with overflow
checking both enabled and disabled. The repaired helper passes an independent
i128 interval oracle over 8,100 combinations, plus the direct boundary case.
The integrated consumer and full library pass. This is a proven proof-helper
failure; no fabricated end-to-end C miscompile is attributed to it.

### C. EVEX decorator whitespace must not change write obligations

Whole-write deletion proofs stripped a mask but did not trim the preceding
whitespace. A GAS-valid `%xmm2 {%k1}` destination therefore lost its register
identity and could evade the upper-zero write obligation. The regression was
red in the actual Rust suite (one failing, one passing neighboring test).

Both whole-write and upper-effect projections now trim the undecorated operand.
Decorated destinations conservatively retain partial-write treatment, including
spaces INSIDE the braces. This also avoids introducing a merge-mask bug while
fixing register recognition. Unmasked full definitions retain their kill proofs;
no optimization family is disabled and no global assembly reformatting is needed.

Tests cover four spellings, observable XMM upper-zero writes, and YMM merge-mask
inputs. A native AVX512F/VL architectural witness on this Xeon additionally
checks all four spellings and a deliberately deleted-producer mutant: four
correct upper-zero results, while deletion preserves the nonzero high 48 bytes.
This witness is NOT a full C optimizer reproducer and is NOT 14700KF evidence;
that processor does not expose this AVX-512 ISA.

### D. Validate both installed GCC toolchain drivers

Package status plus a binary named `g++-16` did not establish its version.
A controlled PATH with GCC16.2 and G++15 was incorrectly certified. The helper
now checks `g++-16 -dumpfullversion` against the validated C driver's version,
including the already-installed fast path. Exact-version overrides and the
existing rolling-major policy remain intact. Four mismatched/malformed-version
subcases were red, then green; the complete eight-test provisioning suite passes.

### E. Preserve evidence without patch whitespace warnings

Ten historical raw logs were compressed, decompressed and SHA256 checked.
`git diff --check origin/main` is clean; this is not a whitespace-ignore switch.
Historical log bytes are preserved, not edited to hide failed runs.

## Wider review and executed controls

The cumulative patch changes twenty Rust files including embedded tests. The
review revisited IVSR definition indexing/dominance and symbol-base reuse,
Copy-only identity, signed-width reduction/remainder bounds, pointer-ring versus
byte-count representability, transactional packed integer DAG construction,
LICM canonical write/ordering barriers, and vector low/upper/slot consumers.
Known CFG/directive, pinned, volatile and FP restrictions were retained.
This is not a claim that each repository function was exhaustively verified.

Full library after B/C: **4,747 pass / 0 fail / 8 ignored**. This includes module
coverage for x86-64, i686, ARM/AArch64 and RISC-V, not native execution of all
architectures. The fresh assertion compiler passes the 22 targeted reported
cases with IR/SSA validation. CI parity: **226 commands / 22 invocation contracts**;
71 CI-gate-parity tests and the eight GCC-provisioning tests pass.

All compiler/linker builds use fastbuild, opt-level1, exactly `-j 2`; 4 GiB swap
is active. Normal candidate, independent immutable main, and assertion compiler
have distinct recorded identities. No release/LTO iteration was used.

## Runtime-codegen controls after the repairs

Three alternating pairs for each of five workloads, GCC16.2 output oracle,
equal-length executable paths, fixed inputs/flags/cache geometry, Callgrind:

| Workload | Candidate/main whole-process simulated Ir |
|---|---:|
| nbody | 1.0 |
| fannkuch | 0.9523269469412591 |
| histogram | 1.0 |
| libm_round_family | 1.0 |
| lz4_match_extend | 1.0 |

All three ratios per workload agree. Fannkuch: **1,230,175 / 1,291,757 Ir**,
a reduction of **4.7673%**, not cycles or native speedup. N6/N8/N9 three-pair
controls also improve and agree with GCC16 and the assertion compiler.
The ten candidate/main measured ELFs are byte-identical to their S41 counterparts;
these repairs are NOT claimed as an additional workload-codegen improvement.
Fresh whole-process counters are retained separately from historical counters.

All fifteen current candidate/main/GCC executables were separately recompiled:
byte-identical ELF, empty compiler stderr, exact output digests. Disassembly,
raw stdout/stderr, and verified compressed raw Callgrind counters are retained.
Optional Godbolt GCC16.2, Clang23.1 and latest ICX accept the same bounded
fannkuch ISA/optimization flags, have empty compiler diagnostics and execute
with the expected output. This is not a native performance ranking of vendors.
No live Compiler Explorer merge gate was added.

## Final acceptance boundary

This tracked document is frozen BEFORE the final immutable FAST run. Its result
and exact before/after identities will be external receipts under
`artifacts/pr831/`, so publishing the result does not mutate the accepted tree.

Still required: resolve/replay the hosted GCC installation failure, rerun the
hosted lanes on the new revision, and obtain native 14700KF/PMU evidence before
claiming that target's performance. Broad effect-descriptor consolidation and
universal missed-optimization claims are not declared completed. No push and no
production-ready/zero-issues certificate are issued by this report.


---
# Independent cumulative-session review (historical base 7f9eb225)

# PR831 red-team synthesis — validation in progress

Upstream checked 2026-10-15: main 7f9eb225def23bda9370d3349304b9b785554fea; PR831 open at 9881506686a270497d635917dde75297ed805a5f. Pre-rebase S54 saved exact history. Rebase was a no-op. PR831 has 133 changed files, including substantial inherited research evidence; those historical files are not fresh validation of this synthesis.

## Independent control

PR831 built independently with scripted fastbuild O1/j2, stable Rust 1.99, Debian GCC16.2 and exact mold2.42.1 (built X86_64/I386 only). Compiler SHA256 01503f8b1383b2ec37d2f02730c1c6e9880d07c38230a1785a622c24f28ab5d8.

The expanded byte-address witness segfaults at O0/O1/O2/O3/Os/Oz with accepted -fwrapv. Dynamic rounding exits 41 at O0 and 21 at the other five levels. Boundary matrix stops at 29 PASS / 1 FAIL (partial, not full coverage). Raw results are attached under evidence/pr831-review. Thus PR831 is not a replacement for our cross-layer semantic repairs.

## Accepted and retained choices

- Accept the x87 store/pop/reload rounding barrier: FST rounds memory, not the cached 80-bit register. Keeping an unrounded cached value violates the advertised F64 contract.
- Accept canonical fallback barriers in both LICM memory models, including atomics/fences and incompletely modeled writes. Keep our independent FP-environment reordering fence.
- Accept strict reciprocal memory effects and scalar result-type classification. Keep the four-lane/m128 packing implementation, not a duplicate eight-lane engine/intrinsic spelling; its smaller exact input contract already has bitwise and ISA/allocator-disable tests.
- Accept whole vector write-region liveness, masked-copy rejection, distinct read versus guaranteed-write footprints, and unknown-CFG fail-closed behavior. Retain our CLZERO, string, AVX2 gather read/write-mask and UTF-8 fixes; replacing files wholesale would lose these protections.
- Accept Copy-only min/max identity and separately proved pointer-domain address bits. Retain narrow -fwrapv recurrence guards. The dominating-symbol reuse policy is retained for empirical evaluation, not presumed faster.
- Do not replace the constrained-host CI resource budgeting with hard-coded two-job unit/Clippy gates. Do not import historical JSON bulk or claim PR831's deferred validation has passed here. Existing oracle scripts already include several identical improvements.

## Additional red-team findings beyond PR831

PR831's read extent could underestimate packed narrowing conversions: `vcvttpd2dq (%rsp), %ymm7` reads 64 source bytes, not its 32-byte result width. GNU as/objdump independently accepts the EVEX instruction. Also, unsuffixed `cmovl (%rsp), %rax` means signed-less conditional move of eight bytes; the `l` is not a four-byte size suffix. Added source-width-aware proofs and liveness boundary tests rather than disable the entire optimizer.

The shared ranges_overlap helper added signed i32 displacement and width directly, wrapping in optimized builds near INT32_MAX. PR831 fixed one local covering-store calculation but not this common overlap predicate. Widen both endpoints to i64 and test both signed boundaries, adjacency and empty intervals.

## Outstanding acceptance

Candidate compiler build, new unit tests, all fast CI, cross-target matrices, exact-oracle/Valgrind checks, fresh canonical corpus Callgrind and isolated paired-native runs remain mandatory. Previous cache-only measurements were lost at harness reset and are not evidence for this tree. No claim of universal compiler superiority, zero defects or production readiness is made.

### Fresh build and tool-discovery defect

The S55 compiler built warning-free (SHA256 7cc2b4db2432232d25541e871683794fb3f4102d1a463c684d06e380d4041bc2). Fast CI was stopped after multiple i686 links failed to find libgcc. Installing Debian's unversioned multilib metapackages did not fix it: the actual compiler bug was a hard-coded GCC-version list ending at 14, despite GCC16's valid `/usr/lib/gcc/x86_64-linux-gnu/16/32` runtime. Do not misclassify this as missing infrastructure or disguise GCC16 as GCC14.

Replaced the direct linker's release allowlists with numeric installed-directory discovery, retaining same-root file checks and multilib-only ABI separation. Verbose library-path reporting now delegates to the same built-in linker search authority rather than an independently stale list. A new unit test covers numeric ordering, future versions, malformed names, duplicate roots and non-directory entries. Fresh build and gates are required again.

### Validation-driven follow-through

S56 binary 3c177d908a5105e98581c32a32bf28958d3e33e720a2d167d040a8ea669b37f4 passed 228 cross-boundary cases, 54 FP cases, 480 RV binding/oracle cases, 13 real RV loader cases, nine 1260-bitcase packing/Memcheck arms, and 24 exact-oracle/ABI/domain Memcheck cases (after installing matching 32-bit glibc debug symbols). PR831 independently returned 250/230 binding results and 3/10 loader results.

Fast CI exposed a synthesis mistake: three-way merging duplicated two existing LEA-CSE test blocks. Restored their already-equivalent S54 version, without dropping unique tests or changing runtime code. That failed run is not a pass.

The additional x87 cancellation probe also exposed missing GNU FP evaluation macros: GCC16's real `<float.h>` expands `FLT_EVAL_METHOD` to `__FLT_EVAL_METHOD__`, which was undefined even though the fallback header macro said 0. Define both GNU evaluation-method macros consistently as 0, and add an executed declared-F64 cancellation witness at all six optimizations on both native ABIs. This repairs standard-header use rather than hiding the error behind a test-local macro definition. A fresh full fast run is still required.

### S58 follow-through: concrete recurrence findings

S58's 4771 Rust library tests and integration test passed, but fast CI found a dynamic-rounding mismatch in the existing FP recurrence fixture (loop length 16, changing environment). `CCC_NO_BEPRE=1` restores exact digest e778c29691d2c901; disabling the unrelated square/doubling optimizers does not. Backedge PRE carried a square computed under the preceding iteration's rounding mode. The repair rejects only FP candidates when the TU's FP environment is observable, retaining integer PRE. The new IR test exercises both policies and the integer positive control. The existing executed multi-mode fixture remains enabled; FMA structural positive controls now explicitly disable the FENV pragma, with an observable-FENV negative control retained.

The TLS IR dump also identified a sound missed proof: `(signed_i & 7)` had no propagated range, so its following `+1` aborted wrapping-safe IV widening. A nonnegative mask clears the sign bit and gives `[0, mask]` for signed as well as unsigned operands. Reuse that fact; do not relax any wrap guard. Added negative masks and overflowing-follow-up unit cases plus a four-architecture executed boundary fixture. S58 no-wrap ablation ratios were TLS 1.17968, sieve 1.11872, strstr 1.03615, fannkuch 1.02622 and varint 1.02187; the ablation is diagnostic, not an acceptable replacement for -fwrapv.

S59's 4773 library tests and integration test pass. The new FP barrier restores all eight executed exact-bit/FENV/oracle combinations. The structural probe separately needed its fixed-environment positive fixture: observable-FENV lowering intentionally does not fold the negative-zero initializer, so the square proof correctly declines that shape. Positive structural cases now use `NO_FENV_PRAGMA`; executed dynamic-rounding cases remain unchanged and the FMA observable-environment negative control remains. The corrected full FP script passes.

Fresh S59 performance: 45/45 output-equal Callgrind cases in each comparison. Candidate/PR831 Ir GM 1.0010277021, AM 1.0014044950; candidate/GCC16 GM 0.9886007778, AM 1.1658291278. TLS now exactly matches PR831's Ir (ratio 1.0), recovering the previous 18.036% instruction-count penalty without weakening -fwrapv. These are bounded whole-process simulated counts, not native speedups. Cross boundaries are now 265/0/0. RV binding 480/0/0, RV loader 13/0/0, all nine packing/Memcheck arms and 24 exact-oracle Memcheck cases passed again.

## Final measured comparison

See **PR831-COMPARISON.md** and the durable fresh-evidence archive for the
completed 45-case counter and 20-case native comparisons, all individual
ratios and regression investigations. Performance approval is withheld:
20-case native GM is 1.011893 against PR831, with fannkuch +23.04%; removing
wrapping semantics is not an acceptable optimization. Original-priority15
GM is 0.996794, not a statistically demonstrated overall win. Full fast-CI
status is published separately in the delivery ledger after the final tree's
run, avoiding self-referential evidence edits that invalidate its stamp.

### Full S61 fast run: three remaining failures, all recorded

The complete run returned 253 pass / 3 fail / 6 slow skips. Strict Clippy
(76 lint names), fmt and MSRV passed. The failures were not accepted as green:

- i686 NOTYPE execution produced `42 7`, but readelf could not decode the PLT
  relocations: the real bug is absent DT_RELENT when only .rel.plt exists.
  Emit entry size 8 whenever either REL table is present, once only; cover all
  four presence combinations and the existing serialized-size matrix.
- The PT_NOTE test treated an eight-byte gap containing seven bytes of
  `.rodata` as padding. The linker correctly emitted two spans; merging that
  data would be wrong. The independent expectation now checks occupied
  intervals and virtual continuity, with five focused tests. Note coverage
  is checked exactly once, not merely at least once.
- Zstd static stack operands rose 8→11 (+3). This is an **explicit ratchet
  exception**, not a claimed optimization or hidden global tolerance increase:
  only this metric's baseline changes. The nine-pair native point estimate is
  1.004486, CI [0.996202,1.015288], with all outputs equal; bounded Ir is about
  +0.89%. The extra stores remain an optimization backlog item. Performance
  approval is already withheld; the baseline exception does not override it.

The only new compiler change here is i686 dynamic metadata, not x86-64 code
generation. Revalidate the linker gates and prove x86-64 benchmark executable
identity before carrying the completed native results forward.

The rebuilt metadata-fix compiler passes the executed NOTYPE gate and all 90
whole-executable x86-64 identity comparisons. See x86-binary-equivalence.json;
no native timing was silently reassigned to different generated code.

## Latest-main and PR831 refresh (S64 onward)

Upstream moved during final CI. S63 was saved before stopping that incomplete
run. The cumulative patch is now based on PR832/main
`63e783b15586eca86f306bd75a52b82c57304bbb`; PR831 is now
`acd48b0ca8b8427e95d30ecdae72bfb1fd888f38`, adding FXRSTOR liveness and CI
failure/assembler follow-ups. Keep its low-XMM-only FXRSTOR kill, masked XRSTOR
negative controls, and user-visible failure annotations. Keep main's direct-GAS
fixture routing and add the explicit assembler pin from refreshed PR831.

Resolution decisions: preserve main's isolated hosted lanes and stronger mask
spacing, nbody census, GCC/G++ version and boundary checks. Preserve the proven
128-bit packed denominator rather than importing a second overlapping 256-bit
DAG engine; use the upstream intrinsic name consistently in all allocator
classifications. All other vectorizer changes agree. The main integration's
runtime packing gate retains all 9 ABI/home/ISA legs. Its no-legacy-transition
assertion now inspects the production pipeline: disabling VEX promotion is a
correctness diagnostic, not the shipping codegen profile of the 128-bit DAG.
Do not resurrect main's intentionally removed obsolete hosted PR824 research
job; keep its reusable offline runner and tested publisher.

The rebuilt compiler again produces all 90 whole x86-64 benchmark executables
byte-identically to the original measured candidate. This is proven, not
inferred from source similarity; see main63-binary-equivalence.json and the
reusable revalidate-binaries.py. The previous perf results therefore still
apply to the candidate's generated code. Latest-main and latest-PR831 control
identities require separate explicit evidence; do not relabel old controls.
