# LCCC engineering backlog — Maybach

Re-triaged 2026-10-17 against `e8ae10ab621589fac61f8504adffa812df475856`.
This is an **open-work queue**, not a compiler superiority claim or a green-CI
receipt. The previous 3,301-line file is preserved byte-for-byte at
[the archive](engineering/archive/BACKLOG-2026-10-16.md).

Canonical method: [rules](engineering/agent/RULES.md),
[negative results](engineering/DECISIONS.md),
[performance catalog](engineering/agent/BACKLOG.md).
New review dispositions and measured limits belong in
[the current audit](engineering/MAYBACH-2026-10-17.md), not session narratives here.

## Evidence and acceptance discipline

- **R**: reproduced with a recorded binary, flags and result. Historical R is
  not a claim that the current tree still fails. **S**: source observation.
  **H**: hypothesis; first deliverable is an experiment.
- Trust, completeness and debuggability precede compile speed and output speed.
  An accepted semantic/ABI/security request must work or be refused.
- Build fastbuild with exactly two Cargo jobs, no concurrent heavy work. Ensure
  swap and native multilib/cross-runtime prerequisites; record tool identities.
- Before delivery: stable-tree `bash scripts/ci_local.sh --fast`, rustfmt and
  strict Clippy. Full/slow CI is hosted-only. Preserve failing reproducers.
- Codegen changes need fresh relevant oracle, gzip longest_match stack-memory
  comparison, and applicable identity/encoding gates. Vectorizer/RA changes
  additionally need workload or targeted-kernel plus differential fuzz checks.
- Every new regression gate needs a failing mutation control. Runtime comparisons
  check exit status and output on every sample. Callgrind Ir is simulated work,
  not CPU cycles. No 14700KF result exists without measurement on that hardware.

## Ranked implementation queue

### MB-01 · P0 · R/S — honest compiler option contracts (partial)

**Why:** upstream silently accepts unknown -f flags; build probes can falsely
report hardening/LTO support. The current patch is not yet a complete option
registry: existing stateful arms remain separately implemented.
**Where:** `src/driver/cli.rs`, `src/driver/option_table.rs`,
`tests/regression/check_driver_option_contract.sh`.
**Reproduce:** `lccc -fdefinitely-not-a-flag -c x.c -o x.o`; compare rc with GCC.
**Plan:** retain strict -f fallback and explicit refused/no-op rationales;
mechanically inventory all existing stateful -f/-m/-W/-O/-std arms; classify
repo flag tokens without accepting typos; generate options documentation from
one authoritative registry. Audit prefix catch-alls, value validation and
option ordering. Unknown -Wno-* requires GCC-compatible delayed diagnostics,
not blanket rejection. Do not add an ignore escape hatch without a tested,
explicit warning contract.
**Gates:** existing driver unit/end-to-end gates; add repo-token inventory,
artifact/semantic checks per implemented family, mutation verification and
50-flag autoconf smoke list. Test builtin aliases through sema, lowering,
post-inline strlen folding, math simplification and purity inference.
**Accept:** every supported spelling is classified; unknown tokens fail
independently of -v order; sanitizer/LTO requests either produce the promised
artifact or fail clearly. Full registry/inventory remains OPEN.

### MB-02 · P0 · S — linker contract and hardening effects

**Why:** permissive options can promise absent output properties.
**Where:** `src/bin/lccc_ld.rs`, `src/backend/linker_common/args.rs`.
**Reproduce:** link a hello-world with `-Wl,-z,pack-relative-relocs` and inspect
`readelf -d`; do not assume the historical no-RELR result still holds.
**Plan:** inventory current parser paths, classify supported/refused/no-op
families; implement fatal-warnings propagation; test image-base/page-size/
binary-output effects before deciding whether to implement or refuse.
Do not demand every vendor's entire help universe be supported.
**Gates:** structural readelf checks plus runtime/exit comparisons against
pinned BFD/lld/mold; malformed/unknown argument and warning-to-error controls.
**Accept:** no silent false promise in the curated distribution flag sets.

### MB-03 · P1 · S — CPU runtime probe residuals, not target folding

**Why:** the proposed Raptor-Lake-table repair is obsolete on this base.
Runtime probes already exist. Compile-target permission is not executing-CPU
capability, even for an enabled ISA feature.
**Where:** [runtime model](engineering/cpu-supports-runtime-model.md),
`src/common/cpu_probe.rs`.
**Plan:** independently review initialization races, feature/xstate coverage,
freestanding and IFUNC constraints; reproduce any defect before editing.
Do not restore constant true answers based on -march.
**Gates:** `bash tests/regression/check_cpu_supports_runtime.sh`, name census,
restricted-CPU QEMU and concurrent first-call controls where applicable.
**Accept:** no false capability positives; initialization and ABI justified.

