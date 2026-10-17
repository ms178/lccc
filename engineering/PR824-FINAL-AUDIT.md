# PR824 independent red-team audit and performance work

PR under review: head `70da244326b9996f614396d79da66d943d4cd9b9`.
Starting main: `3ff36e8592b0d0e26645bf37d6685395b1a6eca5`.
Prior shared-binding work retained; integration is not semantic approval.

## Acceptance strategy

1. Preserve prior delivery before integration; inspect actual PR/API/reviews,
   source changes and prior failed/rejected experiments. No reviewer approvals
   existed in the retrieved reviews/comments. Do not invent expert approval.
2. Map changed compiler functions to legality/aliasing/architectural-state
   obligations and their actual tests, independently of historical pass claims.
   Preserve correct refusal paths and ISA availability checks.
3. Independently built main is the causal baseline. Build candidate using only
   fastbuild/O1/j2 with active swap. Reproduce the fifteen highest supplied
   paired-median ratios. They identify priorities, not proven causes or hardware
   speedups. Treat shared-source workload variants with registry macro flags.
4. Compare source/IR/assembly and correctness against GCC16.2/Clang23.1 and the
   actual ICX CE channel. Retain native and instrumented output checks, tool and
   source identity, raw Callgrind counters, failures and rejected alternatives.
5. Add falsifiable regressions before fixes; consider narrower correctness
   repairs and profitability guards before generic transformations. Cross-target
   semantics must not inherit x86-only profitability assumptions.
6. Autosave each validated change. Freeze all source/test/documentation before
   final rustfmt, strict Clippy and fast-only CI. Never run local slow/full CI.
   Recheck main before delivery; rebase and revalidate if it has advanced.

## Current status

Integrated for testing, not approved. Previous PR824 fast failures, ignored
cases, heuristic census limits and compiler-work regressions are not erased.
No assertion that every function is optimal, all bugs are gone, or a VM proves
EPYC/14700KF superiority. Results and explicit remaining limits follow below.

## A1 — nontrailing AVX2 gather mask writes

Confirmed: the unmodified PR's direct copy-propagation test failed at
`vgatherdps` after substituting the original source for the copied mask. All
8 gather families have a read/write vector mask, not a source-only operand.
The shared effect predicate is now a local barrier for these operations,
protecting both copy relations and register-backed spill homes. Existing
explicit-input operations remain eligible; no whole-function ban was added.
Full-pipeline and direct tests retain active gather instructions. Native
original/illegal-twin pairs all produce 1/0; Memcheck reports 0 errors/leaks.
No automatically emitted C miscompile is asserted: the current emitter does
not generate gathers. The original all-target test attempt hit SIGKILL while
compiling regular and test libraries concurrently; the isolated --lib run
retained fastbuild/O1/j2, active swap and the established warning/link flags,
and reproduced the expected assertion failure. The failure is retained, not
counted as an acceptance pass.

A1 validation: full Rust library suite **4715 PASS / 0 FAIL / 8 ignored**,
including direct gather propagation, full pipeline and eight spill-mask cases.
The 8 ignored tests retain their existing status. Native original/twin and
Memcheck checks passed. This surgical acceptance does not certify final CI.

## P1 — unsigned affine guard canonicalization

The actual byte-load hot loop already has a 32-bit load. Its unsigned
`i + 8 <= 4096` header costs an extra affine temporary and obscures canonical
loop analysis. `loop_bound` proves a monotone, finite trajectory in u128,
including the final failing header observation, then compares `i <= 4088`.
No signed-overflow theorem is used. Negative/dynamic/nonrepresentable bounds,
width mismatches, non-SSA, unsupported latch/CFG forms and possible wrapping
are refused without mutation. IDs, CFG edges and source spans are unchanged.
Four Rust tests pass, including exhaustive four-bit modular trajectories,
U64 carrier boundaries, widths/orientations, SSA verification and refusal
identity. The checked 15-workload causal screen has no output failures:
byte-load reassembly simulated Ir ratio **0.8750**, XML scan **0.9900**;
other thirteen round to 1.0000 versus the gather-fixed PR. The experiment
also places the existing byte-fold plan before inlining: isolated placement
only yielded 0.9993 on byte-load and 1.0000 elsewhere, so this is not called a
substantial standalone win. That placement's compile-time cost remains to be
judged. The unsigned guard itself applies only to x86-64 throughput profiles;
`CCC_DISABLE_PASSES=loop_bound` is the causal ablation.

