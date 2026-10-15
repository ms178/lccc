## Complete the address model. Preserve architectural state. Verify the evidence.

This PR connects scalar GEP offsets, GVN alias glue and vector-memory address
triples; tightens vectorization preflight; and adds x86 peepholes aimed at
redundant copies, address materializations and reloads. An empirical follow-up
repairs the safety defects found by the initial audit instead of treating
integration fidelity or successful lint as semantic approval.

**Review scope:** the demonstrated safety repairs are tested; exhaustive bug
absence, universal performance superiority and unconditional merge approval are
not claimed. The repaired workload-specific nbody census and remaining refactor/CI blockers are
listed explicitly in [the empirical report](engineering/PR824-EMPIRICAL.md).
The [original audit](engineering/PR824-AUDIT.md) remains as a clearly historical
record, with its validation status superseded.

## Current latest-main follow-through and acceptance

Rebased onto `3ff36e8`. Runtime-U64 maps keep the checked byte-domain guard,
scalar fallback and final single-byte-IV fusion. The semantic nbody gate now
counts hoisted/folded reads, tracks actual frame provenance and rejects hidden
loop effects even when the header re-establishes pointers (six red-team mutants
failed before repair). Thirteen census methods and the real shipping gate pass.
Both GCC and swap helpers have seven offline mock methods, covering both
privilege paths; no production privilege/swap-verification bypass was added.
Live Compiler Explorer surveys are **research-only, not GitHub or local CI
gates**. Offline helper self-tests remain. The actual earlier hosted survey
failed because classic ICC rejected `-march=x86-64-v3`; the research helper now
allows explicit Haswell dialect translation with requested/effective flags and
a compile-only ISA admission probe. That probe has offline tests, not a claimed
real four-vendor admission result. Diagnostics retain precise rejection text.

A subsequent lazy CFI-boundary-index prototype was **rejected and reverted**.
Two alternating Callgrind pairs per case produced candidate/baseline compiler-Ir
ratios: minmax **1.002640**, libm **0.997845**, chain-64 **0.999879**;
six-pair geomean **1.000120**. Native/instrumented and baseline/candidate assembly
were byte-identical throughout. This essentially neutral screen does not
justify the new whole-file allocation and invalidation contract. Final Rust
source is unchanged from the earlier folded compiler. See
[the retained screen](engineering/evidence/pr824-rebase/boundary-index-screen.json).

Final restored Rust all-target tests: **4,714 library passes / zero failures /
eight ignored**, plus the bin test. Final Python corpus: **465 tests OK, four
skips**. Rust1.99 formatter and strict Clippy pass
(76 policy names probed, zero unknown). The in-flight fast run ended
**201 passed / 42 failed / 13 skipped**, exit1, and also emitted a partial-comment
shell error after its controller was edited during execution. It is explicitly
**not** an immutable final-tree acceptance stamp. Missing local prerequisites
remain blockers. See `engineering/evidence/pr824-rebase/no-godbolt-fast-validation.json`
and the associated failure excerpts. No full/slow local CI was run.

**Earlier folded-stage validation (not a current-head stamp):**
Latest fast execution is **NOT GREEN: 202 passed/42 failed/15 skipped**. Rust1.99
has **4,714 passing tests/0 failures/8 ignored**; rustfmt and strict Clippy pass.
All 455 final Python corpus tests pass (four skips), plus both helper suites and
the focused genuine x64 map/real nbody gates. Follow-up Python files changed
while fast CI ran, so no immutable final-tree acceptance stamp is claimed.
Missing GCC16/multilib/libgcc, Rust1.97 and external pinned tools remain blockers.
No full/slow local CI ran.

Fresh independent-main Callgrind: all nine runtime comparisons and separate
GCC12 comparisons are correctness-gated and complete. The admitted U64 map
costs **1.011168 candidate/main simulated Ir**, not a free optimization.
Minmax/libm compiler work improves; synthetic chains/diamonds show roughly
1.5–5.0% extra whole-compiler simulated work. A larger diamond baseline times out
at 180s; it is preserved as incomplete, with four narrower doubled points then
completed. These are simulated work counts, not native speedups. Explicit current-head
four-vendor research comparisons, all-green fast CI and broader target-neutral
plan extraction remain unfinished. Full hashes/counters and failure evidence
are in [the current audit](engineering/PR824-REBASE-AUDIT.md).

## Compiler changes

### Address recurrences and vectorization legality

- Discover address uses through Copy/identity glue and modeled vector-memory
  triples, retaining displacement and store-value argument layout.
- Group recurrences by IV, stride, affine offset, product domain and invariant
  base; share equivalent symbol bases and materialize fresh preheader addresses.
- Check unsigned affine addition and offset-product wrapping independently;
  keep the scaled-index profitability rule x86-specific.
- Preflight exact exit comparisons, reduction seeds/uses, remainder references,
  supported multi-reduction shapes, signedness, bounds and unit strides before
  mutating IR.
- Index immutable IVSR definitions, canonical address uses and raw-use membership
  once per discovery scan instead of repeatedly walking instructions. Keep all
  existing legality proofs and alias/raw-use distinctions.

### Safety repairs to x86 peepholes

- Extend implicit-effect protection from dead-move elimination to vector
  propagation, bracket privacy/upper-lane proofs and spill-home invalidation.
  Distinguish string-mask XMM0 writes from string-index ECX writes; preserve
  existing VZEROUPPER and profitable known-FMA controls. Retain only verified
  state-image mnemonic aliases, and share positive ISA admission/MAY writes.