### MB-04 · P1 · H — explicit PIE/static-PIE/build-id policy

**Why:** defaults differ between distribution toolchains; PIE-by-default is a
product policy, not a universal C-compiler requirement.
**Where:** driver startup selection and native linker layout.
**Reproduce:** `readelf -h -l -d a.out` after default, -pie and -static-pie links.
**Plan:** test current behavior first; refuse unsupported static-pie rather
than produce ET_EXEC. Decide default PIE/build-id separately, preserving kernel,
custom-script, freestanding and target-specific behavior.
**Gates:** relocation/textrel inspection, runtime relocated pointers, startup,
IFUNC and unwind checks on both x86 targets; deterministic build-id controls.
**Accept:** documented defaults and no silently substituted output mode.

### MB-05/06 · P1 · S — stack-clash probes and stack-protector canaries

**Why:** refusal is safer than unimplemented hardening but does not satisfy
distribution builds. These are separate implementation changes.
**Where:** x86/i686 prologues, dynamic allocation, returns, CFI and stack layout.
**Reproduce:** compile escaped 8-KiB/1-MiB frames and VLA, inspect probes;
compile an escaped character buffer and inspect canary/check/failure call.
**Plan:** derive probing/guard-gap and object-layout invariants from ABI/OS
requirements. The proposed blanket 1-KiB exemption and array ordering are not
accepted as proofs. Cover dynamic allocation, tail calls and guard options.
**Gates:** guard-page runtime test, smash test, assembly/CFI checks and mutation
controls on x86-64/i686. Do not claim deployment security from assembly alone.
**Accept:** requested hardening is present and effective before changing refusal.

### MB-07 · P1 · S/R-historical — semantic and ABI flag families

**Why:** visibility, GOT-indirect calls, trapping overflow, TLS model and
initialization/clearing policies must be observable or refused.
**Where:** driver, symbol emission, calls, arithmetic lowering and returns.
**Reproduce:** inspect hidden symbol with `readelf -sW`; inspect no-PLT calls;
run signed-overflow trap and zero-init witnesses.
**Plan:** split visibility, no-PLT, trapv, UBSan, TLS and register/automatic
clearing into independent patches. Trap UBSan still requires enabled sanitizer
checks; -fsanitize-trap alone is not instrumentation. Re-run archived wrapv
product/address witness before asserting a current miscompile.
**Gates:** flag × attribute × pragma, runtime ELF bindings, boundary/trap tests;
retain wrap-domain and historical product/address reproducers.
**Accept:** each row has semantic evidence or a clear refusal; no old JSON
capture is relabeled as a current failure.

### MB-10/11/12 · P1 · S/R-historical — debug, language and warnings

**Why:** useful DWARF, correct C23 and enforceable diagnostics are daily-user
contracts. The supplied 40-snippet/zero-debug baseline is not revalidated here.
**Where:** native assemblers, frontend, diagnostic registry.
**Reproduce:** -g object readelf/addr2line/gdb; C23 per-feature snippets;
`printf(argv[0])` under -Werror=format-security.
**Plan:** first inventory existing functionality; add line/DIE/locations stages,
positive/negative C23 semantic tests, and warning groups with false-positive
controls. Do not equate accepting a keyword with full semantics. _Float16
extension/ABI support and __bf16 are separate from C23 mandatory coverage.
**Gates:** object debug relocations and gdb args/locals/types, dialect matrix
against GCC16, format and warning precision on real workloads.
**Accept:** independently scoped contracts, not a fabricated 40/40 census.

### MB-13/14/15/16 · P1/P2 · S — linker parity, RELR, whole program, IFUNC

**Why:** these require real symbol/relocation/runtime semantics, not CLI flags.
**Where:** i686/x86 linkers and shared linker helpers, frontend clone metadata.
**Reproduce:** curated ELF32 option links, relocated pointer-rich PIE, cross-TU
inline witness with colliding static names, readelf IFUNC plus dispatch.
**Plan:** re-inventory current implementations; land each relocation/option
feature independently; LTO remains refused until producer and consumer exist.
Do not treat a warned no-op LTO as completion. Separate RELR format from
loader-specific GLIBC_ABI_DT_RELR policy; avoid unsupported universal size targets.
**Gates:** current structural/runtime linker differentials, static/dynamic TLS,
IFUNC, COMDAT, symbol versioning and cross-TU correctness.
**Accept:** curated unsupported rows eliminated with observable behavior.

