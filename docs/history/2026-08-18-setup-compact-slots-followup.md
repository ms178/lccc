# Linux x86 setup-size work: rebased delivery and follow-up

Date: 2026-08-18

## Delivered state

The canonical cumulative patch is rebased onto `ms178/lccc` `main` at
`c2bfc66eeb55489018029564cc6fc0260e50007b` (`Update loop_memory_promote.rs`).
It contains the fully validated S01-S03 work:

1. the fastbuild executable-profile correction;
2. the post-inline size-policy implementation and regressions; and
3. transitive nested-loop inline-cost accounting, including its focused
   regression and documentation.

The last validated setup-object replay represented by this delivered source
reduced exact `.text` from the earlier 30,431-byte state to 29,838 bytes. The
transitive nested-loop change was previously validated with 339/339 LCCC-only
regressions, 853 Rust library tests (6 ignored), and 25/25 strict user-space
workloads. The canonical patch was regenerated after rebasing rather than
carrying an old context diff forward.

The genuine unmodified Linux setup linker limit is **not yet met**. The kernel
boot code and complete kernel are therefore not claimed to compile, link, or
boot in this delivery.

## Deferred compact-stack candidate

A later, uncommitted research branch produced the following preserved results:

- compact/default setup `.text`: 28,943 bytes;
- compact slots plus equal-width Tier-3 frequency placement and conservative
  same-block certified aliases: 28,887 bytes;
- eight setup objects smaller and none larger versus compact/default;
- structural i686 fixture: 92-byte frame and 1,905-byte `.text`;
- mixed-width safety fixture: exit status 0;
- complete x86-64 comparison against clean S03: 25/25 binaries, assembly
  files, outputs, and statuses byte-identical/equal.

That candidate also still had two local function regressions relative to clean
S03: `get_entry +1` byte and `console_init +6` bytes. The `console_init`
regression was traced to an uncanonicalized same-width signedness-only integer
cast (`I64 -> U64`) which prevented copy propagation and stack-slot
coalescing. A focused implementation was started and passed its cast tests; a
local rebuild reduced `console_init` from 699 to 653 bytes and its object from
1,981 to 1,896 bytes. It did not receive the complete required validation.

These compact-stack, placement, alias, if-conversion, and cast changes are
**not part of the rebased canonical patch**. Their uncommitted source state was
not retained across the harness/workspace reset, while their diagnostics and
results were only partially retained. Reconstructing code from result summaries
would violate the no-experiment/no-unvalidated-code requirement. They must be
recreated carefully from the design record and then validated as a separate
candidate in the next session.

## Required next-session sequence

1. Recreate the same-width integer-cast canonicalization first, with positive
   signedness-only tests and negative float/different-width tests. Require the
   full regression and paired workload gates, not only the promising local
   object result.
2. Recreate compact scalar-slot inference and its two i686 fixtures. Keep
   128-bit inference in synthetic Rust IR tests because i686 C rejects
   `unsigned __int128`.
3. Recreate equal-width Tier-3 frequency placement and 32-bit-only certified
   alias fallback elimination. Preserve exact x86-64 byte identity.
4. Eliminate `get_entry +1` before accepting the candidate. The known cause is
   displacement placement around the final `Select`: the selected value has
   two machine-level destination stores although the IR reference count treats
   it as one definition. Any weighting fix must be deterministic and validated
   globally; do not assume the obvious local correction is globally monotone.
5. Make the i686 if-conversion maximum-one policy deterministic and validate it
   independently from compact stack placement.
6. Replay all 24 setup objects and require no object or function regression,
   an aggregate no worse than 28,887 bytes, both i686 fixtures, 339/339 or the
   then-current full regression count, the Rust library suite, and all 25
   strict workloads.
7. Run pinned balanced AB/BA wall-clock measurements. This environment has no
   usable hardware PMU, so clearly label wall-clock and generated-code evidence
   as proxies rather than hardware-counter results.
8. Commit and snapshot each independently validated candidate immediately,
   updating the canonical patch, series, source archive, bundle, and snapshot
   ledger after each accepted step.
9. Resume function-attributed setup optimization only after the candidate is
   regression-free. The authentic linker script must remain unchanged and both
   `setup_sects <= 64` and `_end <= 0x8000` must be satisfied by the real image.

## Critical triage of the supplied GLM register-allocator audit

The audit is useful as a list of hypotheses, but it is not an implementation
specification and should not displace the immediate x86 setup-size work.
Several claims require direct source confirmation on the new upstream base.

### High-ROI findings worth verifying first

- A fixed, silently terminating liveness iteration cap would be a correctness
  risk if non-convergence can under-approximate live sets. Confirm the exact
  current code and termination behavior, then prefer a convergent worklist
  algorithm with a focused irreducible-CFG regression.
- Any forced physical-register assignment without overlap/interference checks
  is a correctness risk. Confirm the current AArch64 promoted-F64 path and add
  verification before changing allocation policy.
- Register-allocation verification that merely prints overlap failures is not
  an adequate correctness gate. An opt-in release verifier should fail hard;
  debug-by-default verification needs compile-time-cost measurement before it
  is made unconditional.
- Repeated environment parsing in allocator hot paths and repeated linear
  interval searches are plausible compile-time costs. They are lower priority
  than correctness and the setup blocker, and must be profiled before broad
  refactoring.

### Findings that are plausible but overstated or incomplete

- The audit's spill-traffic example is internally inconsistent. It describes
  current demotion as a store at definition plus a load at every use, then
  contrasts it with a proposed store at definition plus a reload at every use,
  while elsewhere claiming the current behavior performs a store and load at
  every use. Exact emitted code and value semantics must be measured before
  accepting its 3-5x or 40% projections.
- Segmented intervals are useful for CFG holes and location changes, but adding
  segments alone does not implement splitting. Split fragments still need
  distinct allocation identities/location ranges, edge moves, spill-slot
  ownership, and correct rewriting. In SSA, a value cannot simply become dead
  and later be reused along the same path without a use keeping it live.
- The proposed reload editor creates fresh SSA temporaries after allocation and
  says they “get their own live range.” That is circular unless allocation and
  liveness are rerun or the rewriter has guaranteed scratch-register
  constraints. This is a major missing design step.
- Briggs/George conservative coalescing is a graph-coloring result. Applying it
  directly to this linear-scan allocator requires an explicit interference
  graph and a cost/compile-time justification; it cannot simply replace the
  existing mechanisms by name.
- `SmallVec` is proposed despite LCCC's zero-dependency policy. An in-tree small
  storage representation or an explicit dependency-policy decision would be
  needed.
- The claim that `std::env::var` is “syscall-ish” is inaccurate on normal Rust
  targets. It can still involve synchronization, lookup, and allocation, so
  caching configuration is sensible, but the stated mechanism and promised
  speedup are unsupported.
- Deleting apparently dead allocator and stack-coloring modules should follow
  feature/configuration-aware call-graph checks and test coverage. Deletion is
  maintenance cleanup, not a high-ROI solution to the kernel size blocker.
- Numeric performance targets, month estimates, and claims that a rejected
  eviction mode will become profitable are speculative until supported by
  paired measurements on this compiler and workload corpus.

### Priority decision

Next session should first restore and fully validate the compact i686 candidate,
because it has direct measured relevance to the authentic Linux setup blocker.
In parallel only with that work, verify the two potentially critical allocator
correctness hypotheses (liveness convergence and forced-register overlap).
Large segmented-interval, spill-rewrite, coalescer, MachInst-deletion, and
configuration redesigns should be separately designed and benchmarked after
correctness fixes and after the current kernel milestone is preserved in a
snapshot.
