# Intel ISPC → LCCC transplant audit — 2026-11-07

## Scope and method

Systematic audit of `https://github.com/ispc/ispc` at `ae2ae73`
(`Fix #4008: Renumber metadata before printing final IR on LLVM 24`,
2026-11-06), shallow-cloned into `/home/user/ispc` (119 MiB).  The goal was
**ideas**, not code: ISPC is BSD-3-Clause and LLVM-IR-based, LCCC is a bespoke
IR with four backends (x86-64, i686, AArch64, RISC-V).  Copying an LLVM pass
is neither possible nor useful; the transferable unit is the *analysis
contract* and the *optimization it unlocks*.

Every candidate below was checked against three questions:

1. Is there a real gap in LCCC (proved by compiling a repro, not by reading
   the pass list)?
2. Is the gap reachable with LCCC's current IR opcodes, or would it require a
   new opcode family?
3. Can the resulting transformation be proved sound on **all four backends**
   with a fail-closed analysis, and can it be exhaustively tested?

Toolchains used for the empirical baseline (restored from the
`harness-cache-8` release tag): `rustc 1.99.1`, `gcc-16 16.2.0`,
`clang-23 23.1.5`, `gdb 18.1`, `mold 3.0.0`, `qemu-user 11.1.3`.  The local
ICX 2026.2 PPA endpoint was unreachable (`http=000`) and the `intel/llvm`
`latest` release carries only SYCL tarballs, so no local ICX comparison was
possible; GitHub CI's authenticated `icx-oracle` lane is the ICX reference.
**No broad "beats GCC/Clang/ICX" claim is made.**  The measured claim is
narrow: one constant-SIMD-mask kernel.

## Measured gap (the reason this audit produced code at all)

```c
#include <emmintrin.h>
int const_movmsk(void) {
    __m128i m = _mm_setr_epi8(
        -1, 0, -1, -1, 0, 0, -1, 0,
        -1, -1, -1, -1, 0, 0, -1, -1);
    return _mm_movemask_pi8(m);           // 0xCF4D == 53069
}
```

| compiler | `const_movmsk` body |
|---|---|
| GCC 16.2 `-O2` | `movl $53069, %eax; ret` |
| Clang 23.1.5 `-O2` | `movl $53069, %eax; ret` |
| LCCC `-O2` **before** | 16 `movb` + `movdqu` + `pmovmskb` + prologue/epilogue (22 instructions) |

`LCCC_DUMP_IR=1` showed why: LCCC has **no vector `IrConst`**.  A constant
vector is a private `Alloca` tiled by 16 scalar `Store`s, read by
`VecLoadI8x16`, written into a second private `Alloca` by `VecStoreI8x16`,
then consumed by `Pmovmskb128`.  `constant_fold.rs` has no
`Instruction::Intrinsic` arm at all, so no pass could reason about the mask.

On AArch64 the same intrinsic is a 17-instruction NEON reduction
(`ushr`/`mul`/`addv`/`umov`/`orr`), and on RISC-V a software reduction.  The
gap is therefore **largest on the non-x86 backends**, which is the opposite of
where most of LCCC's SIMD work has gone.

## ISPC inventory (what was read)

`src/opt/` sizes and the relevant idea in each:

| file | lines | transferable idea |
|---|---:|---|
| `ImproveMemoryOps.cpp` | 1405 | `lGSToLoadStore`: all-equal gather offsets → scalar load + broadcast; `LLVMVectorIsLinear(fullOffsets, ±step)` → contiguous unaligned masked load/store; negative stride (all-on only) → lowest-address load + `lReverseVector` |
| `GatherCoalescePass.cpp` | 1062 | `CoalescedLoadOp` + `lVectorLoadIsEfficient`: coalesce nearby offsets with a gap budget |
| `MemoryAccess.cpp` | 788 | gather/scatter cost model |
| `XeGatherCoalescePass.cpp` | 649 | Xe-specific coalescing |
| `MemoryOpRegistry.cpp` | 415 | registry of pseudo memory ops |
| `LowerISPCIntrinsics.cpp` | 360 | `lGetSequentialMask`, `__packmask` |
| `PeepholePass.cpp` | 323 | `(zext/sext(a)+zext/sext(b)[+1])/2` → `__avg_up/down_{u,i}{8,16}` with a recursion guard |
| `ReplaceMaskedMemOps.cpp` | 313 | `lCheckMask`: constant true-prefix masks → unmasked ops |
| `LowerAMXBuiltinsPass.cpp` | 253 | AMX tile lowering |
| `ReplacePseudoMemoryOps.cpp` | 235 | pseudo → real memory ops |
| `IntrinsicsOptPass.cpp` | 231 | blend with equal/undef arms or all-on/all-off mask → arm; constant `__movmsk` → integer; AVX masked load/store all-off → poison/erase, all-on → regular load/store |
| `InstructionSimplify.cpp` | 151 | all-on/all-off `Select` → arm; `trunc({sext,zext}(i1 vec))` → i1 vec; constant `__movmsk` → scalar |