These are bounded whole-program Callgrind counts, not native speedups. Inputs
and raw counters/manifests are retained under the evidence directory. The
original supplied EPYC screen's timings/flags are not silently replicated.

## P2 — honor the explicit signed-wrapping language contract

`-fwrapv` was ignored. `p[i*4]` at i=1073741824 therefore crashed after
signed-IV widening, although the defined 32-bit products are 0,4,8,12.
Disabling IV widening repaired the witness; disabling IV strength reduction
alone did not. The three other domain probes were already correct before
the fix and are coverage, NOT three additional reproduced defects.

The driver now carries a last-option-wins `-fwrapv`/`-fno-wrapv` policy, reset
per translation unit even at O0/O1. Under wrapping, skip the current
UB-dependent IV widening, signed affine-exit fold and quadratic finite
transformation. Pointer IV strength reduction retains pointer-width ring
and certified unsigned cases but rejects pre-extension narrow signed
products/counters/addends. This is conservative; future range proofs may
recover safe subsets. It does not turn off the optimizer or unsigned bound
canonicalization. Default non-wrapping TUs preserve the old policy.

The full existing IV-domain gate and all 48 new native runs (4 domains ×
6 optimization profiles × 2 pointer widths) pass. CLI flag ordering and
same-worker per-TU policy reset have Rust regression coverage. Exact GCC
16.2, Clang 23.1 and LCCC each pass all four domains on both ABIs under
Memcheck (24 runs). The initially missing i386 glibc debug symbols were
installed; that infrastructure failure was not counted as a pass.

## P3 — lane-wise integer DAGs for ordered reciprocal reductions

The accepted arithmetic subset builds four I32 lanes using existing vector
operations; Add/Sub/Mul/bitwise ops are modular and shifts require literal
counts 0..31. Other nodes retain the original scalar-clone path. Plans are
transactional, memoized and depth-bounded, with a four-operation profitability
floor. Broadcast(j)+{0,1,2,3} avoids four scalar lane live ranges. A separate
five-argument intrinsic keeps scalar result allocation distinct; all three
regalloc classifications are updated. The shared FP backend consumes exactly
XMM/m128, not m256, and preserves lane-ordered scalar additions and operand
order. `CCC_DISABLE_PASSES=strict_recip_pack` restores scalar denominators.

The first structural test REJECTED this candidate despite correct arithmetic:
legacy MOVD/PSHUFD broadcasts remained in the AVX loop. The I32x4 broadcast
emitter now uses explicit VEX on AVX2 and keeps its baseline SSE fallback;
no unsafe relaxation of the conservative text-promotion analysis was made.
Two Rust tests pass (all supported integer operations, modular boundaries,
invalid/variable shifts, division refusal, small-DAG refusal). Nine native
and Memcheck arms each pass 1,260 **bitwise** cases: GCC16.2, Clang23.1,
normal packed, no scalar XMM allocation, no vector allocation, no final VEX
promotion, scalar denominators, scalar loop, and baseline ISA. The arrays
end at a guard page; zero/negative trips, all tails, signed wrapping,
negative denominators, signed zero and allocation fallbacks are exercised.
The older variable-shift reciprocal structural gate also passes unchanged.

The bounded Callgrind A/B against the wrapping-contract-fixed compiler is
**spectral_norm 0.6448442** (35.52% fewer simulated instructions); byte and
XML are exactly unchanged in that causal comparison. Compiler work, size,
native paired timing and the final full fifteen-case screen are not yet
acceptance evidence for this snapshot.

