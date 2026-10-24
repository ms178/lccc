# IPA-PTA and live-range shrinkage

## Scope and pipeline contract

Both transformations are opt-in, including their `-fno-` forms, and are
last-option-wins. They run at `-O1` and above after the ordinary SSA pipeline,
inlining, and indirect-call promotion, but before live-range splitting and phi
elimination create position-dependent copy webs. `CCC_NO_IPA_PTA` and
`CCC_NO_LIVE_RANGE_SHRINKAGE` are compatibility kill switches.

The pipeline verifies the IR after either transformation changes it. Debug
source spans are permuted with scheduled instructions; a malformed parallel
span array disables scheduling for that block rather than associating a source
location with the wrong instruction.

## IPA-PTA design

`passes/ipa_pta.rs` implements a whole-module, flow-insensitive inclusion
solver. SSA values, formal parameters, returns, variadic arguments, and
abstract-object contents are graph nodes. Distinct allocas, allocation sites,
globals, constant-data objects, and functions have distinct abstract objects;
GEP offsets are intentionally ignored, so this is a may-alias analysis rather
than an unsafe field-sensitivity claim.

Copy edges propagate sorted points-to sets through a work queue. Load, store,
`memcpy`, and indirect-call constraints are watchers on pointer nodes and add
edges only when the corresponding object becomes possible. Direct calls bind
actuals to formals and returns to results. Indirect calls accept function
objects, retain a conservative external-call component for unknown targets,
and reject data-as-code solutions. Weak and semantically interposable bodies
are not treated as direct-call definitions.

Escape handling is part of the solver, not a post-hoc approximation:

* unknown calls can modify globals and escaped objects, but not an unescaped
  caller stack/heap object;
* storing a pointer in escaped memory escapes that pointer's pointees;
* escaped object contents and unknown memory are linked in both directions,
  including pointers loaded back from escaped storage;
* externally visible pointer parameters and variadic arguments start with
  unknown provenance, while pointees returned from externally visible bodies
  escape;
* integer-to-pointer casts add unknown provenance instead of fabricating a
  concrete object;
* unmodelled memory-affecting IR has an `all` effect and therefore cannot be
  used to prove that a call preserves a load. Acquire/release/seq-cst atomic
  operations and target fence intrinsics are also summary barriers; relaxed
  atomics retain object precision. TLS address materialization is conservatively
  treated as a possible helper call unless a target-specific proof says otherwise.

The mod/ref fixpoint propagates both modification and reference summaries over
recursive call graphs. The first consumer is late redundant-load forwarding:
it retains an available load across a call only when the call's complete
modification summary cannot affect that load's points-to set. Volatile,
address-space, type-width, multi-definition, and source-span guards in the
existing consumer remain unchanged. Unknown or exhausted analysis disables
forwarding everywhere; partial points-to information is never treated as a
non-alias proof.

The default budget is four million solver work units, one million unique
edges, and 8192 objects per points-to set. It bounds pathological compile-time
behavior and fails closed. Statistics
under `CCC_IPA_PTA_STATS=1` report graph size, escape count, work, and the
number of forwarded loads.

## Live-range shrinkage design

`passes/live_range_shrinkage.rs` is an SSA def-use DAG list scheduler inside
pure, barrier-delimited intra-block regions. Calls, memory operations, PHIs,
atomics, asm, stack manipulation, and target-ABI pseudo-operations are region
boundaries. The implementation does not move a memory access across another
operation or speculate an instruction across control flow.

The objective separately tracks GP and FP/vector pressure, including two-GP
64-bit values on i686. A ready-list candidate minimizes post-instruction excess
pressure, then worst-class excess, then prefers deaths and stable source order.
A candidate is committed only if it strictly improves excess area or peak
without increasing live area or peak in either class. Below-threshold regions
retain source order; rejected simulations have no IR effect.

