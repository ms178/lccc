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
| `MaskStatus` lattice | absent | **TRANSPLANTED** — `MaskStatus::{AllOn,AllOff,Mixed(u32),Unknown}`, single classification entry point `from_lanes` (lane-width aware) |
| constant `__movmsk` → scalar | absent (`constant_fold.rs` has no `Intrinsic` arm) | **TRANSPLANTED** — `Pmovmskb128/256`, `MovemaskPs/Pd128/256` |
| blend with trivial mask → arm | absent | **TRANSPLANTED** — all ten modern `VecBlendv*` ops (lane-exact selectors) **plus** the four legacy `Blendv*128/256` `dest_ptr` ops emitted by the C frontend; `Pblendvb128` excluded on argument order |
| `InstructionSimplify` constant-mask derivation | absent | **TRANSPLANTED (generalised)** — recursive byte-wise vector constant propagation through `Pand/Por/Pxor/Pcmpeqb` (128/256), modern `VecAnd/VecOr/VecXor` SSA forms, and `SetEpi*` splats writing through `dest_ptr` |
| `ImproveMemoryOps` proof ordering | n/a | **NEW** — only writes are order-sensitive; a proven pure read after the movemask no longer invalidates an earlier image |
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

### Lane-aware blends and the legacy C-frontend path (third transplant)

ISPC's `InstructionSimplify` folds a trivial `Select`/blend when the mask is
`AllOn`/`AllOff`. The first LCCC implementation covered only the modern SSA
`VecBlendv*` intrinsics. An end-to-end probe showed that was not enough:

```c
__m128 a = _mm_setr_ps(1,2,3,4), b = _mm_setr_ps(5,6,7,8);
__m128 m = _mm_castsi128_ps(_mm_set1_epi32((int)0x80000000u)); /* all-on */
printf("%g %g %g %g\n", _mm_blendv_ps(a,b,m) ...);            /* 5 6 7 8 */
```

`LCCC_DUMP_IR=1` proved the C frontend emits the **legacy** form

```
Intrinsic { dest: Some(Value(105)), op: BlendvPs128,
            dest_ptr: Some(Value(104)),
            args: [Value(103), Value(101), Value(102)] }
```

i.e. `args = [mask, false_arm, true_arm]` (confirmed at
`src/backend/x86/codegen/intrinsics_simd.rs:1758` for 128-bit and `:2036` for
256-bit). `Pblendvb128` is deliberately **excluded** because its argument
order is `[a, b, mask]` (`intrinsics.rs:3322`).

`legacy_blend_shape` therefore maps only `BlendvPs128 -> (16, 32)`,
`BlendvPd128 -> (16, 64)`, `BlendvPs256 -> (32, 32)` and
`BlendvPd256 -> (32, 64)`. When `from_lanes` says `AllOff`/`AllOn`, `fold`
rewrites the intrinsic into a single `Instruction::Memcpy` of 16/32 bytes
from the selected arm, **iff** the intrinsic's SSA `dest` is dead. `Facts::used`
is a hoisted, shared SSA-use set used by both `fold` and
`dead_private_writes`; this closes the dangling-SSA hole, because
`Instruction::Intrinsic::used_values()` covers `dest_ptr` and `args` but NOT
`dest`. `intrinsic_mention` proves the selected arm's byte image so a later
movemask can still fold. `has_candidate` and `dead_private_writes` both
include legacy blend shapes.

### Write-order proof: a pure read cannot invalidate an image (fourth transplant)

The original `object_image` required **every** mention of the object to be in
the read's block and at or before the read. Probing a two-movemask expression
revealed this was over-conservative:

```c
return _mm_movemask_epi8(_mm_and_si128(a,b))
     + _mm_movemask_epi8(_mm_cmpeq_epi8(a,b));
```

Only the second movemask folded (`add $0, %eax`); the first kept `pand` +
`pmovmskb`. The hook-time IR dump showed `Pcmpeqb128` at index 16 reading the
shared source object while the first `Pmovmskb128` was at index 14 — so
`object_image` rejected on `index > read_index` even though the mention is a
**proven pure read**. The fix makes only writes order-sensitive:

```rust
match self.mention_of(inst, w, budget, visiting) {
    ObjectMention::NoWrite => {}          // harmless anywhere in the CFG
    ObjectMention::Write(off, bytes) => {
        if block_id != read_block || index > read_index {
            rejected = true;
            break 'scan;
        }
        writes.push((off, bytes));
    }
    ObjectMention::Reject => { rejected = true; break 'scan; }
}
```

A `NoWrite` verdict means `mention_of` proved the instruction is a read or an
address derivation with respect to `base`; it cannot change the image. A
`Write` in another block (unproven execution order) or after the read still
refuses. `a_pure_read_after_the_movemask_does_not_invalidate_an_earlier_image`
asserts both folds fire (`run_function == 2`). End-to-end `logic_kernel` went
from 24 to **3 instructions** (`movl $53069,%eax; addl $0,%eax; ret`).

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
5. **Blend selectors are classified by LANE, not by byte.** `blend_shape`
   returns `(mask byte width, selector lane bits)` for all ten modern
   `VecBlendv*` ops: 32-bit lanes for `vblendvps`, 64-bit for `vblendvpd`,
   8-bit for `vpblendvb`. `MaskStatus::from_lanes` reads bit 7 of the LAST
   byte of each lane (`x86` sign-bit semantics, little-endian). The old
   byte-wise `from_bytes` sufficient condition refused a well-formed
   `0x80000000` dword selector because its three low payload bytes are
   don't-care; it has been removed. `dword_blend_with_dont_care_payload_bytes_folds_and_byte_blend_refuses`
   pins that a 32-bit-lane blend folds while the SAME bytes under an 8-bit
   byte-blend are `Mixed`/`Unknown` and must refuse.
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

## Tool-driven audit (GDB 18.1 / objdump 2.47 / readelf 2.47 / nm)

The standing request asked for the best available data-driven tools.
`objtool`, `pahole`, `valgrind`, `perf`, Miri and Callgrind are **absent** from
this image; GDB, `objdump`, `readelf` and `nm` are present and were used. No
claim is made for tools that were not run.

**GDB `disassemble main` on the byte-logic repro**, built at `-O2 -g`:

| binary | asm lines in `main` | vector/stack ops | immediate loads |
|---|---|---|---|
| maskfacts enabled | 9 | 0 | 1 |
| `CCC_DISABLE_PASSES=maskfacts` | 34 | 19 | 1 |

**`readelf -S -W` `.text` size**: 272 bytes enabled vs 408 bytes disabled.
**`objdump -d` whole-binary instruction count**: 90 vs 119.
**`objdump -d` `logic_kernel` in the 100M-call benchmark**: 4 instructions
enabled vs 35 disabled.

**GDB/`-S` derived instruction table** — asm instruction lines in the named
function at `-O2` (counted by a script over the emitted `.s` body, not by
eyeballing). `fx6`/`fx7` also need `-msse4.1`:

| kernel / function | LCCC | GCC 16.2 | Clang 23.1.5 | verdict |
|---|---|---|---|---|
| `one.c` / `const_movmsk` | 2 | 2 | 2 | parity |
| `builtins.c` / `main` | 11 | 11 | 11 | parity |
| `logic.c` / `main` | 9 | 9 | 9 | parity |
| `fx7.c` / `main` (movemask of all-on blend) | 8 | 8 | 8 | parity |
| `kern.c` / `logic_kernel` | 3 | 2 | 2 | parity modulo one redundant `add $0` |
| `fx6.c` / `main` (four float extracts) | 32 | 11 | 11 | **remaining gap — see below** |

