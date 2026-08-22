# Session 58 — full Lev Aug-19+ semantic review and ARM hardening

Date: 2026-08-22

Bases reviewed:

* current work: `ms178/lccc` based on `bbd907bf`;
* Lev tip: `53ef5201`;
* review window: all 70 Lev commits with author timestamps from 2026-08-19 UTC onward.

This document records semantic disposition, not patch-id equivalence. Lev's
branch diverged hundreds of commits ago; copying a patch because its subject
looks useful is unsafe. Current implementations were inspected at their active
call sites and tested on the current IR/backend contracts.

## Major result 1: replace the text peephole with an SSA machine combine

Session 57's hardened assembly-text CSINC fold was still the wrong abstraction.
Even with conservative physical-register scans it had unavoidable problems:

* physical assembly no longer carries SSA identity;
* CFG/liveness reconstruction was expensive and incomplete;
* each iterative peephole round rebuilt a module-wide label map;
* implicit ABI uses needed ad-hoc modeling;
* the fold happened after register allocation, so the removed temporary had
  already distorted register assignment.

The text fold is removed completely. The replacement recognizes, in current
SSA IR:

```text
increment = add base, 1
result    = select condition, increment, base
```

and the swapped-arm form. Legality requires:

1. integer arithmetic of an AArch64-supported 32- or 64-bit width;
2. exactly one definition and one use of `increment`;
3. structural identity between the Add base and the opposite Select arm;
4. exact constant `1` (not `2`, `-1`, or an arbitrary value);
5. width-preserving semantics, including the frontend's U32-to-I64 Select
   representation, where a W-register CSINC provides the required U32 wrap and
   zero extension;
6. for direct compare fusion, exactly one Cmp definition/use and no intervening
   instruction except the Add that is itself removed.

AArch64 then emits either:

```asm
cmp   condition, #0
csinc result, base, base, eq|ne
```

or consumes integer Cmp flags directly. All ten signed/unsigned comparison
conditions are mapped and tested explicitly. Other architectures advertise no
support and therefore retain byte-for-byte ordinary lowering.

This design has no unsafe code, pointer arithmetic, indexing outside checked
slices, shared mutable state, locks, allocation lifetime, or ownership hazard.
Detection builds one block-local definition table and is linear in block size;
unlike the old iterative text pass it performs no repeated reverse scans and
does not rebuild global text maps on every round.

## Major result 2: LICM insertion is transactional and SSA-order-correct

Lev `53ef5201` correctly identified that appending hoisted instructions can put
a definition after an existing preheader use. Its proposed `max(lower, upper)`
insertion is not generally sound: if a hoisted instruction depends on an
existing definition that is itself after the earliest existing use of the
hoisted result, the bounds cross and no order-preserving insertion exists. The
proposal would still insert after the use.

The current implementation now:

1. gathers and topologically sorts candidates without mutating IR;
2. computes the latest existing definition needed by the hoisted slice;
3. computes the earliest existing use of any hoisted destination;
4. rejects the transformation unchanged if the lower bound exceeds the upper;
5. only after legality succeeds removes original instructions and inserts the
   sorted slice;
6. preserves source-span lockstep in source blocks and clears stale preheader
   spans after cross-block movement.

A focused test proves insertion before an existing use. A second test constructs
crossed bounds and proves rejection is atomic by comparing the entire block
representation before and after.

## Major result 3: exact matmul stream legality

Lev `53ef5201` also correctly found that the old matmul matcher accepted column
walks and loops with extra stores. Its candidate `traces_to_iv` check was still
too weak: `iv * 64` traces to the IV but is not a contiguous F64 stream.

The adopted solution is stricter:

* exactly one Store over the complete natural-loop body;
* C and B GEPs must use the existing exact byte-stride recognizer with stride
  exactly 8;
* the unscaled index must reach this loop's IV through Cast/Copy chains only;
* cycles and unknown definitions fail closed;
* AArch64's nominal two-wide lowering is costed at its real four-double machine
  step, requiring two machine iterations for a known constant trip count.

The i-j-k N=4 column-stride reproducer remains scalar and matches AArch64 GCC
under QEMU. Four existing contiguous matmul regressions still vectorize and
pass, so the legality gate does not destroy the intended optimization.

## Full Aug-19+ Lev commit disposition