`ISPCPassRegistry.def` lists `gather-coalesce`, `improve-memory-ops`,
`instruction-simplify`, `intrinsics-opt`, `peephole`,
`replace-masked-memory-ops`, `replace-pseudo-memory-ops`,
`replace-stdlib-shift`, `scalarize`, `fast-math`.  `docs/perfguide.rst:325`
("Understanding Gather and Scatter") and `docs/ispc.rst:1368` (`--wno-perf`)
were read for intent, not implementation.

The single most reusable abstraction is ISPC's `MaskStatus` lattice
(`src/llvmutil.h:32`, `GetMaskStatusFromValue`/`GetMaskFromValue` in
`src/llvmutil.cpp:1920`): `{all_on, all_off, mixed, unknown}`, reused by five
different passes.  LCCC had no equivalent anywhere.

## Transferability decisions

| ISPC idea | LCCC status | decision |
|---|---|---|
| `MaskStatus` lattice | absent | **TRANSPLANTED** — `MaskStatus::{AllOn,AllOff,Mixed(u32),Unknown}`, `from_lanes`, `from_bytes` |
| constant `__movmsk` → scalar | absent (`constant_fold.rs` has no `Intrinsic` arm) | **TRANSPLANTED** — `Pmovmskb128/256`, `MovemaskPs/Pd128/256` |
| blend with trivial mask → arm | absent | **TRANSPLANTED** — all ten `VecBlendv*` ops, plus identical-arm fold |
| all-on/all-off AVX masked load/store | **no IR opcode** (`Instruction` has no gather/scatter/masked-memory variants) | deferred: needs a new opcode family, not a pass |
| `ImproveMemoryOps` stride/broadcast recognition | `VecLoadStrided2*` exists but no pseudo-gather/scatter call to rewrite | deferred: the LLVM analysis operates on pseudo calls LCCC never creates |
| `GatherCoalescePass` | no gather op | deferred |
| `PeepholePass` widen-then-average → `pavgb/pavgw` | `Pavgb128`/`Pavgw128` exist; current `avg_up_u8` emits 7 ins vs GCC 5 | deferred: an IR-pattern rewrite with a recursion guard, a separate risk class from constant folding |
| `LowerISPCIntrinsics` sequential-mask pack | vectorizer-specific | deferred |

The deferred items are **not** "not worth it": they are the next
transplants, but each requires a new IR surface and therefore a separate
proof.  Shipping them together with a constant-fold pass would make the
patch un-auditable.

## What was actually implemented

`src/passes/simd_mask_facts.rs` (pass `maskfacts`, disabled with
`CCC_DISABLE_PASSES=maskfacts`), wired immediately **before** phase 11e
`constarr` in `src/passes/mod.rs`.  Order matters: `constarr` replaces the
constant store image with a `.rodata` global, after which the byte-level proof
is unrecoverable.  Evidence consumers run before evidence destroyers.

### The adaptation ISPC did not need

ISPC inspects an LLVM `ConstantVector` node.  LCCC has only scalar `IrConst`,
so the lattice needs a **constant-object proof**:

* object is a non-volatile, non-semantic-volatile `Alloca`;
* every mention of its address — through `Copy`/`GEP` chains of **any**
  length, including variable offsets (`base_of_value`, memoized, cycle-safe)
  — is in the read's block, at or before the read;
* a scalar `Load`, `Phi`, `Select`, `Cast`, call, inline asm, atomic,
  terminator or arithmetic mention vetoes the proof;
* every write is a scalar `Store` of an `IrConst` whose `to_le_bytes()` width
  equals the store's `ty.size()`, or a recognized whole-register `VecStore*`
  with a known source image;
* the requested window is tiled **exactly once** (gap or overlap → refuse);
* recursion is bounded (`MAX_DEPTH = 16`, `MAX_WORK = 4096`) and
  cycle-checked.

`GetElementPtr.offset` is a raw **byte** offset in LCCC
(`src/backend/x86/codegen/isel.rs:1343`, and `const_array_promote` uses it
the same way), so no element scaling is applied.