### Additional red-team finding: FP traps (not numerical rounding)

A separate `-ftrapping-math` + FENV_ACCESS probe distinguishes source-order
add overflow from a later denominator's divide-by-zero. GCC16.2/Clang23.1
trap first on overflow; independent main, imported PR and this candidate
trap first on division by zero. Disabling all vectorization repairs it;
disabling only packed integer denominators does not. Thus this is a real
pre-existing ordered-reciprocal exception-order defect, not caused by the
new integer DAG. The finite/nontrapping bitwise evidence above is not a
proof of trapping-FP legality. **This snapshot remains unapproved.**

## P2 refinement — retain independently range-proven widening under -fwrapv

The complete fifteen-case S28 screen exposed costs hidden by a surgical
correctness gate: disabling all IV widening under wrapping raised sieve Ir
18.0%, SHA 6.1%, Adler 5.0%, varint 3.2%, strlen 2.3%. These are not called
nonregressions. The refined analyzer restores unsigned counted-range proofs
and a restricted signed subset: nonnegative constant seed, positive constant
stride, typed signed compare against a nonnegative constant, structural
entry/dominance/latch checks, and signed-domain room through the final latch.
Every derived Add/Sub/Mul/Shl needs its own representable range; unsupported
member/member arithmetic and signedness changes decline. It never licenses
the original wrapped address product via signed UB.

The range calculation for constant-minus-member now respects subtraction
operand order (the interval operands used to be reversed). A direct IR test
pins both refusal of the wrapping subtraction and admission of the safe
reverse subtraction. A C probe of that issue did NOT reproduce wrong code
and is not reported as a demonstrated C miscompile.

All **60 IV-widening Rust tests pass**. The existing domain gate, 48 wrapping
domain executions and 12 additional executions of an eight-shape modular
range fixture pass, including signed counter/product wrap, negative seeds,
fixed bounds, positive nonunit steps and a terminating side exit after wrap.
The original four-domain exact-oracle/ABI Memcheck matrix passes again.

## A2 — observable FP environment fences

The demonstrated trap-order defect is repaired for explicit
`-ftrapping-math` and active `#pragma STDC FENV_ACCESS ON`/`_Pragma` requests.
The preprocessor records any active ON for the translation unit (later OFF
cannot erase an earlier obligation); inactive branches do not enable it.
The per-TU policy resets at every pass-run entry including O0/O1. Explicit
trapping options are last-wins; fast-math clears trapping permission. The
existing numerical-only `-fno-fast-math` handling is not represented as a
complete implementation of all GCC FP suboptions. Ordinary FENV-off TUs
retain their existing defaults.

Until these passes have a constrained-FP effect model, FP-containing
functions conservatively decline automatic batching, GVN, constant folding,
simplification, DCE, LICM, if-conversion, expression sinking and FP constant
hoisting; integer-only functions retain the normal optimization path.
Automatic contraction is also disabled in the observable-environment mode.
This is intentionally a correctness-first fence, not a claimed full Annex-F
implementation or an assertion that every dynamic-FP corner was audited.

Validation: **4727 Rust library tests PASS, 0 FAIL, 8 existing ignored**,
including CLI precedence, active/inactive/_Pragma recognition, and same-worker
TU-policy resets. All 18 native opt-level/request-mode combinations pass
both zero/negative-trip non-speculation and source-first overflow ordering.
GCC16.2 and Clang23.1 independently agree. The pre-repair and post-repair
native statuses are retained; hardware FP traps are checked natively, not
claimed to be modeled by Callgrind. All nine packed-integer bitwise/Memcheck
arms still pass 1,260 cases each. Both new gates are wired into fast local
and hosted CI; the standalone gate-parity checker passes.

## A3 — implicit architectural effects and partial writes