The `fx6.c` gap is real and is NOT claimed as fixed. maskfacts removes the
`blendvps` (enabled 32 instructions vs 46 with the pass disabled), but LCCC
then materializes the constant vector into a stack object and re-loads four
`float`s through `movss`/`cvtss2sd`, while GCC/Clang keep them in registers.
That is a **scalar constant-propagation / SROA** gap in a different pass, not
a mask-lattice gap. It is recorded here so it is not silently left behind.

### Host-CPU runtime A/B (measured, not estimated)

Intel Xeon @ 2.60 GHz, 2 online CPUs, `CLOCK_MONOTONIC`, 100,000,000 calls to
a `noinline` byte-logic kernel, `volatile` sink to prevent dead-loop
elimination. Alternating order, 3 runs each; `sink = -1679577856` on every run.

| binary | median total | median ns/call |
|---|---|---|
| LCCC, maskfacts enabled | 92.90 ms | **0.929 ns** |
| LCCC, `CCC_DISABLE_PASSES=maskfacts` | 1063.14 ms | **10.631 ns** |

That is an **11.4x speed-up of LCCC-with-pass vs LCCC-without-pass on this one
kernel**. It is NOT a claim about other compilers: GCC 16.2 and Clang 23.1.5
both **eliminate the entire benchmark loop** (0.0001 ms), because they hoist
the constant kernel out of the loop. LCCC still has that separate loop-level
missed optimization; it is documented as an open gap.

## Hosted CI: red -> green (the escalation that started this round)

`9e1ffba` was RED. Triage via `gh api .../actions/jobs/<id>` (the
`--log-failed` and zip-log routes both failed: EOF / HTTP 415) isolated the
two real failures to the **`scripts/audit_census.py --check` function-length
ratchet**: `fns_over_300 233 -> 234`, caused by a 446-line `object_image`.
`Test Suite` was only the roll-up. The fix was NOT to re-baseline: `9129c09`
refactored `object_image` into `ObjectMention` / `ImageWindow` plus
`tile_image`, `mention_of`, `memcpy_mention` and `intrinsic_mention` (largest
function now 203 lines), made `visiting.pop()` unconditional, and wired the
dangling-SSA guard. Hosted run `42370415640` then reported
**22 pass / 3 skipping / 0 fail / 0 pending**, including
`Authenticated Intel ICX 2026.2 integration`, `Clippy`, both fast-contract
shards, both codegen/runtime shards, all three frontend/ABI shards, Debian
GCC 16.2 / Rust 1.99, benchmark corpus and `Test Suite`.

The lane-aware blend, legacy blend, write-order fix and fixtures in this
revision are **additional** to that green commit and require their own hosted
run; no green claim is made for them until that run is observed.

## Verification performed

