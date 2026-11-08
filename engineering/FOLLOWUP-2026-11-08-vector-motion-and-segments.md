# SIMD store motion and segment-relative writes

## Two additional IR-level counterexamples

A nonvolatile FS/GS-relative `Store` can address the ordinary vector source
or the destination slot even when its numeric pointer is rooted in a different
alloca: the segment base changes the effective address. The load forwarder
previously used that numeric root as a disjointness proof. A focused test for
both segment bases yielded `[1, 1]` forwards before the guard, versus `[0, 0]`
required. It now clears available loads on every FS/GS store. Ordinary,
bounded, direct distinct-root writes still have a forwarding positive control.

Moving a vector producer's destination write from a later `Memcpy` to an earlier
intrinsic is unsafe across an unknown load or store: a missing edge in the
address-influence graph does not prove disjointness from the destination. A
fixed numeric pointer, for instance, can denote a stack address. A focused
read/write matrix produced `[1, 1]` promotions before the guard, versus
`[0, 0]` required. Store motion now crosses only an explicit allowlist of
nontrapping, non-memory IR operations or ordinary nonvolatile scalar accesses
directly to a different alloca with a nonzero in-bounds access size. Unknown
memory effects, atomics, calls, inline asm, segment-relative and volatile
accesses block it. The bounded direct-alloca positive case and oversize,
segment and volatile negative cases are tested. These are IR proof gaps; no
LCCC C-level misscompile is claimed.

Two further edge cases in the motion allowlist need conservative treatment.
An alloca marked `semantic_volatile` may be externally observable even when
an individual load or store does not carry its own volatile flag; an IR test
saw one promotion before the alloca-level guard, instead of zero. A distinct
alloca marked only internally `volatile`, but not `semantic_volatile`, is a
positive control. Also, an i686 x87 long-double access can potentially touch
16 bytes although `F128.size()` is 12. The 12-byte-allocation test likewise
saw one promotion instead of zero before replacing the size comparison with
`access_extent`'s possible-access bound; a 16-byte allocation is the positive
control. These are conservative IR-level checks, not demonstrated C-level
miscompilations.

## Evidence and trade-offs

On `cb89d48`, fastbuild, census and diff checks passed, as did the vector-temp
suite (53 tests), three targeted large-snapshot tests, the 485-case mask-facts
three-way LCCC/GCC 16.2/Clang 23.1.5 matrix and its pass-off mutation, 72 AVX
FP build/runtime/frame contracts, and ten AArch64/RV64 QEMU snapshot executions.
The new C integer-address fixture passed the regression runner and its A/B
mode. The authenticated hosted ICX 2026.2 lane on `2020ab8` also passed its
new four-compiler AVX2 C-fixture comparison (step 15 succeeded, with no AVX2
skip annotation); that is not a differential execution of the IR-only cases.

A before/after `-O2` census reported the same instruction, register-move,
stack-reference, push and accumulator counts in 316/316 measured functions
from 114 files (one AVX file was skipped on both sides without AVX flags). The
skipped `avx_fp_private.c` was rerun separately with `-mavx2`: its measured
function had 157 instructions and 66 stack references both before and after;
the whole assembly was byte-identical. Twenty-four kernel assembly files were
also identical. These samples do not rule out other missed optimizations.

The same AVX workload was timed on one pinned vCPU of a KVM Intel Xeon VM,
using 16 randomized four-way repetitions, two million loop iterations and the
same checksum. The raw wall-time samples are in
`engineering/vector-motion-avx-runtime.json`. Median seconds: old LCCC
0.212550, new LCCC 0.208547, GCC 16.2 0.030247, Clang 23.1.5 0.012211.
New/old median ratio 0.981; paired median difference -0.004009 seconds, with
overlapping ranges and byte-identical old/new assembly, so no speedup is
claimed. New/GCC ratio 6.895 and new/Clang ratio 17.078 expose a serious
workload-specific gap; VM scheduling and one benchmark limit generalization.
The raw register/home roundtrips remain open and warrant an independent,
correctness-proven optimization rather than weakening the alias barriers.

A separate experimental GS-base C example using `rdgsbase/wrgsbase` ran
correctly with LCCC and GCC 16.2 at O0/O1/O2/O3/Os. Clang 23.1.5 returned the
expected result only at O0 and a different result at the four optimized levels.
That example did not trigger the LCCC IR counterexample; its address-space
semantics and Clang divergence need further independent investigation, so it is
not used as a three-way correctness oracle or a claimed fixed C regression.

## Integration and remaining work

The previously tested code tree `4526de986e058df34ef2ebe1bc763109d2f65171`
passed all executed hosted checks at commit `c566734`; three optional jobs were
skipped. Its commit messages were subsequently rewritten without changing that
code tree, so the rewritten head requires its own hosted CI result. In line
with the current request, no new local CI run is required or authorized; the
interrupted local runs are not counted as passes. The snapshot publisher's
local-gate policy means the saved patch is explicitly **UNGATED**, not a
same-tree locally gated delivery.

PR #1145 independently addresses the measured AVX-512 mask-compare HOME
roundtrip and also modifies `vector_temp_promotion.rs` and the CI workflow.
A `git merge-tree` check of its head `8430e1a4` with the rewritten head
`039abb4d` was conflict-free. This checks text integration only: the combined
code and performance still need CI and independent verification after either
PR lands. PR #1146 covers peephole allocations and #1141 covers a separate SIMD
audit; neither shares a changed file with this patch at the inspected heads.

Hardware-vendor expert approval, general freedom from defects, and global
optimality have not been established. The broader backlog, segment and
provenance audit, dynamic memory/concurrency interop on physical AArch64/RV64,
and workload-specific performance gap remain open.