### Correctness decisions that a naive port gets wrong

1. **Little-endian lane sign bits.**  `from_lanes` reads bit 7 of the LAST byte
   of each lane.  `IrConst::to_le_bytes` uses `to_le_bytes` /
   `f32::to_bits`, so this is exact for I8/I16/I32/I64 and for the IEEE-754
   sign bit of F32/F64.  Verified by the exhaustive 65,536-mask sweep and the
   F32/F64 sign-bit tests.
2. **Zero-vector widths are checked.**  `VecZeroI32x4` is 16 bytes,
   `VecZeroI32x8` is 32.  A 128-bit zero cannot satisfy a 256-bit mask:
   legacy SSE leaves the upper YMM half undefined.  A naive
   `vec![0; width]` for any width is a miscompile; the pass refuses.
3. **Splat lanes must be exact-width.**  Zero-extending `I8(-1)` into an I32
   lane gives `0x000000FF` (sign bit 0); sign-extending gives `0xFFFFFFFF`
   (sign bit 1).  There is no IR evidence of intent, so refuse.
4. **`VecStoreI32x4Pair` is excluded.**  It is a half-wide `movq` of the LOW
   EIGHT bytes of a 16-byte XMM source (`src/passes/alias.rs:155` reports
   `(true, 8)`; `vector_temp_promotion` answers `Some(8)` deliberately to
   fail its own exact-width gate).  Treating `args[0]` as an 8-byte vector
   value is not a proof.
5. **Blend masks use `from_bytes`**, a lane-width-independent sufficient
   condition: if every byte's sign bit agrees, every dword/qword/byte lane's
   selector agrees.  A well-formed `0x80000000` dword mask is reported
   `Unknown` — conservative, but correct on all ten blend ops and all four
   backends without a per-intrinsic lane table.
6. **Two-phase rewrite.**  All decisions use one immutable `Facts` snapshot
   (borrowed `&Instruction`, zero clones), so one fold cannot change another
   fold's evidence.  The borrow ends before the first mutation.
7. **`I32` result type.**  `pmovmskb`/`movmskps` produce a 32-bit GPR; the
   all-on 32-lane case is `u32::MAX` whose `i32` bit pattern is `-1`, exactly
   what `vpmovmskb %ymm, %eax` leaves in `%eax`.
8. **Private-image reclamation.**  Folding the movemask alone leaves 16
   `movb` + `movdqu` + the prologue — not a win.  `prune_dead_private_writes`
   re-derives non-escape from scratch and deletes only writes to objects with
   no reader; `dce` removes the orphaned derivations; a bounded fixpoint also
   reclaims the SOURCE object once the dead `VecLoad*` disappears.  Span
   vectors stay 1:1 with instructions throughout.

### Why this is architecture-independent

The fold produces one `I32` constant.  No backend-specific assumption about
`args[0]`, `dest_ptr`, register widths or ABI enters the proof; the intrinsic
is deleted before codegen.  That is precisely why AArch64 and RISC-V benefit
most: their software `pmovmskb` sequences disappear entirely.

## Verification performed

* `cargo test --lib --profile fastbuild --locked simd_mask_facts` — see the
  PR conversation for the exact pass/fail counts.
* 65,536 exhaustive 16-lane byte masks with arbitrary low-7-bit noise.
* Exact `53069` GCC/Clang oracle, nine mixed masks, 32-lane no-shift-overflow
  (`0x5555_5555` and `-1`), F32/F64 sign-bit masks, all ten blend ops,
  zero-vector width mismatch, splat lane width, half-wide pair store,
  phi agreement/disagreement, copy cycles, unsound-image refusals (gap,
  overlap, non-constant/volatile store, semantic-volatile alloca,
  arithmetic/variable GEP, unknown intrinsic, undersized object, post-read
  and cross-block writes), escaping address, deep 40-link GEP derivation,
  terminator escape, cast/arithmetic/volatile/address-store vetoes, and
  span parallelism + idempotence.
* `LCCC_DUMP_IR`, `-S` assembly comparison against GCC 16.2 and Clang 23.1.5.

## Explicitly NOT claimed

* No general "LCCC beats GCC/Clang/ICX" claim.  One measured kernel only.
* No local ICX 2026.2 result (PPA unreachable, no compiler asset upstream).
* No local CI run (per standing instruction: GitHub CI performs full
  validation).  Local checks are `cargo fmt`, strict Clippy, focused unit
  tests, and the C/asm probes.
EOF
echo "DOC"; wc -l engineering/ISPC-TRANSPLANT-AUDIT-2026-11-07.md