Multi-cycle integer and floating-point arithmetic is also a hard region
boundary. The first sustained comparison showed a 16.12% regression from a
pressure-improving multiply DAG, and an experimental in-order latency proxy
still left a 17.62% regression on a later host sample. Rather than disguise
that result or treat an idealized issue score as a hardware model, the pass now
restricts reordering to unit-latency integer DAGs. Both the preserved
multi-cycle case and an independently scheduled unit-latency case have
sustained timing workloads; this is a measured eligibility boundary, not a
claim that all unit-latency schedules are universally profitable.

The budgets are architectural starting points (x86-64 14 GP/16 vector, i686
6/8, AArch64 28/32, RISC-V 27/32), not claims about a particular host CPU.
APX's additional GPRs are not assumed by this IR pass: backend eligibility and
calling-convention reservations must be proven before raising that budget.
The existing backend's APX-aware allocation remains independent.

`CCC_LR_SHRINK_STATS=1` reports eligible-region counts and excess-area
before/after. Multi-cycle boundaries are intentionally not counted as eligible
regions.

## Reference review

The design was informed by current upstream implementations, not by copying
their passes:

* [GCC optimize options](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html)
  and current `tree-ssa-structalias.cc`: whole-unit pointer and call
  use/clobber solving, escaped/nonlocal solutions, and recursive instances.
* Current GCC `sched-rgn.cc`, `haifa-sched.cc`, and `ira-lives.cc`: pre-RA
  pressure-guided scheduling and the distinction between scheduling and
  program-point compression. GCC PR114415's stack-pointer memory-dependence
  failure is why stack operations are hard boundaries here.
* [LLVM `LiveIntervals.cpp`](https://github.com/llvm/llvm-project/blob/main/llvm/lib/CodeGen/LiveIntervals.cpp):
  `shrinkToUses` rebuilds minimal segments from real uses. LLVM's 2026
  distance-maintenance work also motivates avoiding stale slot-index distances
  and quadratic updates; this pass uses SSA dependencies and a bounded region
  simulation instead.
* [Intel SDM and optimization manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html),
  version 093 / optimization volume version 050 (2026-10-09), and the
  [APX specification](https://cdrdv2-public.intel.com/861610/355828-007-intel-apx-spec.pdf):
  additional EGPRs and three-operand forms must follow actual allocator and
  ABI eligibility, not a flag-based assumption.
* [AMD64 Architecture Programmer's Manual](https://docs.amd.com/api/khub/documents/OdDzCKsU4uf~KbI1htdPvA/content),
  24592 rev 3.25 (May 2026), and the latest located
  [Zen 5 optimization guide](https://docs.amd.com/v/u/en-US/58455_1.00):
  register localization can reduce memory traffic, but the final measure is
  target-specific and cannot be inferred from register count alone.

## Validation and measurement

`tests/regression/check_ipa_pta_live_range.py` compiles and runs direct,
indirect, escaping, variadic, recursive, volatile-boundary, and high-pressure
cases. It compares baseline/feature output and strict available GCC 16.2,
Clang 23.1.3, and authenticated ICX 2026.2 oracles (the dedicated lane installs
GCC from the repository's exact-version cache), checks a positive IPA forwarding witness,
and records an interleaved median timing report. Both a positive IPA forwarding
witness and a positive changed-region LRS witness are required, so a silent
no-op cannot produce a green correctness run. Performance executables have
compiler-specific names and all available compilers are rotated through the
same interleaved timing loop; the report includes feature/baseline and
feature/oracle medians, min/max and median absolute deviation without treating
process-time noise as a universal win. The three performance fixtures execute
20 million call/load iterations, four million multi-cycle pressure-function
iterations, and four million unit-latency pressure-function iterations,
respectively; CI takes nine rotated samples per compiler so sub-millisecond
process startup does not dominate the measurement. ICX is explicitly run with
`-fp-model=strict`.

The Rust unit tests additionally pin fail-before/pass-after behavior for the
block-local consumer, recursive and variadic summaries, semantic interposition,
escape through globals, source-span preservation, and budget failure. The
harness writes JSON rather than making a universal superiority claim. A
performance claim requires the recorded workload, compiler identities, host,
repetitions, and spread; an unmeasured target or a noisy median is reported as
a remaining risk, not presented as a win.