| Commit(s) | Semantic disposition on current tree |
|---|---|
| `51285cd5`, `a0aa48d7` | Previously audited. Alias extraction/GVN ideas are superseded by current per-object epochs and shared alias infrastructure; dead-end notes remain historical only. |
| `0c5134cc`, `dd0fa7c5`, `9f304050`, `9c48a277` | FP register-direct loads, loop-phi handling, and call-spanning callee-saved allocation are represented by the current FP allocator/backend. Old patches must not be replayed over current hole-aware liveness. |
| `6e955227`, `3fa030e5`, `bbab1d12` | Plain-copy vectorization and late load/DCE concepts were previously reviewed; current vector/temp and DCE pipelines supersede these revisions and carry stronger memory barriers. |
| `8a052b9a` | Register-direct Select is present. Its active helper was reused by the new SSA CSINC emitter to prevent staging-policy drift. |
| `885f129c` | Quadratic/triangular strength reduction exists in current `quadratic_sr`; no old patch replay. |
| `6a42dd1e` | Documentation snapshot only; not authoritative. |
| `ed36c446` | GlobalAddr CSE was independently hardened and is active before GVN. The old multi-use fixes are superseded by current centralized visitors. |
| `e3b21b8f`, `62cc4bb1` | FMSUB/FNMSUB contraction exists behind the current floating-point contract; roadmap correction is historical. |
| `1b4bac8b` | Aggregate-forwarding soundness and unroll policy were independently changed multiple times; current fail-closed escape handling wins. |
| `aadab341`, `de451e57`, `b712c79c` | FP slot forwarding, pair fusion, and repeated-slot elimination are active, with current overlap/clobber tests. |
| `e20cdf6e` | Rejected as a whole: its derived-base GDSE caused regressions on current codegen. Current per-function conservative address-taken bail-out remains the sound policy. |
| `d0a30aa0` | Dead-end documentation only. |
| `8ef1978f`, `8b139820` | Conditional-sum/max vectorization ideas are represented by current late vectorization, with stronger alias/trip/element-type gates. |
| `89111a1f`, `2e57bcf2` | Zero-extension chains and gap FMA fusion are active in current backend forms; current fusion-aware liveness supersedes old text windows. |
| `dbd22184` | Benchmark snapshot only; no performance claim imported. |
| `e03da2f1` | Aggregate initializer DSE bug is superseded by current default-closed root/escape analysis. |
| `f958b6af`, `b4fb8604`, `eb587035` | Priority scans, exact occupancy, peeled-index liveness, and caller/callee FP ordering are superseded by current multi-segment/hole-aware allocator. Replaying these monolithic patches would regress current RA contracts. |
| `339cdb8b`, `69bbce24` | Overwritten move/store and loop-store passes are present. Current tests retain branch/call/address hazards. |
| `a0791ef9`, `21801059` | Direct call staging and x0-clobber correction are represented in current call codegen; call-argument regressions pass. |
| `51673cf6`, `fcc571fc`, `0730d480` | Explicitly recorded dead ends; not adopted. |
| `c3d66f9a`, `89ec59c5`, `b15548ac`, `6b8fa0b5`, `38a85795`, `c710ed4a` | Coalescing/anti-dependency experiments are superseded by current point/segment-aware coalescing and call-span checks. No isolated old patch is safe against current RA. |
| `8974cc9a` | ADR/jump-table and UBFX idea reviewed; not part of this CSINC/LICM/vector legality patch and receives no unmeasured performance claim. |
| `86b34508`, `9f064faa` | Zero-register stores and integer MADD/MSUB are present in current upstream combined ARM work. |
| `177b560d` | FP-through-GPR copy folding is represented in current ARM peephole battery. |
| `2fdf496e` | Caller-saved-first leaf allocation is not blindly adopted. It interacts with hard-coded address/call/soft-float scratch registers and later Lev notes report pool-swap regressions. The Godbolt result still exposes leaf-prologue debt, recorded below. |
| `073cf9da`, `dbb9b3ae`, `809231f7`, `6ce9c620`, `f3adae65` | Roadmap/dead-end/result notes only; no code imported. |
| `972e5535`, `03e21b1a`, `bdc2aba1` | SXTW known-nonnegative reasoning exists in the current peephole family. Broader range-proof work remains preferable to textual constant tracking. |
| `558e3ed9` | Integer constant hoisting exists; current policy and loop analyses supersede this revision. |
| `7c54ece0` | Cross-kind affine disjointness is represented in current shared alias logic; no old patch replay. |
| `0d47feeb` | Shrink-wrapping remains deferred: clean-early-return text matching is not a substitute for CFG-level prologue/epilogue placement. |
| `4364a6b7`, `cac07bd5` | Benchmark snapshots only. QEMU timing and old-machine geomeans are not accepted as current evidence. |
| `1a432a1a` | Function-scoped GDSE is active and tested; this part was previously adopted independently. |
| `49b7e085`, `cc32a8f9`, `c9285e2f` | Bounded self-inlining was reverted by Lev; final disposition is reject. |
| `a08f6768`, `5ae9b311` | Backedge PRE remains deferred. It lacks the current centralized operand-rewrite contract and adequate zero-trip/multi-latch/FP tests. |
| `24200ab4` | Useful CSINC idea, but the physical-register text implementation is removed. Reimplemented as the SSA machine combine described above. |
| `1e3d8c84` | IR half already superseded; ARM half patches an unused tracker under the current conservative GDSE design. Not adopted. |
| `53ef5201` | Root causes confirmed. LICM and vector gates adopted with stronger transactional insertion and exact-stride legality; candidate's crossed-bound and arbitrary-IV-scale gaps are fixed. |

