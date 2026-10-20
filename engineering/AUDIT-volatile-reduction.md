# Volatile reduction: adversarial engineering review

## Review boundary

This feature is an x86-64 O3 scalar transformation, not an architecture-independent
unroller. i686, AArch64 and RISC-V must not enter it. Signed arithmetic, floating
point, atomics, stores, extra reads, side exits, non-unit IVs and dynamic addresses
are outside its legality contract. No ISA extension, thread-local mutable pass
configuration, unsafe Rust, mutex or spinlock is introduced.

The review does not assert universal optimality, absence of undiscovered defects,
or approval by Intel or any other external engineer. It separates executable
checks, arithmetic arguments and machine-dependent measurements.

## Transformation proof and function audit

| Component | Obligation and implementation decision |
| --- | --- |
| `identity` | Accept only unsigned-width add/mul/and/or/xor. Seed one lane once; other lanes use 0, 1 or all-one bits. Unsupported operations have no identity in this pass. |
| `analyze` | Require two header phis, a top-tested unsigned comparison, one dedicated unconditional preheader, and exactly one body backedge. The body has one volatile load, one matching-width reduction and one unit IV increment, optionally preceded by one constant GEP. Reject all other shapes. |
| Invariance/escape checks | The pointer base, seed and limit cannot be loop definitions. Body results cannot be observed outside the original header/body. SSA-valid input then supplies dominance of the invariant operands at the preheader. |
| `rewrite` | Validate instruction variants and checked allocation budgets before mutation. Preserve the complete load, including segment, width and volatility. Emit optional GEPs only in guarded bodies, never on zero trips. Four reads remain four scalar reads in source order. |
| Bound | `m = n & ~3` is computed in the IV's unsigned width. Process complete groups and enter the original scalar loop at IV `m`. No speculative volatile read occurs on zero trips. |
| Phi wiring | Replace the original preheader phi edge with the combine edge. Generated main phis refer only to their preheader and latch. The scalar tail and original accumulator/IV names remain the live-out interface. |
| Overflow | For a successful main iteration, `iv <= m-4`, so `iv+4 <= m <= n <= UINT_MAX`. The tail's last increment is at most `n`. Unsigned reduction arithmetic is modular at its original width. |
| Source locations | Preserve pre-existing preheader spans, append locations for generated control, and associate cloned loads/GEPs/reductions/increments with their originals. Missing locations are represented by the repository's dummy span, not invented positions. |
| Fresh IDs | Scan actual mentioned SSA IDs and actual block labels, not only cached counters. Reserve exactly 19 values without GEP or 23 with GEP, plus three labels, using checked arithmetic before mutation. Exact exhaustion and stale-cache cases are tested. |
| Fixed factor | A compile-time assertion makes four lanes an explicit requirement. The balanced four-way combine is not presented as a generic configurable lane algorithm. |
| `dynamic_tls` | Declaration model overrides command-line model, which overrides output defaults. Known GD/LD addresses are currently unprofitable because the backend repeats descriptor calls; do not interleave them. LE/IE and ordinary pointers remain eligible. |
| `CostlyAddresses` | Build roots once per function and allocate alias metadata only when a costly root exists. Follow the backend's bounded constant-GEP/Copy chain. No whole-program alias guess is made. |
| `run` | Cheap volatile-load prefilter; rebuild CFG after mutation rather than use stale edges. The upstream 200-block and 32-rewrite budgets bound compile-time analysis work. Generated main loops have five phis and remainders no longer start at a constant zero IV, preventing recursive interleaving of either. |
| Pipeline | Resolve TLS policy at TU scope; no per-candidate environment reads. Retain first-iteration/O3/non-size/x86-64/unroll/disable gates. The existing timed-pass macro already verifies output, so the duplicate verification was removed. |

The 32-rewrite budget is a compile-time/profitability policy, not a claim to
transform every eligible loop in arbitrarily large functions. Four chains are a
measured compromise, not proof that no CPU or workload benefits from a different
factor. Avoiding a speculative backend-wide resolver-hoisting change keeps the
semantic and ABI review boundary small.

## Adversarial tests

- 3,040 interpreted operation/width/IV-width/seed/tail combinations use a changing
  load stream and compare the complete access trace, result and final IV.
- 90 additional cases cover constant offsets and generic/FS/GS segments.
- 160 terminal-state executions use the actual generated bound at high-bit U32
  and U64 limits, including unsigned maxima; no billion-step interpretation is
  needed. These are implementation tests, not merely a duplicate mask formula.
- Structural tests cover original source spans, observable IV live-outs, sibling
  loops with stale counters, a dedicated inner preheader inside a noneligible
  outer loop, allocator boundaries and fail-closed candidate variants.
