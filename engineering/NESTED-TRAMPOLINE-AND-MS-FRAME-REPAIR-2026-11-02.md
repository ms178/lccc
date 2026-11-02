# Follow-up: trampoline liveness, Microsoft leaf frames and portable mask gate

This follows `a53d98d33` and the merge of main `ac28d3a24` in `ac372dc40`.
It does not complete the warning/options/full-current-kernel acceptance task.

## 1. Live incoming parameters across trampoline construction

The earlier supplementary x86-64 nested-capture witness was not restricted to
Microsoft ABI: removing that attribute also reproduced the failure with SysV.
At `-O2/-Os`, two enclosing invocations prevented constant specialization from
masking it; GCC 16.2 returned zero. The historical source remains at
`engineering/reproducers/msabi-nested-capture-pending.c` (the filename records
its discovery state, not a permanently failing or disabled test).

The IR kept a parameter live across two `InitTrampoline` instructions before
copying it into a loop-carried value. The x86 emitter stages the trampoline
through RDI/RSI/RCX, but liveness did not model those clobbers. Keeping the
parameter in its incoming RDI home therefore read the buffer address instead
of the original integer. This explains why a constant-specialized single call
could conceal the problem.

`instruction_is_call_point` is the existing register-clobber abstraction, not
an instruction-emission command. Classifying `InitTrampoline` there protects
live caller-saved homes **without emitting a fictitious CALL**. A negative unit
control keeps `GetStaticChain` classified as a read, not this broad barrier.
This is deliberately conservative: introducing architecture-specific physical
clobber masks would require a separate complete emitter audit. No speedup is
claimed for making previously wrong code correct.

The new permanent fixture runs on all four LCCC targets, additionally with
Microsoft x86-64 and GNU fastcall i386 conventions; GCC 16.2 runs the native
SysV/Microsoft variants. Every variant requests O0/O2/Os and both frame-pointer
modes: **48 added object/runtime executions**. The Microsoft and SysV discovery
witnesses also returned zero at all three levels after the liveness repair.
Capturing GNU nested trampolines require executable stack memory. The native
linkers' exact executable-stack notice is validated only for this fixture;
other diagnostics still fail. No linker-security warning switch was disabled.

## 2. Microsoft leaf-function XMM saves need allocated storage

The audit also found a separate ABI/frame defect. Empty-local-frame elision
returned zero when the only reserved bytes were callee saves, assuming those
bytes were all represented by pushes. That assumption is valid for GP saves,
not for the Microsoft XMM6–XMM15 save region.

A noinline Microsoft leaf that explicitly clobbers XMM6 reproduced an actual
out-of-frame write: at O2 the emitted stack was only **24 bytes** below RBP but
`movdqu %xmm6,-48(%rbp)` wrote below it. The new final-assembly contract failed
on the pre-fix binary. Preserving register values in an unallocated region is
not a correct ABI implementation, even when an ordinary smoke test survives.

Elision now additionally requires that the reserve equal the bytes represented
by GP pushes. Microsoft SSE save storage remains allocated. The new O0/O2/Os
contracts walk the emitted stack depth and prove every nonvolatile XMM save
lies inside the allocated frame; XMM6 must be saved because the fixture really
clobbers it. **Three added frame contracts.** This fixes correctness, not a
claimed performance improvement. Narrowing the conservatively preserved XMM
set is separate work.

## 3. Byte-mask gate: remove libc/multilib coupling without dropping coverage

Fresh-main validation passed 7,671 library tests and the 281-contract cohort,
plus the parity and CLZ gates, but the new byte-mask gate stopped at missing
32-bit libc headers on this sandbox. Treating that as a passing skip would not
satisfy the requested architecture coverage.

The fixture is now freestanding in both modes. It preserves every original
probe and its complete decimal hash, implements only its small output routine
using Linux user-space write syscalls, and links with explicit startup assembly.
GCC compiles **only this standalone non-kernel oracle**. Separate 32/64-bit
reference hashes are still compared at O0/O1/O2/O3/Os. QEMU-i386 is used when
available, otherwise actual native execution is required. Missing GCC, missing
execution, malformed output and compiler/link failures are errors, never
successful skips. The original instruction-shape/sign-reader checks remain.
The standalone repaired gate passes with GCC 16.2; no old compiler or warning
suppression was substituted.

## Verification status

Final combined verification is recorded separately after the exact working
tree completes library tests, fastbuild binaries, the expanded contracts,
parity/CLZ/mask gates, DWARF round-trip, all-target warnings-denied Clippy and
strict authentic usercopy validation. Intermediate passes above must not be
misrepresented as a current-source full Linux boot, zero-warning kernel or
all-current-head green hosted CI. The broader warning/option and mitigation
inventories remain open.

## 4. Actual hosted red check: hold the audit ratchet, do not raise it

Head ac372dc40's fast-contract shard b (run 41514827449, job 125518456323)
failed the census: panic sites 879→882, functions over 300 lines 232→233,
and over 500 lines 97→98. The dependent Test Suite check also failed. Expected
negative-test output in that job is not itself evidence of additional failures.

The repair leaves the baseline unchanged. One checked static-chain-home
accessor replaces three duplicated expects; destructuring the indirect target
proves its presence without the pre-existing target expect. Frame-chain storage
allocation and indirect result representation are extracted as separate,
meaningful helpers. The original invariants still fail closed; no check was
removed, assertion substituted to evade the scanner, or baseline raised.
The exact census check now passes at 879 / 232 / 97, with all other metrics
unchanged against main ac28.

## Verified ac28 follow-up cohort

After the final refactoring, the serialized current-source pipeline completed
successfully (logs `/home/user/final-cohort-*.log`):

* 7,672 library tests pass, zero fail, 10 existing ignores (14.74 s execution);
* all fastbuild binaries pass (1m02s), active 4 GiB swap;
* **332 object/runtime/frame contracts pass**;
* mandatory 32/64-bit byte-mask full-hash execution, parity and CLZ gates pass;
* expanded four-target DWARF 2/3/4/5/GCC 16.2 round-trip passes;
* all-target Clippy `-D warnings` passes (43.12 s);
* strict authentic saved-command usercopy/configured objtool passes;
* unchanged audit census holds.

Compiled identities (before a subsequent upstream merge):

* lccc: `0fb4616fd0da0e2533462d2aa1de388406d3f0a2b88b3d70df41c6f6c7dbeec5`
* lccc-i686: `cde15f33aa697d3cdf0252865811550df7fea98c3d16ed85ef565da6ffbcdb0f`
* lccc-ld: `8a6e01c238e70d9ac811e998a0c733b48e6b809b708824f7588e064e44854537`

A fresh fetch then advanced upstream again to **1dd907def** (preprocessor/time
builtins and conditional-carrier changes). The ac28 results above are not
silently attributed to that newer source: integrate it and repeat the relevant
gates before claiming a post-merge pass. Full current local kernel and QEMU,
all warning/option families and current-head hosted CI remain acceptance gates.