Only mild local checks, per the standing instruction; **no local CI was run**.
`cargo fmt --all`, `git diff --check`, `cargo check --lib --profile fastbuild
--locked` under `-D warnings`, focused `cargo test --lib ... simd_mask_facts`,
`scripts/audit_census.py --check`, `tests/regression/check_param_home_contracts.py`,
`tests/regression/check_verify_ir_clean.sh`, the strict Clippy policy checker,
`tests/regression/run_regression.py --filter simd_mask_facts` and
`scripts/build_lccc_fast.sh` are the gates used. Focused test counts grew
22 -> 27 -> 29 -> 30 -> 34 -> 36 -> **42 passed / 0 failed / 8257 filtered**.

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
idempotence; four-op byte-logic oracle plus refusals; **lane-exact selector
classification** (dword don't-care payload vs byte blend); **legacy blend**
all-off/all-on rewrites, mixed-mask refusal, live-SSA-dest refusal and
`Pblendvb128` exclusion; **two independent proofs sharing one source** with
a pure read after the earlier movemask; and a rejected proof never poisoning
a later independent proof.

**Seven** C fixtures were added under `tests/regression/` (auto-discovered by
`run_regression.py`, no new CI gate wiring): constant movemask, byte-logic
chain, three movemask widths, a runtime-valued control that must *not* fold,
an exhaustive 256-splat differential, the legacy all-on dword blend, and a
movemask-of-all-on-blend. All seven produce byte-identical stdout under LCCC,
GCC 16.2 and Clang 23.1.5; the filtered regression run reported
**7 passed / 0 failed**.

A/B controls with `CCC_DISABLE_PASSES=maskfacts`: the byte-logic repro keeps
19 vector/stack instructions, the four-mask repro keeps 52, `const_movmsk`
keeps 11 including `pmovmskb`, the 100M-call benchmark kernel keeps 35
instructions, and `fx6.c` keeps 1 `blendvps` (0 with the pass enabled).

## Pass / fail table

| acceptance item | status | evidence |
|---|---|---|
| CI red corrected | **PASS** | `9129c09`, hosted run `42370415640`: 22 pass / 3 skipping / 0 fail |
| census ratchet without re-baselining | **PASS** | `fns_over_300` back to 233; max function 203 lines |
| ISPC concepts transplanted | **PASS** (4) | `MaskStatus`, instruction-simplify on byte logic, trivial-blend fold, improve-memory-ops constant propagation; plus node-level proof order fix |
| line-by-line red-team of the new pass | **PASS** | 42 focused unit tests, refusals and exhaustive lattice sweep |
| measured x86-64 / i686 kernel parity | **PASS** | GDB/`-S` table above, 7 C fixtures, 3-way stdout |
| GDB / objdump / readelf / nm audit | **PASS** | tool-derived counts and `.text` sizes above |
| host-CPU runtime A/B | **PASS** | 0.929 ns vs 10.631 ns per call, identical sink |
| all supported architectures folding `one.c` | **FAIL / NOT PROVEN** | AArch64/RISC-V reject x86 headers; no target-native proof |
| `objtool` / `pahole` / `valgrind` / `perf` audit | **FAIL / UNAVAILABLE** | tools absent from image; substitutes used, no claim made |
| local ICX 2026.2 comparison | **FAIL / UNAVAILABLE** | PPA unreachable, no compiler asset; hosted ICX lane passes |
| GCC/Clang loop-invariant hoisting parity | **FAIL** | GCC/Clang remove the whole benchmark loop; LCCC does not |
| `fx6.c` scalar constant-propagation parity | **FAIL** | 32 vs 11 instructions; diagnosed, different pass |
| broad "beats GCC/Clang/ICX" claim | **NOT CLAIMED** | only measured parity on the listed kernels |
| local CI | **NOT RUN** | explicitly forbidden; hosted CI is the authority |

## Per-function efficiency verdict

Measured on the x86-64 fastbuild compiler, `-O2`. "Efficient" means the
function is reduced to the same immediates GCC 16.2 / Clang 23.1.5 emit.

| function / kernel | before | after | GCC 16.2 / Clang 23.1.5 | verdict |
|---|---|---|---|---|
| `const_movmsk` (`Pmovmskb128`) | 16 `movb` + `movdqu` + `pmovmskb` + frame | `movl $53069, %eax; ret` | identical | **efficient — parity** |
| `const_movmsk` i686 | `pmovmskb` + `I128` aggregate traffic | `movl $53069, %eax; ret` | identical | **efficient — parity** |
| four-movemask printf kernel | 52 vector/byte instructions, `Memcpy` chains | 3 immediates + `printf` call | identical | **efficient — parity** |
| `and` + `cmpeq` byte-logic kernel | 19 vector/stack instructions | 2 immediates + `printf` call | identical | **efficient — parity** |
| two-movemask expression `logic_kernel` | 35 instructions (`pand`+`pmovmskb`+…) | 3 instructions | 2 | **efficient — parity modulo one redundant `add $0`** |
| `_mm_blendv_ps` all-on dword (`fx6.c`) | 46 instructions incl. `blendvps` | 32 instructions, **0 `blendvps`** | 11 | **partially efficient — blend eliminated, scalar-propagation gap remains** |
| movemask of all-on blend (`fx7.c`) | `blendvps` + `movmskps` | 8 instructions, immediate | identical | **efficient — parity** |
| runtime-valued splat sweep | not foldable (correctly) | unchanged | identical | **efficient — no regression, no unsound fold** |
| 100M-call benchmark kernel | 35 instructions / 10.631 ns | 4 instructions / 0.929 ns | loop removed | **efficient for LCCC; GCC/Clang still better via loop hoisting** |
| AArch64 / RISC-V native | header-rejected | header-rejected | n/a | **not measured; no verdict** |

## Explicitly NOT claimed

* No general "LCCC beats GCC/Clang/ICX" claim. Measured parity is limited to
  the constant-SIMD-mask kernels listed above.
* No local ICX 2026.2 result (PPA unreachable, no compiler asset upstream).
* No native AArch64 or RISC-V end-to-end result; see "Architecture scope".
* No native `objtool`, `pahole`, `valgrind`, `perf`, Miri or Callgrind result;
  those binaries are absent. GDB, `objdump`, `readelf`, `nm`, `pyelftools`
  0.33, `capstone` 5.0.7 and `rustc -Zprint-type-sizes` WERE used as explicit,
  labelled substitutes. The Intel toolchain could NOT be fetched locally:
  `intel/llvm@nightly-2026-11-07`'s `sycl_linux.tar.gz` (242 MB) redirects to
  `release-assets.githubusercontent.com`, which the sandbox egress allowlist
  rejects (`curl (35) SSL_ERROR_SYSCALL`, `http=302`); the Intel PPA is
  likewise unreachable. The hosted authenticated ICX 2026.2 lane below is
  the ONLY ICX reference, and it is not claimed as a local result.
* No local CI run and no full local unit suite in this change. Hosted CI is
  the authority; the additional work in this revision is still awaiting its
  own hosted run.
* No claim that GCC/Clang loop-invariant hoisting or LCCC scalar
  constant-propagation through vector extracts was fixed. Both are measured
  open gaps.
* No claim that the deferred ISPC passes (gather coalescing, masked-memory
  ops, widen-then-average peephole) were implemented: LCCC has no
  gather/scatter/masked-memory IR opcodes yet.

---

# Round 3 escalation: "do even better"

This round started from a **green** CI run (`42370415640`, 22 pass / 3
skipping / 0 fail) and did NOT stop there. A line-by-line red-team of all
1,900 lines of the pass found two real defects and one quadratic design flaw;
all three were fixed, and two further ISPC concepts were transplanted. Every
claim below is backed by a tool-derived measurement.

## Defects found and fixed (not guessed)

| # | defect | severity | fix |
|---|---|---|---|
| 1 | `fold` rewrote a legacy `Blendv*` to a 16/32-byte `Memcpy` after proving only the MASK image; the SELECTED ARM could be a live runtime value in a SMALLER object — an unchecked buffer over-read | **soundness** | `window_in_bounds` proves BOTH dest and src windows are in-bounds plain non-volatile allocas, and refuses `dest_base == src_base` (memcpy overlap is not a value proof). Two new refusal tests. |
| 2 | `object_image` rescanned EVERY instruction of the function for EVERY read — O(n²) analysis in the optimizer; adding load consumers would have made this worse | **compiler performance** | `root_of` + `build_mention_index` build a per-alloca `(block, index)` mention table, escape set and constant-write hint in ONE linear pass. `object_image` audits only that object's k mentions: O(n + k). `dead_private_writes` reuses the same helper. |
| 3 | `dead_private_writes` budget `break` exited only the inner instruction loop, so a function with >4096 instructions kept walking later blocks at zero work; `prune_dead_private_writes` had no early-exit on a zero-progress round | **wasted cycles** | labelled `break 'outer`, plus `if removed == 0 { break; }` fixpoint convergence. |
| 4 | duplicated/stale doc comments: `object_image` had two doc blocks, and `legacy_blend_shape`'s documentation was attached to `blend_shape` | **technical debt** | split and corrected; module doc now lists all FIVE consumers and the correct write-order rule. |

Measured compiler-time impact of the new index, on a generated function with
1,000 scalar loads, 5 alternating runs each (Python `perf_counter`):

| configuration | median | min | max |
|---|---|---|---|
| pass enabled | 170.15 ms | 168.67 ms | 179.86 ms |
| `CCC_DISABLE_PASSES=maskfacts` | 171.63 ms | 167.74 ms | 177.33 ms |

Overhead: **-1.48 ms (within noise)**. The `maybe_constant_load` O(MAX_DEPTH)
gate means ordinary loads never pay for a full image proof.

## Two more ISPC transplants

### `ImproveMemoryOps`: load-of-proven-constant

A non-volatile, default-address-space scalar `Load` from a proven constant
window becomes a bit-exact `IrConst` via `const_from_bytes`. That function
round-trips the constructed constant through `const_store_bytes` at the
load's declared width and **refuses if the bytes differ** — this is what makes
LCCC's unsigned spelling safe (`U8`/`U16`/`U32` are zero-extended `IrConst::I64`
per `constants.rs:640`; a narrow `I32(-1)` spelling would turn `U32(0xFFFFFFFF)`
into `-1`). `Ptr`, `F128` and C23 BID decimal containers (`D32`/`D64`) are
refused: their `Copy { Const }` lowering is target-specific and unproven.
`-0.0`, non-canonical NaN payloads and `U64` high-bit patterns are pinned by
unit tests.

### Ordering fix: `constant_fold` chaser

`maskfacts` is scheduled at phase 11e-pre, AFTER every `constant_fold::run`.
Constants created here therefore flowed into `Cast { Const(F32(5.0)), F32 -> F64 }`
that no pass could fold — measured as `movss .LCFP + cvtss2sd` in `fx6.c`.
`run` now re-runs the canonical `constant_fold::fold_function` after any
successful fold. No new verify hooks, no pipeline change, idempotent. Result:
**`fx6.c` went from 32 to 11 instructions — exact parity with GCC 16.2 and
Clang 23.1.5, with 0 `blendvps` and 0 `cvtss2sd`** (disabled-pass control:
46 instructions, 1 `blendvps`, 4 `cvtss2sd`).

## Final measured table (fastbuild, `-O2`, 3-way)

Asm instruction lines in the named function; `off` = `CCC_DISABLE_PASSES=maskfacts`.

| kernel | LCCC | off | GCC 16.2 | Clang 23.1.5 | `blendvps` off/on | `cvt` off/on |
|---|---|---|---|---|---|---|
| `one.c` / `const_movmsk` | **2** | 10 | 2 | 2 | 0/0 | 0/0 |
| `kern.c` / `logic_kernel` | **2** | 28 | 2 | 2 | 0/0 | 0/0 |
| `logic.c` / `main` | **9** | 34 | 9 | 9 | 0/0 | 0/0 |
| `fx6.c` / `main` (all-on dword blend) | **11** | 46 | 11 | 11 | 1/0 | 4/0 |
| `fx7.c` / `main` (movemask of blend) | **8** | 33 | 8 | 8 | 1/0 | 0/0 |
| `loads.c` / `main` (scalar constant loads) | **18** | 29 | 16 | 18 | 0/0 | 0/0 |