- Fifty TLS policy combinations check declaration/TU/default precedence.
- Source-derived refusal cases cover atomics, signed and FP reductions,
  non-unit IVs, varying addresses, intermediate stores and two volatile reads.
- Native C checks 6,000 results per invocation through twelve noinline functions,
  plus runtime-opaque zero-trip null probes. Optimization/code-model/ISA and
  disabled controls check both values and the actual mechanism. Cross-backend
  assembly/IR checks prove that excluded architectures do not enter the pass.
- Dynamic TLS remains covered through dlopen. A GD/LD refusal is an intentional
  profitability result, not a silent loss of optimization coverage.
- The assembly checker uses explicit errors, anchored function boundaries and
  instruction-only matching. Empty/wrong/malformed assembly, duplicate symbols,
  comments/data and wrong offsets are tested with ordinary and optimized Python.
- Measurement helper tests cover child failure, warning rejection, timeouts,
  wrong reference versions, hashing and dispersion.

Actual production-source mutation experiments deliberately changed the group
mask, truncated the generated bound type, cleared cloned volatility and bypassed
the costly-TLS lookup. All four mutants failed the intended focused tests. The
original source was restored byte-for-byte and all sixteen focused tests passed.

## Additional defects exposed by the wider review

### Measurement driver effective type

The initial diagnostic driver supplied a U64 object to a U32 pointer kernel.
Opaque calls do not excuse violating C effective-type rules. It now owns two
separately typed objects and selects the correct address. Final measurements
must use that corrected driver. Clang 23.1.3 trapping UBSan passed 220 diagnostic
executions. A GCC ASan/UBSan link attempt could not find the cached toolchain's
runtime libraries; it is not counted as a sanitizer pass.

### Late x86 memory folding

The new driver exposed a real O2 `-g` code-quality discrepancy: two stack-load
folds disappeared, growing `.text` by six bytes. Direct assembly-level tests also
exposed previously unsafe behavior in the same implementation:

1. A pinned load could survive deletion while its consumer was rewritten into a
   second memory access, duplicating a protected access.
2. A 64-line liveness search treated budget exhaustion as proof of death and
   could delete a definition read later.
3. Byte/word stack loads were treated as full-register kills despite preserving
   upper bits; an address-dependent reload could read the very value called dead.
4. The relaxed push/pop scan did not account for the old stack pointer as an
   implicit input.

These are tested at the actual peephole-function boundary, not inferred from
benchmark timings. The repairs share a debug-transparent adjacency walk between
the fold and its prediction predicate, refuse protected edits, fail closed at
liveness-budget exhaustion, and recognize only full-width independent kills.
The budget counts semantic lines so debug attribution does not consume it.
Layout/unwind/control barriers, live values, narrow writes and frame-base reads
remain barriers. The full corpus debug/size parity gate must pass after repair.

## Performance evidence requirements

Use `scripts/measure_volatile_matrix.py` for short/long trips, five operations,
both widths, pointer/IE/GD TLS and a register-pressure context. It compares the
same candidate with the pass disabled, a two-chain alternative, and exact
GCC 16.2 / Clang 23.1.3 paths. An optional previous candidate supports paired
measurements. The driver verifies every call; internal timers exclude process
and loader startup. CPU affinity, exact commands, binary/source hashes, full
versions, environment, raw samples, medians and MAD/min/max are recorded. An
incomplete run is marked as such and the JSON is atomically published.

The long-loop script has equivalent provenance, reference-version and child
failure checks. Timing is diagnostic, never a noisy CI threshold. Shared-library
microbenchmarks are not a claim about cold TLS allocation, mixed application
traffic or every calling context. The native assembler still documents `.loc`
as parsed/ignored; IR-span tests and GAS line-table inspection must not be
misrepresented as native-object DWARF support.

The authenticated Intel CI job additionally builds O3 contracts and checks 480
ICX diagnostic executions. Local host ICX measurements are unavailable under the
sandbox's network policy; CI results must be checked, not assumed.

## Integration boundary

This follow-up starts on merged main, preserving its Regehr/YARPGen tooling,
constant-fold fixes, extension peepholes, 200-block/32-rewrite budgets and
lane-zero compatibility switch. The original four-chain optimization is already
upstream. The proposed short-trip bypass is intentionally omitted because prior
paired data showed medium-trip and U64 Initial-Exec regressions. Current-main
comparisons, not the old pre-feature baseline, determine incremental gains.

Prior mutation and sanitizer evidence above describes the original investigation;
it is not a claim that those experiments were rerun on this integration. Fresh
compiler, unit, Clippy, native regression and hosted results are reported separately.
Strict GCC 16.2 also exposed an old-style declaration in the existing TLS hot-loop
benchmark; `__thread volatile` removes it without suppressing the warning.