### MB-20/21/22 · P1/P2 · H — compiler-work and peephole cost

**Why:** historical Callgrind concentrations identify candidates, not current
bottlenecks or predicted gains. Fastbuild ratios are not release-speed claims.
**Where:** peephole line decoding/liveness and compiler profiling harness.
**Reproduce:** `python3 scripts/callgrind_compiler.py --help`; choose fixed
40-/300-function inputs, freeze both compiler hashes and identical geometry.
**Plan:** measure first; prefilter allocations, then consider decoded caches
with invalidation proofs. Share liveness only across unchanged regions. Do not
change -O0 optimization policy based only on text size.
**Gates:** byte identity where behavior is intended unchanged; run-and-compare,
paired native times and Callgrind compiler-work; self/control profiles.
**Accept:** repeatable reduction without semantic/identity regressions; 30%/50%
shares are aspirational, not justified requirements.

### MB-30/32/33 · P1/P2 · H — calibrated performance and differential coverage

**Why:** published microarchitecture tables and historical small corpora are
inputs, not end-to-end speed proofs.
**Where:** performance catalog, benchmark and linker differential harnesses.
**Reproduce:** existing corpus and paired/Callgrind tools under recorded flags.
**Plan:** recover and commit missing i686 witnesses before claiming 14-program
coverage; add fixed expected results and GCC multilib comparisons. Use uops data
with version/licensing/provenance, validate cost-model changes empirically.
**Gates:** runtime/exit/checksum checks; linker semantic property allowlists;
codec workloads and >=200-ms paired arms before small-delta claims.
**Accept:** no claimed universal win; report per-workload gains and regressions.

### MB-40/41/42 · P1/P2 · S — evidence and queue integrity

**Why:** a misleading census, malformed report or stale backlog defeats review.
**Where:** audit census, docs/link gates, benchmark publication/probe scripts.
**Reproduce:** production after cfg(test), malformed function spans, incomplete
sample records and deliberately broken links.
**Plan:** scope-aware census and malformed-span errors; mechanical backlog
format/path checks; promote probes with versioned expected outcomes and complete
failure records. Do not mark unknown/unavailable rows successful.
**Gates:** adversarial scanner/report tests, current doc-link gate, mutation
checks and nonempty expected inventory.
**Accept:** reported coverage is reproducible and includes skips/failures.

## Carry-forward IDs (open until independently revalidated)

Full reproducers/plans remain in the unchanged archive; demotion is not closure.

- Evidence/infrastructure: A14-01/04/06/07/22/23/26/27/28/32/33,
  INF-HARNESS-1, INF-BENCHGATE-1, INF-FRESHCLONE-1, INF-GAS-1, INF-LINK-1,
  INF-PGOPID-1, INF-M32HDRS-1; PR834 measurement-boundary/hosted-cohort work.
- Correctness/architecture: A14-08/09/10/11/12/13/15/17/18/31/35,
  ANON-KEY-1, PARSER-ALIGN-1, CONST-QUAL-1, INTPTR-1, REDEF-1, IVSR-OFFSET-1;
  frontend/zstd/fusion residuals require new baseline witnesses.
- Performance: RA-GLA-04, RA-CSAVE-1, RA-PRESSURE-3/4, PF-SN-1, PF-LZ4-1,
  PF-MB-1, PF-FB-1, PF-CHACHA-1, PF-SCHEDULER-1, PF-CLS-1, PERF-1/2/4/5/6,
  SETMEM-SHAPE-1, RANGEFOLD-OR-1, CONSTLOOP-FOLD-1, WR-COND-RETEST,
  MI-CLOBBER-1/MI-XMM-1/MI-PARAM-1/MI-ROTATE-1/MI-ENCODE-1,
  IVOPTS-1, LOOP-PREHEADER-3, EDG-TRANSPLANT.
- A14-02/03 repairs have upstream source evidence; do not reimplement the old
  descriptions. Re-run adversarial sample/layout controls before final closure.
- METRIC-TRAP-1/METRIC-SPLIT-1 remain methodological guardrails, not code tasks.

## Programme completion

A curated Arch/Debian hardening contract must have zero unsupported/false-success
rows before declaring distribution completeness. Debug, language, ABI and
performance acceptance remain separate. Report measured rows and boundaries,
never a single perfection score.