- Track low-XMM, YMM-high and ZMM-high liveness separately. VEX move deletion
  requires every written region dead; legacy low-only deletion remains enabled.
- Classify all eleven exercised flag/string-only VEX tests as reads, not definitions.
  Optimizer tests are backed by eleven executed flag/string witness pairs
  and nine implicit-state/copy/spill original/illegal-rewrite pairs.
- Preserve observable upper zeroing across legacy overwrites, masked wider
  observations and CFG paths; retain destructive EVEX destination dependencies.
- Preserve pinned FP moves and CSE boundaries, including metadata/change flags.
- Share stable-symbol recognition between both LEA CSE implementations. Reject
  `.`, location-counter addends and numeric RIP-relative expressions while
  retaining ordinary symbols/addends and `.LC0`.
- Invalidate address homes with conservative MAY-write effects, including both
  `xchg` operands, partial writes, multi-output operations, string-comparison
  outputs and unknown instructions.
- Recognize masked streaming stores, direct stores and enqueue instructions as
  memory-write barriers.
- Replace a vacuous immutable-text assertion with an active-line assertion and
  add positive/negative preservation, deletion and invalidation matrices.

## Harness and oracle quality

- Add actual Debian-package **GCC 16.2.0 / Rust 1.99.0** correctness CI over the
  complete canonical benchmark registry, with exact version checks and retained
  commands, source hashes, outputs, binaries and disassembly.
- Keep this correctness-only lane separate from the existing repeated native
  benchmark workflow. Do not call static counts, Callgrind Ir or VM timing a
  bare-metal speedup.
- Consolidate GCC installation into one owner, retain system/versioned multilib
  and development objects, refuse Ubuntu PPAs on Debian, support exact-version
  enforcement, avoid reinstalling ready packages and propagate failures.
- Add GCC/swap offline provisioning and registry/evidence tests to both
  hosted CI and its local mirror.
- Add `scripts/benchmark_codegen.py`, using non-rank batch oracle mode and the
  **45-workload registry**, not a source glob. Preserve shared-source variants'
  macro flags, verify report source/flag identity and reject stale/missing output.
- Retain the diagnostic publisher for explicitly produced research evidence,
  with bounded output and artifact hashes. There is no live Godbolt GitHub job
  or local survey gate. Correct the stale 39-workload narrative.

## Earlier fb02007 follow-through (historical, superseded status)

The earlier rebase preserved 20 native architectural controls, wide-minmax
and FMA guard repairs. Its 197/44/15 fast result and unresolved U64/F8 issues
are historical; the current source repairs those two issues as described above.
Earlier exact-version Debian GCC16 evidence includes 45/45 correctness and 45
Callgrind rows, with a simulated-Ir geomean 1.000095 and material individual
losses. That is earlier-head evidence, not current-head validation or speedup.
The prior nine-case/main and alternating compiler-work measurements retain
`fb02007` identities in the audit; the fresh `3ff36e8` measurements are separate.
Current-head hosted results and all-green fast acceptance remain mandatory.

## Earlier-stage executed evidence (not final-head acceptance)

Full methods, flags, identities and limitations are in
[PR824-EMPIRICAL.md](engineering/PR824-EMPIRICAL.md).

- Baseline CI empirically reproduced the five failures predicted by the audit.
- Rust **1.99.0** local all-target tests: **4,684 library tests passed, zero
  failed, eight ignored**; remaining targets passed, including one bin test.
- Rust 1.99 formatter and strict Clippy wrapper passed.
- Local native output differential: **45/45** at `-O2`, and **45/45** with
  `-fwrapv` in both arms. Local reference: honestly labelled **Debian GCC 12.2**.
- Actual hosted Debian **GCC 16.2.0 / Rust 1.99.0** full-registry correctness
  lane passed at the first repaired infrastructure head; consult current-head
  checks for the final run.
- Workload-macro-aware static A/B: **45/45 assembly listings identical** between
  baseline and repaired compiler at `-O2 -march=x86-64-v3 -fwrapv`.
- Six-pair, alternating AB/BA affine-kernel compile-time screen at four doubled
  sizes: repaired/baseline median ratios **1.004, 0.989, 0.986, 0.989**. These
  small VM differences do **not** establish a measurable speedup.
- CI/local parity: **219 commands / 22 invocation contracts**. Conflict-marker,
  whitespace and pipefail/SIGPIPE checks passed; four GiB of swap was activated
  and actually used during the constrained local build.

## Integration integrity

The supplied patch was fetched through the **GitHub Contents API**. An existing
upstream workflow overlap was reconciled explicitly while retaining QEMU and
local-CI additions. Strict forward/reverse application passed with **zero fuzz**
and full context; offsets were permitted. The 32 result files outside the two
upstream-diverged CI files matched the supplied result blobs exactly.

Subsequent commits are substantive safety, indexing, test, harness and evidence
changes; integration fidelity alone never established correctness.

## Remaining limitations

- The repaired nbody census is workload-specific and intraprocedural; it does
  not establish runtime speed or universal semantic load optimality.
- Broader target-neutral vectorizer plan extraction remains follow-up work;
  helper mocks are now present, but real hosted provisioning/i686 still matter.
- The exercised effect tables are not a formal model of every x86 extension.
- Fresh local Valgrind/Callgrind evidence exists, including regressions and an
  explicit timeout. No completed current-head live four-vendor sweep, hardware-PMU
  result or exhaustive input/target/ABI proof is claimed.
- Required checks must be green on the **current head**. An earlier successful
  job is not evidence that a later head passed; no correctness gate is weakened
  or skipped to obtain green.
