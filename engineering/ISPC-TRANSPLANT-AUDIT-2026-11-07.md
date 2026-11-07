# Intel ISPC → LCCC transplant audit — 2026-11-07

## Scope and method

Systematic audit of `https://github.com/ispc/ispc` at `ae2ae73`
(`Fix #4008: Renumber metadata before printing final IR on LLVM 24`,
2026-11-06), shallow-cloned into `/home/user/ispc` (119 MiB). The goal was
**ideas**, not code: ISPC is BSD-3-Clause and LLVM-IR-based, LCCC is a bespoke
IR with four backends (x86-64, i686, AArch64, RISC-V). Copying an LLVM pass is
neither possible nor useful; the transferable unit is the *analysis contract*
and the *optimization it unlocks*.

Every candidate was checked against three questions:

1. Is there a real gap in LCCC (proved by compiling a repro, not by reading
   the pass list)?
2. Is the gap reachable with LCCC's current IR opcodes, or would it require a
   new opcode family?
3. Can the transformation be proved sound on **all backends** with a
   fail-closed analysis, and can it be exhaustively tested?

Toolchains used for the empirical baseline (restored from the
`harness-cache-8` release tag): `rustc 1.99.1`, `gcc-16 16.2.0`,
`clang-23 23.1.5`, `gdb 18.1`, `mold 3.0.0`, `qemu-user 11.1.3`. The local
ICX 2026.2 PPA endpoint was unreachable (`http=000`) and the `intel/llvm`
`latest` release carries only SYCL tarballs, so **no local ICX comparison was
possible**; the hosted authenticated `icx-oracle` CI lane is the only ICX
reference. `objtool`, `pahole`, `valgrind` and `perf` are absent from the local
image; GDB 18.1, `readelf`, `nm`, `objdump` and `qemu-user` are available.
**No broad "beats GCC/Clang/ICX" claim is made.** The measured claim is
narrow: a set of constant-SIMD-mask kernels.

## Measured gap (the reason this audit produced code at all)

```c
#include <emmintrin.h>
int const_movmsk(void) {
    __m128i m = _mm_setr_epi8(
        -1, 0, -1, -1, 0, 0, -1, 0,
        -1, -1, -1, -1, 0, 0, -1, -1);
    return _mm_movemask_epi8(m);          // 0xCF4D == 53069
}
```

| compiler | `const_movmsk` body at `-O2` |
|---|---|
| GCC 16.2 | `movl $53069, %eax; ret` |
| Clang 23.1.5 | `movl $53069, %eax; ret` |
| LCCC **before** | 16 `movb` + `movdqu` + `pmovmskb` + prologue/epilogue |
| LCCC **after** | `movl $53069, %eax; ret` |

`LCCC_DUMP_IR=1` showed why: LCCC has **no vector `IrConst`**. A constant
vector is a private `Alloca` tiled by scalar `Store`s, read by `VecLoad*`,
written into a second private `Alloca` by `VecStore*`, then consumed by
`Pmovmskb128`. `constant_fold.rs` has no `Instruction::Intrinsic` arm, so no
pass could reason about the mask.

Two further gaps were found by probing rather than reading:

* **All four movemask widths.** `_mm_movemask_ps` / `_mm_movemask_pd` reach
  the mask through `Memcpy` chains and `VecLoadI64x2`/`VecStoreF32x4`
  temporaries; `_mm256_movemask_epi8` reaches it through 32 narrow
  `I64(255|0)`/`U8` stores. Before the fix only `Pmovmskb128` folded.
* **Byte-logic chains.** `_mm_movemask_epi8(_mm_and_si128(A, B))` reaches the
  pass as a legacy `Pand128` intrinsic that **writes through `dest_ptr`** into
  a stack object, with two pointer operands. Before the fix no pass could
  derive a constant vector from a computed vector at all, and both movemasks
  survived with 19 residual vector/stack instructions.

Reading the backend emitters shows the x86 `Pmovmskb128` lowering is a single
`pmovmskb`, while the AArch64 lowering is a NEON reduction and the RISC-V
lowering is a software reduction. **This is emitter inspection, not a native
end-to-end measurement**: the x86 intrinsic headers reject non-x86 targets
with `#error "SSE2/SSE/MMX intrinsics require an x86 target"`, so no C-level
native repro was run on AArch64 or RISC-V. See "Architecture scope" below.

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
`replace-stdlib-shift`, `scalarize`, `fast-math`. `docs/perfguide.rst:325`
("Understanding Gather and Scatter") and `docs/ispc.rst:1368` (`--wno-perf`)
were read for intent, not implementation.

The single most reusable abstraction is ISPC's `MaskStatus` lattice
(`src/llvmutil.h:32`, `GetMaskStatusFromValue`/`GetMaskFromValue` in
`src/llvmutil.cpp:1920`): `{all_on, all_off, mixed, unknown}`, reused by five
different passes. LCCC had no equivalent anywhere.