`loads.c` is 2 instructions behind GCC (parity with Clang) — a register-allocation
difference, recorded honestly. All six kernels produce byte-identical stdout
under LCCC, GCC 16.2 and Clang 23.1.5; `builtins.c` at `-O2 -mavx2` prints
`53069 5 1 817024845` on all three.

### Host-CPU runtime A/B (final binary)

100,000,000 `noinline` calls, alternating order, 6 runs per configuration,
`sink = -1679577856` on every run. Median **~0.94 ns/call enabled vs
~10.7 ns/call disabled — ~11.4x** for LCCC-with-pass vs LCCC-without-pass on
this one kernel. GCC 16.2 and Clang 23.1.5 eliminate the whole loop
(0.0001 ms); LCCC's loop-invariant hoisting remains an OPEN gap.

## Tool-driven audit, round 3

`objtool`, `pahole`, `valgrind`, `perf` are absent; these substitutes were
installed from `pypi.org` and actually RUN:

* **`pyelftools` 0.33 + `capstone` 5.0.7** on the A/B ELF binaries:
  `audit_on` `.text` 297 B / 81 decoded instructions / **0 undecodable
  non-zero bytes** / 0 `blendvps` / 0 `pmovmskb` / 0 `cvtss2sd`;
  `audit_off` `.text` 540 B / 116 instructions / 1 `blendvps` / 4 `cvtss2sd`.
  Both: ELF64 `EM_X86_64` `ET_EXEC`, 30 sections, 41 symbols, entry
  `0x401450`. Independent instruction decoding confirms no invalid opcodes.
* **`rustc -Zprint-type-sizes`** (`RUSTC_BOOTSTRAP=1`, the `pahole`
  equivalent for Rust): `Facts<'_>` = 168 B / align 8, `ImageWindow` =
  32 B / align 8, `ObjectMention` = 32 B / align 8, `MaskStatus` = 8 B /
  align 4, `ByteOp` = 1 B / align 1. `ImageWindow` is `Copy` and passed by
  value to every mention classifier — deliberate: a borrow would thread a
  lifetime through four recursive functions for 32 bytes.
* **GDB 18.1 / objdump 2.47 / readelf 2.47 / nm**: earlier round's 9 vs 34
  `main` asm lines, 272 vs 408 `.text` bytes, 90 vs 119 whole-binary
  instructions.
* **4 GB swap**: `/proc/swaps` shows `/tmp/lccc-ci.swap` 4194300 KB and
  `/swapfile` 4194300 KB, both active. fastbuild preset used throughout.

### Native AArch64 / RISC-V codegen (target-neutral source)

A C program with NO x86 headers and a 16-byte constant mask reduction compiles
on all four drivers. `-O2 -S` output:

| target | assembly |
|---|---|
| x86-64 | `movl $53069, %eax; ret` |
| AArch64 (`lccc-arm`) | `mov x0, #53069; ret` |
| RISC-V (`lccc-riscv`) | `li a0, 53069; ret` |

An A/B with `CCC_DISABLE_PASSES=maskfacts` shows the SAME fold on all three
targets, so the owner is `constarr`/`constant_fold`, **not** `maskfacts`. This
is therefore native proof that the constant-object machinery and all three
backends agree on `53069`; it is **NOT** a claim that `maskfacts` folds a
movemask on AArch64/RISC-V. `check_param_home_contracts.py` with all four
CCC drivers reported `[1a]`–`[13]` PASS, including `[5b]` "all four drivers
emit deterministic, panic-free ELF objects" and `[9]` "-g does not change code
... on x86-64/i686/AArch64/RISC-V at -O0/-O2"; `check_verify_ir_clean.sh`
passed on 79 files with `CCC_VERIFY_IR=abort` and 38 pinned
`verify_after_pass` hooks. No target-native SIMD-movemask source path exists
on ARM/RISC-V (the x86 builtin is rejected by those frontends), so that
specific claim remains unproven.

### Hosted ICX 2026.2

Run `42370415640`'s `Authenticated Intel ICX 2026.2 integration` job
`128398075708` passed 12 real differential steps, including "Numeric -O and
-Ofast parity against ICX 2026.2, GCC 16.2 and Clang 23.1.5", "SIMD folded-index
correctness against authenticated ICX", "SIMD scoreboard checksums against
authenticated ICX", "SSE1-only packed F32 workload against ICX" and "FP select
min/max matrix against ICX". That is the ICX reference. A local ICX was
attempted and is impossible here (see the egress evidence above); no local
ICX result is claimed.

## Verification, round 3

| check | result |
|---|---|
| `cargo fmt --all -- --check` | RC0 |
| `git diff --check` | RC0 |
| `cargo check --lib --profile fastbuild --locked` (`-D warnings`) | RC0 |
| focused `cargo test --lib ... simd_mask_facts` | **50 passed / 0 failed / 8257 filtered** |
| `scripts/audit_census.py --check` | RC0 — `fns_over_300` = 233 baseline, max function 203 lines |
| `scripts/check_clippy_policy.py` (strict) | PASS, 76 lints probed, 0 unknown |
| `tests/regression/check_verify_ir_clean.sh` | RC0, 79 files, 38 hooks pinned |
| `tests/regression/check_param_home_contracts.py` (4 drivers) | RC0, `[1a]`–`[13]` PASS |
| `run_regression.py --filter simd_mask_facts` | **8 passed / 0 failed** |
| `scripts/build_lccc_fast.sh` | RC0 |
| 3-way runtime + asm | byte-identical stdout / parity counts above |

New tests: `const_from_bytes` bit-exactness over every supported scalar type
(including exhaustive U8 domain and U32 boundaries), NaN payloads, `-0.0`,
`U64` high bits; refusals for Ptr/F128/Void/D32/D64/mismatched widths;
load folds for U32/U8/F32/U64 and refusals for volatile, non-default address
space, runtime store, volatile alloca, out-of-bounds GEP, terminator escape,
gap inside the read window, cross-block write; a partially tiled image
OUTSIDE the requested window correctly still folds; and the two
`Memcpy`-bounds soundness regressions. Fifty tests total.

## Updated pass / fail

| acceptance item | status |
|---|---|
| CI red corrected | **PASS** — hosted `42370415640`, 22 pass / 3 skipping / 0 fail |
| census ratchet without re-baselining | **PASS** — 233 held, max function 203 lines |
| full line-by-line red-team, every found defect fixed | **PASS** — 4 defects above, all with regression tests |
| ISPC concepts transplanted | **PASS** (6) — lattice, movemask, blends (modern + legacy), byte-logic propagation, private-image reclamation, load-of-constant |
| O(n²) optimizer analysis removed | **PASS** — mention index; measured -1.48 ms (noise) on 1000-load function |
| measured x86-64 kernel parity | **PASS** — 5 of 6 kernels exactly equal GCC/Clang counts |
| new regression tests | **PASS** — 50 unit + 8 C fixtures |
| GDB / objdump / readelf / nm / pyelftools / capstone / `-Zprint-type-sizes` | **PASS** — real measurements above |
| host-CPU runtime A/B | **PASS** — ~11.4x LCCC-with vs LCCC-without, identical sink |
| 4-architecture gate coverage | **PASS** — native ARM/RISC-V codegen + 4-driver contract gates |
| `maskfacts` folding a movemask natively on AArch64/RISC-V | **FAIL / NOT PROVEN** — no target-neutral source path |
| native `objtool` / `pahole` / `valgrind` / `perf` | **FAIL / UNAVAILABLE** — substitutes used, labelled |
| local ICX 2026.2 | **FAIL / UNREACHABLE** — asset CDN blocked; hosted ICX lane passes |
| GCC/Clang loop-invariant hoisting parity | **FAIL** — open gap, different pass |
| `loads.c` 2-instruction gap vs GCC | **FAIL** — register allocation, recorded |
| broad "beats GCC/Clang/ICX" claim | **NOT CLAIMED** |
| local CI | **NOT RUN** — explicitly forbidden |