Three new CLZERO counterexamples FAIL before repair: an implicit memory-store
classification, its RAX-address read, and direct stale-load forwarding across
the cache-line clear. After repair, both direct and full-pipeline tests retain
the address setup and post-clear memory read. No native AMD CLZERO execution
or automatically emitted C CLZERO miscompile is claimed on this Intel VM.
AMD's instruction description is linked in the vendor-contributed assembler
patch [1](https://github.com/yasm/yasm/pull/78).

Review of the adjacent shared table exposed additional effect omissions:
INS reads its destination pointer, MWAITX conditionally consumes EBX, RDPRU
reads ECX and writes EAX/EDX, RDPKRU also clears EDX, ENTER consumes the old
stack/frame pointers, and SYSENTER/SYSEXIT replace RSP. The partial-write
projection must not treat MULW/DIVW/IMULW/IDIVW or POPAW's 16-bit data-register
results as whole-family definitions. Minimum-read/exact-write/full-write
fixtures reproduce **19 failures out of 23 cases before repair**, then zero.
These are classification proofs, not 19 demonstrated C miscompilations.

References: RDPKRU's EDX zeroing [2](https://www.felixcloutier.com/x86/rdpkru),
SYSEXIT [3](https://www.felixcloutier.com/x86/sysexit), SYSENTER stack reload
[4](https://xem.github.io/minix86/manual/intel-x86-and-64-manual-vol3/o_fe12b1e2a880e0ce-173.html),
and the Linux MWAITX register contract
[5](https://git.yoctoproject.org/linux-yocto-dev/plain/arch/x86/include/asm/mwait.h).
The production classifiers can be checked without a full rebuild using
`scripts/check_implicit_operand_matrix.py`; the Rust library test consumes
the same independently reviewed fixture, including trailing-comment forms.
A native upper-bits witness independently passes GCC16.2, Clang23.1 and LCCC,
also under Memcheck. The initial test implementation incorrectly assumed an
available serde_json dependency; it was replaced by a dependency-free TSV
fixture parser, not by adding a production dependency or weakening the test.

## Rejected P5 — read-only scalar unroll and earlier post-inline byte folding

A two-block scalar reduction admission experiment kept source-order 32-bit
loads/adds, admitted only one independent load and one 32-bit sum phi, capped
body size, and retained all existing trip/CFG/SSA proofs. Two direct units
passed, including SSA and load-width checks. The first placement did not
change byte-load code at all: the byte DAG had not yet folded when the sole
post-vectorization unroll opportunity ran. Moving byte folding before that
opportunity activated the four-way candidate. Twelve native/Memcheck
compiler/ABI arms passed 33 guard-boundary/seed cases each.

Nevertheless the candidate is **REJECTED AND REMOVED FROM PRODUCTION**:
Callgrind Ir improved to **0.892666** of S31, but the short native run was
inconclusive (1.0210, p=.1096). Repeating the kernel 100× within each process
removed most startup dilution: 15 pinned paired rounds measured **1.0684**
candidate/S31 median, sign-test p=.0098, with the minimum also worse.
The larger body repeatedly materialized addresses and increased register
pressure. An independently labeled two-way SOURCE model (not a claim about
production codegen) counted 51,622,774 Ir but had no significant calibrated
native win: ratio .9700, p=.1213. Neither proposal is shipped merely because
its instruction count looks better. Candidate source patch, phase receipts,
raw profiles, native samples and exact experiment sources remain evidence.

The guard-page fixture remains useful for the accepted byte-load folding and
future strided optimizations. Its final eight compiler/ABI arms pass 33 cases
each, including native/Memcheck and LCCC SSA checks. A GCC16 i686 AVX run
passes natively but Valgrind 3.24 cannot decode its VEX instruction; that
failure is retained, not counted as PASS. The instrumented 32-bit arm is
explicitly labeled i686/SSE2, not AVX2.

Raw evidence TAR streams were losslessly repacked to XZ with byte-stream and
compressed-file hashes verified before removing redundant GZIP copies. No
counter samples or failed experiments were discarded.