## Transferability decisions

| ISPC idea | LCCC status | decision |
|---|---|---|
| `MaskStatus` lattice | absent | **TRANSPLANTED** — `MaskStatus::{AllOn,AllOff,Mixed(u32),Unknown}`, `from_lanes`, `from_bytes` |
| constant `__movmsk` → scalar | absent (`constant_fold.rs` has no `Intrinsic` arm) | **TRANSPLANTED** — `Pmovmskb128/256`, `MovemaskPs/Pd128/256` |
| blend with trivial mask → arm | absent | **TRANSPLANTED** — all ten `VecBlendv*` ops, plus identical-arm fold |
| `InstructionSimplify` constant-mask derivation | absent | **TRANSPLANTED (generalised)** — recursive byte-wise vector constant propagation through `Pand/Por/Pxor/Pcmpeqb` (128/256), modern `VecAnd/VecOr/VecXor` SSA forms, and `SetEpi*` splats writing through `dest_ptr` |
| exact constant-object image proof | ISPC has no analogue (LLVM has `ConstantVector`) | **NEW** — needed because LCCC has no vector `IrConst`; see "adaptation" below |
| private-image dead-write reclamation | ISPC relies on LLVM DCE | **NEW** — needed because the constant image lives in dead stack stores |
| all-on/all-off AVX masked load/store | **no IR opcode** (`Instruction` has no gather/scatter/masked-memory variants) | deferred: needs a new opcode family, not a pass |
| `ImproveMemoryOps` stride/broadcast recognition | `VecLoadStrided2*` exists but no pseudo-gather/scatter call to rewrite | deferred: the LLVM analysis operates on pseudo calls LCCC never creates |
| `GatherCoalescePass` | no gather op | deferred |
| `PeepholePass` widen-then-average → `pavgb/pavgw` | `Pavgb128`/`Pavgw128` exist | deferred: an IR-pattern rewrite with a recursion guard, a separate risk class |
| `LowerISPCIntrinsics` sequential-mask pack | vectorizer-specific | deferred |

The deferred items are **not** "not worth it": they are the next transplants,
but each requires a new IR surface and therefore a separate proof.

## What was actually implemented

`src/passes/simd_mask_facts.rs` (pass `maskfacts`, disabled with
`CCC_DISABLE_PASSES=maskfacts`), wired immediately **before** phase 11e
`constarr` in `src/passes/mod.rs`. Order matters: `constarr` replaces the
constant store image with a `.rodata` global, after which the byte-level proof
is unrecoverable. Evidence consumers run before evidence destroyers. A DCE
chaser runs on a positive result, and `verify_after_pass` is invoked for the
new hook (the `check_verify_ir_clean.sh` literal inventory pin was updated in
the same change).

### The adaptation ISPC did not need

ISPC inspects an LLVM `ConstantVector` node. LCCC has only scalar `IrConst`,
so the lattice needs a **constant-object proof**:

* object is a non-volatile, non-semantic-volatile `Alloca`;
* every mention of its address — through `Copy`/`GEP` chains of **any** length,
  including variable offsets (`base_of_value`, memoized, cycle-safe) — is in
  the read's block, at or before the read;
* a `Phi`, `Select`, `Cast`, call, inline asm, atomic, terminator, unknown
  intrinsic or arithmetic mention vetoes the proof. A non-volatile scalar
  `Load` is allowed only when its pointer resolves back into *this* object
  and its read window is inside the object's declared size: no `IrConst` can
  encode a stack address, so a loaded constant cannot be turned into an
  aliasing pointer;
* every write is a scalar `Store` of an `IrConst` whose byte image is exact —
  `to_le_bytes()` of matching width, `IrConst::Zero`, or an integer wider than
  `ty.size()` whose high tail is **uniformly `0x00` or uniformly `0xFF`**
  (zero- or sign-extension; a mixed tail refuses), or a whole-register
  `VecStore*` with a known source image, or a `Memcpy` from a recursively
  proven source window, or a recognized computed-vector write described below;
* a `Memcpy` is refused for self/overlapping copies, zero or oversized
  lengths, or an unproven source; its `dest` is a write and its `src` a read;
* the i686 aggregate-copy idiom (`Load I128` from a constant local, then
  `Store I128`) is followed through bounded `Copy` chains with an explicit
  cycle guard (`MAX_DEPTH`) and a shared work budget;
* the requested window is tiled **exactly once** (gap or overlap → refuse),
  and **no write may extend past the alloca's declared size** — a 32-byte
  splat into a 16-byte object is refused, not silently truncated;