## Updated per-function efficiency verdict

| function | verdict |
|---|---|
| `const_movmsk` (x86-64 / i686 / AArch64 / RISC-V codegen) | **efficient — parity** (`movl $53069` / `mov x0,#53069` / `li a0,53069`) |
| `logic_kernel` (two movemasks, one expression) | **efficient — parity 2/2/2**, redundant `add $0` removed by the chaser |
| byte-logic `and`+`cmpeq` kernel | **efficient — parity 9/9/9** |
| `_mm_blendv_ps` all-on dword (`fx6.c`) | **efficient — parity 11/11/11**; gap from round 2 CLOSED |
| movemask of all-on blend (`fx7.c`) | **efficient — parity 8/8/8** |
| scalar constant loads (`loads.c`) | **efficient — parity with Clang, +2 ins vs GCC** |
| 100M-call benchmark kernel | **efficient for LCCC (11.4x)**; GCC/Clang still better via loop hoisting |
| runtime-valued splat sweep | **efficient — no regression, no unsound fold** |
| `object_image` analysis | **efficient** — O(n + k), measured zero overhead |
| AArch64 / RISC-V native maskfacts movemask fold | **not measured; no verdict** |

---

# Round 4: a real CI-caught miscompile, found and fixed

The first round-3 push (`4d988cb`) turned hosted CI **RED** on
`SSA regression corpus` (run `42377164468`, job `128417821627`, step 17).
The annotation named the fixture: `sse34_encoding_emission`,
`run failed (rc=3)`. This was NOT a flake and NOT a pre-existing failure:
the same fixture passes with `CCC_DISABLE_PASSES=maskfacts` and fails with the
pass enabled. **My change caused it.** It is recorded here rather than hidden.

## Root cause (IR-diff driven, not guessed)

`LCCC_DUMP_IR=1` A/B of the fixture:

```diff
-    Load { dest: Value(251), ptr: Value(573), ty: I128, ... }
-    Load { dest: Value(253), ptr: Value(574), ty: I128, ... }
-    InlineAsm { template: "pcmpistri $0,%2,%1",
-                inputs: [("x", Value(Value(251)), None),
-                         ("xm", Value(Value(253)), None)], ... }
+    InlineAsm { template: "pcmpistri $0,%2,%1",
+                inputs: [("x",  Const(I128(97)),      None),
+                         ("xm", Const(I128(6387832)), None)], ... }
```

The new `ImproveMemoryOps` load fold correctly proved both 16-byte constant
images and replaced the `Load I128`s with `Copy { Const(I128) }`. The new
`constant_fold` chaser then propagated those constants into the `InlineAsm`
`"x"` inputs. The x86 backend cannot materialise a 128-bit `IrConst` in that
context:

```diff
-    movdqu (%r12), %xmm0 ; movdqu (%rbx), %xmm0 ; ...
+    movl $0, %eax ; movq %rax, %xmm0
+    movl $0, %eax ; movq %rax, %xmm1
     pcmpistri $0,%xmm1,%xmm0
```

Both XMM operands became ZERO, so `pcmpistri` returned the wrong index and
the C witness returned 3. **The IR-level fold was sound; the backend constant
materialisation for wide asm operands is not.** That is the pre-existing
limitation the new consumer exposed.

## The fix (fail-closed, two independent guards)

1. **`const_from_bytes` and `fold_load` refuse widths > 8 bytes.** Byte, word,
   dword and qword loads are the ones proven end-to-end by `fx6.c` (F32) and
   `loads.c` (U8/U32/U64). `I128`/`U128` are out of scope. `fold_load` also
   exits before paying for an image proof on a wide load — no wasted cycles.
   The i686 aggregate-copy path is unaffected: it uses `known_scalar_bytes`
   and `IrConst::I128` directly, not `const_from_bytes`.
2. **The `constant_fold` chaser is now scoped and guarded.** It moved from
   `run` (where it touched EVERY function in the module) into `run_function`,
   so it runs only on a function this pass actually edited, and it is
   **skipped entirely when the function contains `Instruction::InlineAsm`**.
   Inline asm is opaque to a late optimizer; never re-fold it on the strength
   of a constant created at phase 11e-pre.

Two new unit tests pin the regression: `const_from_bytes` refusal for
I128/U128, and `wide_loads_are_not_folded_and_inline_asm_functions_are_not_refolded`.

## Verification of the fix

| check | result |
|---|---|
| `sse34_encoding_emission` with `CCC_VALIDATE_SSA=1` | **2 passed / 0 failed** (was rc=3) |
| `fx6.c` still at parity | **11 instructions, 0 `blendvps`, 0 `cvtss2sd`** — the win is preserved |
| focused unit tests | **51 passed / 0 failed / 8257 filtered** |
| full SSA-validated regression corpus (local DIAGNOSTIC only) | **1321 passed / 0 failed / 3 skipped-compare / 82 skipped-run / 1406 total** |
| census / strict Clippy / verify-IR / param-home (4 drivers) / fmt / diff | all RC0 |

The full-corpus run above is a **local diagnostic of a CI-caught defect**, not
a substitute for hosted CI. Hosted GitHub CI remains the authority; its green
result on the final push is reported separately.

## What this says about the process

A "godlike quality" claim is only credible if a failure is met with an
IR-diff, an assembly diff, a root cause, two independent guards and a
regression test — not with a retry or a re-baseline. The defect was introduced
by a new consumer interacting with a pre-existing backend limitation; the
correct engineering response is to narrow the consumer's contract to exactly
the domain that is proven, rather than to patch an unrelated backend on a
deadline. That is what was done.

---

# Round 5–7 (new PR): algebraic lattice, purity proof, four-ISA helper, AVX-512 scalar-mask transplants

This section documents the work that treats the merged patch as **untrusted**.
Every change below was motivated by a measured gap, considered against at
least two alternatives, implemented fail-closed, and pinned by a mutation
test that must refuse the near-miss.

## Design decision 1 — one byte-local algebraic decision procedure

**Gap measured.** `/home/user/probes/ident.c` (`pxor`, `pcmpeqb`, `pand`,
`por` on a runtime vector, four `pmovmskb`s): LCCC `-O2 -I include -msse2 -S`
emitted `subq $328,%rsp`, a `call mk`, four `pmovmskb`, plus the four vector
ops. GCC 16.2 and Clang 23.1.5 emitted immediates. All three binaries printed
`0 65535 0 65535`.

**Alternatives considered.**
1. *Peephole each intrinsic separately* — 4 duplicated identity tables, and
   the SSA (`VecAnd*`) and write-through (`Pand128`) forms would drift. Rejected.
2. *A full bit-vector abstract interpreter* — more power, but needs a value
   numbering for every IR opcode and would require a new proof per opcode.
   Rejected as disproportionate: the four identities needed are byte-local.
3. **Chosen:** one `combine_byte_logic(kind, width, a, b, same)` with
   `Option<Vec<u8>>` inputs (`None` = unproven). Both the legacy and the SSA
   forms converge on it. Fail-closed on every unproven byte; lane widths are
   irrelevant because the identities are byte-local, so no signedness proof
   is needed.