## Oracle evidence and remaining gap

`codegen_oracle.py` now supports `--arch aarch64`. Its statistics understand
AArch64 brackets, load/store mnemonics, stack bases, vector operands, and branch
families. The oracle compared `inc_if_sge_i32` against Compiler Explorer ARM64
GCC 16.1:

* both choose direct compare + conditional increment in the hot body;
* GCC's complete leaf is 3 instructions (`cmp`, `cinc`, `ret`);
* LCCC's complete leaf remains 16 instructions because current ARM register
  allocation moves parameters into callee-saved homes and emits save/restore.

The assembly dumps and report are persisted under
`engineering/evidence/godbolt/session58-arm-csinc/`.

Therefore this session claims **instruction-selection parity for the idiom**, not
whole-function superiority. The leaf-register/prologue gap is real and must be
fixed through ABI-aware caller-register hints and scratch-liveness modeling, not
by unsafe pool swapping.

## Validation record

All compiler builds used only `scripts/build_lccc_fast.sh` (`fastbuild`, Rust
O1, two jobs, no LTO). No release build was run.

Passing:

* fastbuild with warnings denied;
* library tests: 1085 passed, 6 ignored, 0 failed (1091 total, including 4 new combine tests and 2 new LICM tests);
* correctness differential: 50/50;
* progressive: 22 pass, SQLite-only level skipped;
* new CSINC semantic regression under AArch64 GCC/QEMU: pass;
* 14 positive CSINC shapes and 2 negative shapes in assembly checks;
* all ten integer comparison condition inversions checked exactly;
* independent GNU AArch64 assembly of generated CSINC output;
* the same semantic corpus matches GCC when compiled by LCCC x86-64 and i686,
  proving unsupported backends retain correct ordinary lowering;
* ARM full-suite A/B: 257 pass, 56 pre-existing failures, 67 skip with the same
  failure set on both sides (one diagnostic line number differs because code
  size changes);
* four existing contiguous matmul ARM regressions: pass and remain vectorized;
* new N=4 column-stride matmul: pass and remains scalar;
* Python AArch64 stats self-check and live Compiler Explorer oracle: pass.

The x86 regression run exposed a pre-existing test-script mismatch: the compiler
correctly reports `'return' with a value`, while the script searched for
`return with a value`. The expectation is corrected; the focused script passes.

## Follow-up priorities

1. Design ABI-aware ARM leaf allocation that can preserve x0-x3 parameter homes
   while proving every hard-coded scratch use; target GCC's 3-instruction leaf.
2. Classify the 56 ARM regression failures into x86-only filtering defects,
   unsupported features, assembler defects, and real runtime miscompilations.
3. Add a pre-register-allocation machine-combine inventory so every skipped SSA
   producer is excluded from RA/stack layout and cannot distort unrelated homes.
4. Rework shrink wrapping at machine CFG level, never as assembly-text matching.
5. Validate CSINC and matmul changes on real AArch64 hardware with retired
   instructions, cycles, IPC, branch, and cache counters. QEMU wall time is not
   hardware-performance evidence.