* recursion is bounded (`MAX_DEPTH = 16`, `MAX_WORK = 4096`), shares one
  budget and one cycle set across nested vector expressions (`operand_image`),
  and every `object_image` exit — including rejection — pops its `visiting`
  marker.

`GetElementPtr.offset` is a raw **byte** offset in LCCC
(`src/backend/x86/codegen/isel.rs:1343`, and `const_array_promote` uses it the
same way), so no element scaling is applied.

### Vector constant propagation (second transplant)

ISPC's `InstructionSimplify` derives constant masks from constant vector
operands. LCCC needs more: its computed vectors are *memory writes*, not SSA
constants. `byte_logic_shape` maps 30 legacy/modern opcodes to
`(result byte width, ByteOp)` for the four **commutative, byte-wise**
operations `And`, `Or`, `Xor`, `Eq` (`pcmpeqb` → `0xFF`/`0x00` per byte).
Commutativity removes operand-order hazards; byte-wise semantics remove
lane-width hazards — a `pand` on I8x16, I32x4 or F64x2 produces the identical
16-byte result. Non-commutative compares (`pcmpgtb`) and saturated arithmetic
are deliberately **absent**: their operand order or saturation rule is
backend-specific.

Two integration points:

* `known_vector` handles modern SSA forms (`VecAndI32x4`, ...) whose operands
  are vector values;
* `object_image` handles legacy `dest_ptr` forms (`Pand128`, ...) and
  `SetEpi*` splats. An intrinsic whose `dest_ptr` names **this** object is a
  write with a recursively computed image; one whose `dest_ptr` names a
  *different* object is a pure read of this object and is allowed only when
  every pointer argument resolves to a known stack object. This two-sided
  verdict is what makes the proof terminate instead of rejecting its own
  source images.

`dead_private_writes` treats these intrinsics exactly like `Memcpy`: the
destination write is a deletion candidate, every pointer argument vetoes its
source object, and a bounded fixpoint reclaims the sources in later rounds.

### Correctness decisions that a naive port gets wrong

1. **Little-endian lane sign bits.** `from_lanes` reads bit 7 of the LAST byte
   of each lane. `IrConst::to_le_bytes` uses `to_le_bytes` / `f32::to_bits`,
   so this is exact for I8/I16/I32/I64 and for the IEEE-754 sign bit of
   F32/F64. Verified by the exhaustive 65,536-mask sweep and F32/F64 tests.
2. **Zero-vector widths are checked.** `VecZeroI32x4` is 16 bytes,
   `VecZeroI32x8` is 32. A 128-bit zero cannot satisfy a 256-bit mask:
   legacy SSE leaves the upper YMM half undefined.
3. **Splat lanes must be exact-width.** Zero-extending `I8(-1)` into an I32
   lane gives sign bit 0, sign-extending gives sign bit 1; there is no IR
   evidence of intent, so refuse.
4. **`VecStoreI32x4Pair` is excluded.** It is a half-wide `movq` of the LOW
   EIGHT bytes of a 16-byte XMM source.
5. **Blend masks use `from_bytes`**, a lane-width-independent sufficient
   condition. A well-formed `0x80000000` dword mask is reported `Unknown` —
   conservative, but correct on all ten blend ops.
6. **Two-phase rewrite.** All decisions use one immutable `Facts` snapshot
   (borrowed `&Instruction`, zero clones), so one fold cannot change another
   fold's evidence.
7. **`I32` result type.** `pmovmskb`/`movmskps` produce a 32-bit GPR; the
   all-on 32-lane case is `u32::MAX` whose `i32` bit pattern is `-1`.
8. **Private-image reclamation.** Folding the movemask alone leaves 16
   `movb` + `movdqu` + the prologue — not a win. `prune_dead_private_writes`
   re-derives non-escape from scratch and deletes only writes to objects with
   no reader; `dce` removes orphaned derivations; a bounded fixpoint also
   reclaims SOURCE objects. Span vectors stay 1:1 with instructions.
9. **No early `return None` inside `object_image`'s scan.** Every rejection
   sets a flag and breaks the labelled scan so `visiting.pop()` always runs.

## Architecture scope: what is proved and what is not

The **IR-level proof is backend-independent**: it consumes only `Alloca`,
`Store`, `Load`, `Copy`, `GEP`, `Memcpy` and `Intrinsic` opcodes, produces one
`I32` constant, and deletes the intrinsic before codegen. No assumption about
register widths, `dest_ptr`, ABI or ISA enters the proof, so the four byte
operations are exact on every backend that implements them bitwise.

The **end-to-end C-level proof is narrower**:

| target | C-level repro | result |
|---|---|---|
| x86-64 (`lccc`, `lccc-x86`) | `const_movmsk`, 4-mask family, byte-logic chain | `movl $53069, %eax; ret` or printf immediates; 0 residual `pmovmskb` |
| i686 (`lccc-i686`) | `const_movmsk` | `movl $53069, %eax; ret`, 0 `pmovmskb` (needed the `Load I128`/`Copy`-chain proof) |
| AArch64 (`lccc-arm`) | x86 intrinsic headers | compile refused by the headers' `#error`; **no native proof** |
| RISC-V (`lccc-riscv`) | x86 intrinsic headers | compile refused by the headers' `#error`; **no native proof** |

Therefore: the pass is *designed* to be architecture-independent and its unit
tests are IR-only, but **no claim is made that all four backends were measured
end-to-end**. A target-native proof needs header-independent IR fixtures or
NEON/RVV intrinsic programs, which is future work.

## Verification performed

Mild local checks only; hosted GitHub CI performs full validation (per the
standing instruction). `cargo fmt --all`, `git diff --check`,
`cargo check --lib --profile fastbuild --locked` under `-D warnings`, focused
`cargo test --lib ... simd_mask_facts`, strict Clippy policy checker, and
`build_lccc_fast.sh` all returned RC0 at each round. Focused test counts grew
22 → 27 → 29 → 30 → **34 passed / 0 failed / 8257 filtered**.

Unit coverage: 65,536 exhaustive 16-lane byte masks with arbitrary low-7-bit
noise; 32-lane no-shift-overflow; F32/F64 sign-bit masks; all ten blend ops;
zero-vector width mismatch; exact-width splat lanes; half-wide pair store;
phi agreement/disagreement; copy cycles; narrow/zero/sign-extended stores;
`Memcpy`-backed PS/PD images and negative `Memcpy` cases; i686 `I128`
aggregate copies and bounded `Copy` chains; unsound-image refusals (gap,
overlap, non-constant/volatile store, semantic-volatile alloca,
arithmetic/variable GEP, unknown intrinsic, undersized object, out-of-bounds
read, post-read and cross-block writes, write past alloca size); escaping
address; deep 40-link GEP derivation; terminator escape; span parallelism and
idempotence; and, for the new propagation, an independent four-op oracle over
non-trivial bytes plus refusals for wrong arity, missing `dest_ptr`,
unresolved source, volatile source, non-commutative compare, width mismatch,
widened splat lane, runtime splat operand and 32-byte splat in a 16-byte
object.

Five C fixtures were added under `tests/regression/` (auto-discovered by
`run_regression.py`, no new CI gate wiring): constant movemask, byte-logic
chain, three movemask widths, a runtime-valued control that must *not* fold,
and an exhaustive 256-splat differential. All five produce byte-identical
stdout under LCCC, GCC 16.2 and Clang 23.1.5; the five-test filtered
regression run reported **5 passed / 0 failed**.

A/B controls with `CCC_DISABLE_PASSES=maskfacts`: the byte-logic repro keeps
19 vector/stack instructions, the four-mask repro keeps 52, and `const_movmsk`
keeps 11 including `pmovmskb`.

## Per-function efficiency verdict

Measured on the x86-64 fastbuild compiler, `-O2`. "Efficient" means the
function is reduced to the same immediates GCC 16.2 / Clang 23.1.5 emit.

| function / kernel | before | after | GCC 16.2 / Clang 23.1.5 | verdict |
|---|---|---|---|---|
| `const_movmsk` (`Pmovmskb128`) | 16 `movb` + `movdqu` + `pmovmskb` + frame | `movl $53069, %eax; ret` | identical | **efficient — parity** |
| `const_movmsk` i686 | `pmovmskb` + `I128` aggregate traffic | `movl $53069, %eax; ret` | identical | **efficient — parity** |
| four-movemask printf kernel | 52 vector/byte instructions, `Memcpy` chains | 3 immediates + `printf` call | identical | **efficient — parity** |
| `and` + `cmpeq` byte-logic kernel | 19 vector/stack instructions | 2 immediates + `printf` call | identical | **efficient — parity** |
| runtime-valued splat sweep | not foldable (correctly) | unchanged | identical | **efficient — no regression, no unsound fold** |
| AArch64 / RISC-V native | header-rejected | header-rejected | n/a | **not measured; no verdict** |

## Explicitly NOT claimed

* No general "LCCC beats GCC/Clang/ICX" claim. Measured parity is limited to
  the constant-SIMD-mask kernels above.
* No local ICX 2026.2 result (PPA unreachable, no compiler asset upstream).
* No native AArch64 or RISC-V end-to-end result; see "Architecture scope".
* No local CI run, no full local unit suite in this change, no
  GDB/objtool/pahole/valgrind/perf campaign. Local checks are the mild ones
  listed under "Verification performed".
* No claim that the deferred ISPC passes (gather coalescing, masked-memory
  ops, widen-then-average peephole) were implemented.