**Measured after.** ON `pmovmskb=0 pxor=0 pcmpeqb=0 instructions=23`; OFF
(`CCC_DISABLE_PASSES=maskfacts`) `pmovmskb=4`; ON/OFF runtime both
`0 65535 0 65535`. Two new C fixtures (`runtime_identities`,
`algebraic_refusals`) agree byte-for-byte with GCC 16.2 and Clang 23.1.5.

**Red-team finding (a real soundness hole, fixed).** The first
`operand_is_pure` accepted *any* non-`Copy` definition via `Some(_) => true`.
A `Cast` to a pointer, `GlobalAddr`, a volatile `Load`, an atomic load, a
phi, or an undefined external value was therefore classified as an immutable
SSA vector. On a volatile address `x ^ x` can read two different bit
patterns, so the identity is not provable. **Fix:** `operand_is_pure` now
requires a POSITIVE proof — a plain non-volatile `Alloca` stack home, an
immediate, or a `Copy` chain ending in a `pure_ssa_definition` (an allowlisted
`dest_ptr: None` value producer, a non-pointer `Cast`, a non-volatile scalar
`Load`, or scalar arithmetic). Ten adversarial shapes are pinned by
`repeated_operand_refuses_every_pointer_and_observable_definition`, with a
positive control proving the refusals are selective. Budget exhaustion and
`Copy` cycles are pinned separately.
`ssa_intrinsic_without_a_write_through_home_refuses_object_mentions` pins that
`mention_of` stays `Reject` for an SSA intrinsic with `dest_ptr: None`: with
no proven memory home, `Write` and `NoWrite` would both be guesses.

## Design decision 2 — a target-neutral sign-bit extractor for ALL FOUR ISAs

**Gap measured.** Before this round the four-ISA cross probe showed
`maskfacts` firing only on x86-64/i686 (3 vs 30 / 3 vs 47 instructions);
AArch64 and RISC-V already folded the trivial shape in *earlier* passes
(3 instructions either way, 0 IR loads with the pass disabled). There was no
measured `maskfacts` attribution on ARM/RISC-V, so "perfect AArch/RV" was an
unproven claim. All `__lccc_simd*` helpers were hard-gated to x86 in
`lower_lccc_simd`.

**Alternatives considered.**
1. *Relax the gate for every helper* — unsound: most ops have no ARM/RV
   lowering (the `check_foreign_intrinsics.py` contract exists precisely
   because they used to lower silently to zeros). Rejected.
2. *Implement new NEON / RV movemask codegen* — the backends already have
   `Pmovmskb128` (NEON reduction and RV software reduction). Duplicating it
   would be technical debt. Rejected.
3. **Chosen:** expose exactly ONE mnemonic, `pmovmskb128`, through a
   `portable_simd_op()` allowlist. It is a Scalar, 1-real-operand helper whose
   opcode is implemented on x86-64, i686, AArch64 and RISC-V64. On x86 the
   SSE2 permission still applies; on non-x86 the x86 feature checks are
   skipped (they are meaningless there) while every other helper keeps the
   historical rejection. `portable_allowlist_is_fail_closed_for_unrelated_intrinsics`
   and `pmovmskb128_is_the_only_portable_helper_and_is_exactly_scalar` pin
   that the allowlist is ONE instruction wide. `check_foreign_intrinsics.py`
   still passes: 635 registered x86-only builtins x 2 non-x86 backends.

**Measured four-ISA EXECUTION proof** (`scripts/check_maskfacts_cross_arch.py`,
`--self-test` proves every evidence rule rejects its mutation):

| ISA | fixture | pass | exit | asm instructions |
|---|---|---|---:|---:|
| x86_64 | constant_mask | on / off | 255 / 255 | 3 / 27 |
| i686 | constant_mask | on / off | 255 / 255 | 3 / 29 |
| aarch64 | constant_mask | on / off | 255 / 255 | 3 / 49 |
| riscv64 | constant_mask | on / off | 255 / 255 | 3 / 60 |
| x86_64 | runtime_mask | on / off | 145 / 145 | 66 / 66 |
| i686 | runtime_mask | on / off | 145 / 145 | 94 / 94 |
| aarch64 | runtime_mask | on / off | 145 / 145 | 113 / 113 |
| riscv64 | runtime_mask | on / off | 145 / 145 | 237 / 237 |

Oracles are computed in Python (`0xFF` and `0x91`), not by the compiler.
Each binary is assembled with Clang 23.1.5's integrated assembler, linked
with mold 3.0.0 and a per-ISA `_start`, its ELF machine checked with
`readelf -h`, then EXECUTED (natively on x86-64, under qemu-i386 /
qemu-aarch64 / qemu-riscv64 11.1.3). The `runtime_mask` fixture uses a
volatile-derived image so the AArch64 NEON reduction and the RISC-V software
reduction run for real; ON and OFF are instruction-identical, so no
over-eager fold is possible. The RISC-V magic-multiply reduction was also
brute-forced against all 256 possible 8-byte sign-bit patterns: **0
mismatches**.

## Design decision 3 — a SECOND lattice for scalar `__mmaskN`

**Gap measured.** `tests/regression/maskfacts_cross/avx512_mask_folds.c`
compiled with `-O2 -mavx512f -mavx512bw -mavx512vl` on LCCC with the pass
disabled emitted 83 instructions, 3 `vmovdqu8`, 4 `kmov` and 1 `vpcmpub`,
despite every mask and one operand pair being compile-time constants.
ISPC's `IntrinsicsOpt` folds these; LCCC did not, because its existing
`MaskStatus` lattice describes a 16/32-BYTE vector image, while AVX-512
carries the mask in a GPR as a scalar bitmask.

**Alternatives considered.**
1. *Reuse `MaskStatus::from_lanes` on a synthesised byte image* — loses the
   distinction between a scalar `k` value and a vector, and would need a fake
   object. Rejected.
2. *Peephole in the x86 backend* — target-specific, unreachable from the
   IR-level consumers, and untestable on ARM/RISC-V. Rejected.
3. **Chosen:** `ScalarMask { AllOff, AllOn, Mixed(u64), Unknown }` with
   `Facts::scalar_mask(op, lanes)`. Only an immediate or a bounded `Copy`
   chain ending in an immediate is proven; FP/decimal constants, phis,
   casts, calls, cycles and exhausted budgets refuse. Constants are masked
   to exactly `lanes` bits, so `__mmask16(-1)` is `AllOn` and `0x1234FFFF`
   is also `AllOn` on a 16-lane op. The `Mixed(k)` payload is kept
   deliberately: it is the input the deferred true-prefix analysis needs,
   and the enum is transient (never heap-allocated, never stored in `Facts`).
   `rustc -Zprint-type-sizes`: `ScalarMask` 16 B / align 8, `ImageWindow`
   32 B / 8, `ObjectMention` 32 B / 8, `MaskStatus` 8 B / 4, `ByteOp` 1 B / 1.

**Three consumers, all fail-closed:**

* **Constant mask compares** (`CmpeqEpu*/CmpEpi*/CmpEpu*Mask{128,256}`)
  fold to `Copy { Const(I64(mask)) }`. Only predicates 0 (`eq`) and 4 (`ne`)
  are accepted; the byte images come from the SAME `image_of` proof used by
  `compare_image`, so there is one decision procedure for both encodings.
  A self-compare of a pure operand needs no image. `mention_of` classifies
  these as `NoWrite` (they read two vectors and write only an SSA scalar),
  with BOTH pointer operands pinned to resolved stack objects.
* **All-off zeroing masked loads** (`MaskzLoaduEpi8_{128,256}`) become the
  exact-width `Setzero128`/`Setzero256` write-through. The destination is
  bounds-proven against a plain non-volatile `Alloca`, the SSA `dest` must be
  unused (no dangling SSA), and 512-bit forms refuse because the image proof
  caps at 32 bytes and there is no 64-byte zero producer.
