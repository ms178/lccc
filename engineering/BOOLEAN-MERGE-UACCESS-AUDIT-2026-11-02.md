# Boolean-merge/UACCESS audit — focused repair, full acceptance still pending

## Root cause and decision

In actual lib/usercopy.c, check_zeroed_user's inlined access-begin helper
returns a 0/1 phi. A residual integer cast separates that phi from the branch.
The existing phi-only threader rejects the block. Its emitted code therefore
sets the success result after STAC, rejoins and tests that constant again.
Objtool cannot correlate the register value with the predecessor, and reports
a return with UACCESS enabled on an infeasible failure path. The warning also
reproduces with GNU as assembling LCCC's output, excluding an assembler-only
explanation. This is not permission to suppress the validator.

Removing arbitrary narrowing casts would be wrong: 256 narrows to u8 zero.
Instead, a canonicalization proves **every incoming phi arm is exactly integer
0 or 1**, both phi and cast types are integers, and the cast's sole use is the
branch. Only then is the cast removed. This exposes the existing checked
threader; no asm, memory access or state change is cloned, moved or annotated
away. Other phi/cast uses, nonconstant arms, floating/pointer casts and empty
phis are not covered by this proof and remain unchanged.

A backend text rewrite cannot establish all predecessor values and downstream
uses. General path cloning costs code size and must model side effects. The
local value proof is narrower: no new blocks/instructions, and no liveness
repair. Candidate discovery allocates nothing when no candidate exists. A
single function-wide saturated use-count pass runs only for actual candidates,
rather than rescanning the function per cast. The existing bounded threader
and cleanup own the subsequent CFG changes. This is not a claim that the
entire optimizer is maximally efficient.

## Executed checks

* **13 focused IR tests PASS**, including all 100 integer width/signedness
  pairs, source-span retention, both constant edges, 256/-256/2 truncation
  counterexamples, an i128 high-bit counterexample, unknown SSA inputs,
  extra cast-result uses, and floating/pointer/contradictory metadata guards.
  The new contradictory-phi negative test initially failed; adding the
  integer-phi guard made the final suite pass. No assertion was removed.
* Normal optimized fastbuild PASS (23.04 s); all-target Clippy -Dwarnings PASS
  (35.02 s). Lib-only unit harness uses fastbuild opt-level/debug 0; actual
  compiler binaries use normal optimized fastbuild. Active swap remains 4 GiB.
* **233 object/runtime contracts PASS**: all four LCCC targets and native
  GCC 16.2.0 / Clang 23.1.4. New standalone, non-kernel tests execute observable
  state changes through casted boolean merges and reject unsafe u8/u16
  truncation assumptions, at O0/O2/Os on every target/oracle.
* Backend + assembler units: x86 **2433 PASS / 5 existing ignores**, i686
  **622 PASS / 1 existing ignore**, AArch64 **254 PASS / 4 existing ignores**,
  RV64 **184 PASS / 0 ignored**. Ignored tests are not counted as passes.
* CI/local parity remains 512 commands / 27 invocation contracts. The existing
  required authenticated ICX 2026.2 native lane receives these added fixtures
  when pushed; local ICX is absent, so no local vendor pass is claimed.

## Actual kernel object and bounded native measurement

Using the exact saved kernel lib/usercopy.o command, two compiler binaries
before/after this repair compiled the same patched 6.18.58 source with LCCC.
No GCC kernel-target compilation occurred. check_zeroed_user shrank from
**611 to 591 bytes (20 bytes / 3.27%)**. Its UACCESS-return finding disappeared.
**Strict objtool still FAILS** that object on two other dead-helper findings:
outlined stac's enabled return and outlined clac's redundant disable. Those
require the separate assembly-symbol-retention repair; this is not a full
objtool pass.

`tests/benchmark/run_kernel_usercopy_replay.py` links those LCCC-built objects
into a native freestanding userspace harness, with identical harness code and
a test-only return thunk. Every run verifies **4488** aligned/unaligned,
length-0..32, zero/nonzero and outside-range-byte cases before timing 10 million
calls. The shared read buffers are valid; this does **not** exercise kernel
fault recovery, patched privileged SMAP, scheduler behavior or whole-kernel
runtime. No GCC compiles the objects or harness.

Final recorded paired screen on the VM-reported Intel Xeon 2.60 GHz CPU:
3 warmup pairs, 21 alternating-order sample pairs, pinned logical CPU,
median **79,520,174 ns before / 78,275,324 ns after** per 10 million calls;
ratio **0.98434548** (about **1.57% lower median workload time**), faster in
18/21 pairs. Two earlier screens were also about 1.6% lower. Raw samples,
object/compiler/harness hashes and measurement limits are in
`engineering/measurements/2026-11-02-usercopy-replay.json`. The bootstrap is
exploratory: VM scheduling/frequency noise and correlated samples remain.
This is a small measured kernel-function replay improvement, not a promise
of a whole-kernel or bare-metal speedup.

Reproduce with the saved before/after kernel objects:

```sh
python3 tests/benchmark/run_kernel_usercopy_replay.py \
  --before-object /path/to/before-usercopy.o \
  --after-object /path/to/after-usercopy.o \
  --lccc target/fastbuild/lccc --output /path/to/replay.json
```

## Remaining confirmed work

* Dead stac/clac bodies are retained because conservative asm word scanning
  mistakes instruction mnemonics for C-symbol references. No suppression added.
* i686 accepts -mindirect-branch=thunk-extern but still emits raw indirect calls.
  AArch64/RV64 also silently accept the x86-only request. These newly reproduced
  target/option defects require implementation or explicit target rejection.
* The recorded CS-prefix driver flag is not forwarded to emission. A standalone
  six-GP/variadic-SSE probe shows GCC 16.2 emitting the CS prefix for r10 while
  LCCC omits it. Protection option ordering/register-only forms also need tests.
* Remaining warning families/options and full-kernel strict objtool, new QEMU
  boots, current-head hosted CI and broader workload gains are still pending.
  Existing ignored backend optimization/architecture tests are not erased or
  represented as completed audit coverage. No expert approval is asserted.