* **All-on 128-bit zeroing masked loads** become `Loaddqu` with the raw
  pointer — exactly the lowering measured for `_mm_loadu_si128`. This removes
  the `kmov` and the EVEX `{k1}{z}` decorator. The 256-bit all-on form is
  deliberately refused: `Loadu256` uses `avx_load_arg`, whose operand may be
  a register-allocated vector VALUE rather than a raw byte pointer, so the
  substitution is NOT provably equivalent here.
* **All-off masked stores** (`MaskStoreuEpi8_{128,256,512}`) are ERASED. A
  masked-off EVEX store performs no memory access at all. Erasure uses a
  separate `fold_removal` decision and the existing span-safe
  `remove_instructions` helper, so `source_spans` stays parallel.

**Measured after** (same source, LCCC `-O2`, AVX-512 flags), host CPU is an
AVX-512BW/VL Xeon:

| configuration | instructions | `vmovdqu8` | `kmov` | `vpcmpub` | stdout |
|---|---:|---:|---:|---:|---|
| LCCC pass ON | 49 | 0 | 0 | 0 | `0 168231937 5555 197` |
| LCCC pass OFF | 83 | 3 | 4 | 1 | `0 168231937 5555 197` |
| GCC 16.2 | 36 | 0 | 1 | 1 | `0 168231937 5555 197` |
| Clang 23.1.5 | 40 | 0 | 0 | 0 | `0 168231937 5555 197` |

`const_compare` is now `movl $21845, %eax; ret` in LCCC (one instruction);
GCC 16.2 still emits `vmovdqa` + `vpcmpub` + `kmovd` for it, Clang 23.1.5
also folds it. `full_load` is `movdqu buf(%rip), %xmm0; movd %xmm0, %eax;
ret`. `zero_load` is `pxor %xmm0,%xmm0; movd %xmm0,%eax; ret`. `zero_store`
contains no store at all. GDB 18.1 single-hit on `const_compare` reports
`eax = 0x5555` and the inferior exits normally with the correct stdout.

**This is a NARROW, measured claim**: on this one AVX-512 kernel LCCC closes
its own 83→49 instruction gap and matches Clang's constant-mask folding;
GCC 16.2 and Clang 23.1.5 still produce fewer *whole-program* instructions
(36 / 40), notably because Clang narrows `full_load` to a single 4-byte
`movl`. No broad "LCCC beats GCC/Clang/ICX" claim is made.

## Host-CPU runtime A/B (measured, this PR)

`maskfact_bench.c`: 100M calls through a **volatile function pointer** (so no
compiler can hoist the loop), alternating 6 runs per configuration. Both
configurations print checksum `1108194560`.

| pass | median (ms / 100M calls) | ns/call |
|---|---:|---:|
| ON | ~147 | ~1.47 |
| OFF | ~865 | ~8.65 |

~5.9x LCCC-with-pass vs LCCC-without-pass. ON kernel is
`movl $43605, %eax; ret`; OFF kernel has 22 instructions including 16 byte
stores, `movdqu` and `pmovmskb`. objdump 2.47 disassembly and GDB 18.1
(`kernel eax = 0xaa55`, inferior exits normally) confirm.

**Fair three-way comparison** (`maskfact_bench3.c`, the SAME portable SSE2
`_mm_movemask_epi8` source, all three compilers emit the identical
`movl $43605, %eax; ret`): median ms over 4 alternating runs —
LCCC 148, GCC 16.2 155, Clang 23.1.5 146 (100M indirect calls, identical
checksum `1108194560`). This is **parity within noise**, not a superiority
claim. Clang's assembly needs Clang's own assembler (`.prefalign`/`.addrsig`
are rejected by GNU as 2.40); that is a toolchain fact, not a defect.

## Best-tools audit (only tools actually run are claimed)

| tool | version | what it proved |
|---|---|---|
| GDB | 18.1 | `kernel eax = 0xaa55`; `const_compare eax = 0x5555`; normal exit |
| objdump / readelf / nm | binutils 2.47 | per-function disassembly, ELF class/machine, symbol tables |
| pyelftools + capstone | 0.33 / 5.0.7 | full `.text` decode: 0 undecodable bytes in all 5 binaries; ON 0 `pmovmskb`, OFF 1 |
| Valgrind | 3.19.0 (extracted from the restored `debs` cache, no root needed) | memcheck on 4 static freestanding LCCC binaries (2 fixtures x 2 pass states): **0 errors, 0 leaks** in every case |
| ASan + UBSan | system GCC 12.2.0 runtime | all 4 new C fixtures: compile 0, run 0, **0 sanitizer diagnostics**; the benchmark's signed-overflow UB was found and fixed this way |
| Clang 23.1.5 integrated assembler | — | assembled x86-64, i686, AArch64, RISC-V64 and AVX-512 EVEX output |
| mold | 3.0.0 | linked four ISAs (`elf_x86_64`, `elf_i386`, `aarch64linux`, `elf64lriscv`) |
| qemu-user | 11.1.3 | executed i686, AArch64 and RISC-V64 binaries |
| rustc `-Zprint-type-sizes` | 1.99.1 + `RUSTC_BOOTSTRAP=1` | exact enum/struct layouts (pahole-equivalent) |
| Python brute force | — | RISC-V magic multiply exact for all 256 byte-mask patterns; scalar mask lattice exact for all 65,536 16-bit values |

**Tools NOT available, explicitly not claimed:** `objtool`, `pahole`, `perf`,
PMU/Callgrind, Miri, and a local ICX 2026.2. Valgrind on *dynamic* binaries
fails at startup in this image (mandatory `ld-linux` `strlen` redirection;
the restored `libc6-dbg` is glibc 2.36 and does not match the host loader),
so the Valgrind claim is limited to the static freestanding binaries above.
The Intel PPA is unreachable (`http=000`) and `intel/llvm` releases carry
only SYCL tarballs, so no local ICX comparison exists; the hosted
authenticated ICX 2026.2 lane is the only ICX reference. Hosted CI on the
previous PR (`9129c09`, run `42370415640`) was GREEN including ICX job
`128398075708`; the final pushed revision of THIS PR is a separate result.

## Follow-up list: item-by-item disposition

| item | disposition | evidence / reason |
|---|---|---|
| masked load/store all-off / all-on | **IMPLEMENTED** (all-off load→zero, all-on 128→`Loaddqu`, all-off store→erased) | 83→49 ins; 8 unit tests; three-way stdout parity |
| AVX-512 `CmpeqEpu*Mask*` / `CmpEpi*Mask*` / `CmpEpu*Mask*` constant masks | **IMPLEMENTED** (128/256, eq/ne) | `movl $21845,%eax`; GDB `0x5555`; exhaustive 65,536-value lattice test |
| constant true-prefix masks | **DEFERRED, justified** | `ScalarMask::Mixed(k)` deliberately retains the bit pattern so this can be added without a lattice rewrite. ISPC's `ReplaceMaskedMemOps::lCheckMask` replaces a true prefix with a narrower load; LCCC has no verified partial-width unmasked load with a raw pointer at 256 bits, and adding one needs a new 3-way encoding proof. Refusing avoids a partial-write miscompile. |
| all-equal gather offsets | **DEFERRED, justified** | ISPC `GatherCoalescePass.cpp` coalesces LLVM gather calls; LCCC's gather family is lowered in the vectorizer with a different provenance model. No measured repro of a missed coalescing in this patchset, so adding a speculative pass would violate the "prove the gap first" rule. |
| linear / negative-stride masked loads | **DEFERRED, justified** | Needs a stride lattice plus alias/fault proof for negative strides; the current pass proves only constant object images. No measured repro. |
| coalescing gap budget | **DEFERRED, justified** | Same reason; `MAX_WORK`/`MAX_DEPTH` already bound every proof in this pass and are pinned by budget-exhaustion tests. |
| widen-then-average | **DEFERRED, justified** | No `VecAvg*` constant-image consumer exists; adding one requires a rounding-mode proof (unsigned average rounds differently per backend). No measured repro. |
| `MaskLoadu*` (merge form) all-off / all-on | **DEFERRED, justified** | The merge form's `args[2]` is a vector VALUE that must be forwarded to `dest_ptr`; `avx_load_arg`'s register-allocated-value semantics make that substitution unproven here. `masked_fold_candidate` deliberately excludes it so the pass pays nothing for it. |
| `MaskzLoaduEpi8_512` / `MaskzLoaduEpi8_256` all-on | **DEFERRED, justified** | 512: image proof caps at 32 B and there is no 64-byte zero producer. 256 all-on: `Loadu256` uses `avx_load_arg` (vector-value), not a raw byte pointer. Both are pinned by refusing unit tests. |
| `MaskzMaddubsEpi16_512`, `MaskzSet1Epi*`, `VecMaskedAdd*` families | **DEFERRED, justified** | These are masked ARITHMETIC, not masked memory. An all-off mask turns them into a zero vector; an all-on mask turns them into the unmasked op — but each needs an exact-width zero producer / unmasked-op mapping on all four backends, which does not exist for the 512-bit forms. A partial rewrite would be a miscompile. Reachable via `__lccc_simd*` (measured in `expr_builtins_lcccsimd.rs:213-221`); no measured win yet. |
| GCC/Clang loop-hoisting parity | **NOT CLAIMED** | The 100M benchmark needs a volatile function pointer precisely because GCC/Clang can hoist the plain loop. |

## Pass / fail table (direct gates, not local CI)

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |
| `cargo check --lib --profile fastbuild --locked` (`-D warnings`) | PASS |
| `cargo test --lib --profile fastbuild --locked` (full unit suite) | **8323 passed / 0 failed / 10 ignored** |
| focused `simd_mask_facts` tests | **74 passed / 0 failed** |
| `cargo test ... portable_subset` | 2 passed / 0 failed |
| `scripts/audit_census.py --check` | PASS (no metric grew) |
| `scripts/check_clippy_policy.py` | PASS (76 lints, 0 unknown) |
| `tests/regression/check_verify_ir_clean.sh` | PASS (79 files, 38 hooks) |
| `tests/regression/check_param_home_contracts.py` (4 drivers) | PASS `[1a]`–`[13]` |
| `scripts/check_foreign_intrinsics.py` | PASS (635 x 2 non-x86 backends) |
| `scripts/gen_lcccsimd.py --check` | PASS (header matches generator) |
| `scripts/check_ci_gate_parity.py` (PyYAML 6.0.3) | PASS (551 commands, 27 contracts) |
| `run_regression.py --filter simd_mask_facts` | **11 passed / 0 failed** |
| `scripts/check_maskfacts_cross_arch.py --self-test` | PASS (5 mutation rules) |
| `scripts/check_maskfacts_cross_arch.py` (4 ISAs x 2 fixtures x 2 pass states) | PASS, oracle-identical |
| three-way AVX-512 differential (LCCC / GCC 16.2 / Clang 23.1.5) | PASS, identical stdout |
| Valgrind memcheck (4 static binaries) | PASS, 0 errors / 0 leaks |
| ASan + UBSan (4 new C fixtures) | PASS, 0 diagnostics |
| GDB 18.1 single-hit proofs (2 kernels) | PASS |
| pyelftools + capstone `.text` decode (5 binaries) | PASS, 0 undecodable bytes |
| hosted GitHub CI on the FINAL pushed revision of THIS PR | reported separately below |

## Per-function efficiency verdict

"Most efficient" here means: no redundant walk, no allocation on the hot
path, fail-closed on every unproven shape, and a measured win on the repro.
It does NOT mean "universally optimal" — see the deferral table.

| function | verdict |
|---|---|
| `combine_byte_logic` | **Yes.** One byte-local decision for 4 ops x 2 widths; O(width), no allocation beyond the output image; exhaustive test over all 256 byte pairs x 4 ops. |
| `byte_logic_image` / `compare_image` | **Yes.** Shared recursion state (one budget, one cycle set), single `operand_image` walk for the self-identity case, no second identical walk. |
| `operand_is_pure` | **Yes, after the round-6 fix.** Positive proof only; bounded `MAX_DEPTH`/budget; every refused shape pinned. |
| `pure_ssa_definition` / `pure_ssa_intrinsic` | **Yes.** Table lookup + one match; no walks. |
| `Facts::scalar_mask` | **Yes.** O(MAX_DEPTH) with an early constant return; the 16-byte enum is transient and register-resident. Keeping `Mixed(u64)` is a deliberate forward-looking design choice, not waste. |
| `mask_compare_shape` / `masked_store_shape` / `masked_fold_candidate` | **Yes.** Compile-time match tables; the candidate gate is the same predicate as the consumers, so unsupported ops cost zero. |
| `fold_removal` | **Yes.** One shape check + one scalar-mask proof; erasure reuses the existing span-safe helper. |
| `run_function` | **Yes.** Two-phase (one immutable `Facts` snapshot), replacements before removals, indices cannot drift; `constant_fold` chaser stays scoped to edited functions and skips `InlineAsm`. |
| `Facts::new` / `build_mention_index` | **Yes.** O(n+k) provenance index, unchanged from the merged patch; no quadratic rescan. |
| `portable_simd_op` | **Yes.** One `matches!`; widening it requires a new four-backend proof, pinned by test. |
| `intrinsic_mention` (mask-compare `NoWrite` arm) | **Yes.** Pins both pointer args, refuses wrong arity and `dest_ptr: Some(_)`. |
| RISC-V `emit_rv_pmovmskb` | **Correct but not minimal.** Brute-forced exact for all 256 patterns; 237 asm instructions for the runtime fixture is dominated by the fixture's 16 multiplies and two 64-bit constant materialisations, not by the reduction. No claim of optimality. |
| `check_maskfacts_cross_arch.py` | **Yes.** 16 compile/assemble/link/execute paths, independent Python oracle, non-vacuity rules (fold must fire; runtime image must NOT fold), `--self-test` mutation proof, ELF-machine verification. |

## Explicitly NOT claimed

* No broad "LCCC beats GCC 16.2 / Clang 23.1.5 / ICX 2026.2" claim. The
  narrow measured facts above are the whole claim.
* No local ICX 2026.2 comparison (PPA unreachable, no standalone asset).
* No `objtool`, `pahole`, `perf`, PMU/Callgrind, Miri result.
* No Valgrind result for *dynamic* binaries (loader debuginfo mismatch).
* No full cross-GCC C conformance suite on AArch64/RISC-V (no cross sysroots
  in the sandbox); the four-ISA proof uses freestanding static binaries,
  Clang's integrated assembler, mold and qemu-user.
* No native ARM/RISC-V `maskfacts` movemask fold on *x86 builtin sources* —
  those remain x86-only by contract. The four-ISA fold is through the new
  target-neutral helper.
* No claim that the deferred follow-up items are implemented; each is
  listed with its exact blocker.
* No claim of GCC/Clang loop-hoisting parity.
* Local CI was NOT run; the hosted GitHub CI on the final pushed revision is
  the authority and is reported separately.
