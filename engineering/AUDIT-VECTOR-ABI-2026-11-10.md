# Vector ABI and intrinsic plumbing — measured audit

Date: 2026-11-10.  Base: `fc8350a`.  Toolchains: GCC 16.2.0, Clang 23.1.5,
Rust 1.99.1, binutils 2.47, all from the pinned `harness-cache-8` tag.
Host: Xeon 2.60 GHz, AVX-512F/VL/BW/DQ.

Every number below came from an executed command in this session; nothing is
estimated.  Instruction counts are of the emitted body, between the function
label and `ret`, excluding directives and `#` markers.

## 1. The intrinsic round trip

`__m256 f(__m256 a, __m256 b, __m256 c) { return _mm256_fmadd_ps(a,b,c); }`
at `-O2 -mfma -mavx2`:

| | instructions | frame | aligned slots |
|---|---|---|---|
| LCCC | 28 | 408 B | 6 |
| GCC 16.2 | 2 | 0 | 0 |

The emitted body moves all three arguments to 32-byte-aligned stack slots and
reloads them, then does the same to the result:

    andq $-32, %r11 ; vmovdqu %ymm0, (%r11)   x3   (arguments in)
    andq $-32, %rax ; vmovdqu (%rax), %ymm0   x3   (arguments out)
    vfmadd132ps %ymm1, %ymm2, %ymm0
    andq $-32, %rax ; vmovdqu %ymm0, (%rax)         (result out)
    andq $-32, %rax ; vmovups (%rax), %ymm0         (result in again)

Values were executed at `-O0` and `-O2` and are correct, so this is overhead,
not a miscompile.

## 2. Function-to-function vector return

    typedef float v8 __attribute__((vector_size(32)));
    extern v8 ext8(v8);
    v8 f(v8 a){ return ext8(a); }

| width | flags | LCCC | GCC 16.2 |
|---|---|---|---|
| 128 | `-msse2` | 25 | 2 |
| 256 | `-mavx2` | 25 | 1 |
| 512 | `-mavx512f` | 35 (27 after the peephole) | — |

## 3. Root cause, established by tracing

`src/ir/lowering/lower.rs`, in the function-definition lowering:

    } else if full_ret_ctype.is_vector()
        && abi.sret_size.is_none()
        && abi.two_reg_size.is_none()
    {
        // A small vector travels as packed I64 data, not a pointer.
        ret_ty = IrType::I64;
    }

`IrType` (`src/common/types.rs`) has no vector variant — `I8..U128, F32, F64,
D32, D64, F128, Ptr, Void`.  A ≤16-byte vector return therefore loses the SSE
class that `aggregate_return` just computed, and `lower_function_call`
decomposes the `%xmm0` result into two `I64` halves through GP registers.

The `> 16`-byte case keeps `[Sse, SseUp…]`, so `hidden_vector_return_bytes`
(`src/backend/call_abi.rs`) returns a size, the trait `emit_call`
(`src/backend/traits.rs`) calls `emit_call_store_vector_return`, and the x86-64
implementation (`src/backend/x86/codegen/calls.rs`) does the right thing — one
`vmovdqu %ymm0,(%rax)`.  The i686 backend's caller-side call is identical in
shape (`src/backend/i686/codegen/emit.rs`), which is why the two backends agree
there.  What remains in the x86-64 path is the I64 struct-alloca
materialization the driver wraps around the result.

## 4. What is NOT wrong

* `_mm256_fmsub_ps` and `_mm256_fnmadd_ps` are scalar `for` loops in
  `include/fmaintrin.h`, but the SLP vectorizer contracts them to one
  `vfmsub132ps` / `vfnmadd132ps`, matching GCC exactly (3 FMA instructions for
  the three functions, 0 `vmulps`, 0 `vsubps`, same as GCC).
* `_mm_fmadd_ps` at 128 bits through the `lcccsimd.h` macro: the same class of
  round trip, one width down.
* The i686 128/256-bit FMA placement work from PR #1167 is on `main` and the
  gate is green.

## 5. Gate shipped with this audit

`tests/regression/vector_abi/check_vector_return_abi.sh` asserts two things
separately, because they fail independently:

1. every lane survives a return at 128 and 256 bits, executed at `-msse2` and
   `-mavx2` — **passes today**, so the cost claim is not hiding a wrong answer;
2. `v8 f(v8)` stays within 6 instructions — **fails today at 12**, and starts
   passing when the register home is kept, so it cannot be satisfied by
   accident.

The budget is not aspirational: GCC 16.2 emits 1 instruction (`jmp ext8`).

## 6. Register-homing campaign — session 2026-11-12

Base: `3964191` (= `origin/main` at session start), branch
`arena/5ed3efcb-lccc`, fastbuild `-O1 -j2`. Reference: GCC 16.2
(`gcc-16 -O2 -mavx2 -mfma -S`). Every count below is the number of emitted
instructions between the function label and `.size`, so `ret` is included in
all three columns and the comparison is method-identical.

Two round trips were removed in this session. Both are register-homing
defects of the same family as §1 and §3: a value that is pure data was routed
through a 16/32-byte stack home, and the fix is an IR-representation change,
not a peephole.

### 6.1 F1 — the 256-bit FMA3 map family was memory-homed (fixed)

`_mm256_fmadd_ps/pd` reached codegen through the `m3` lowering table entry
(`FmaPs132v256`/`FmaPd132v256`), which takes an ADDRESS per operand and writes
its result through a `dest_ptr`.  Each register-resident operand was therefore
spilled into a 32-byte dynamically-realigned frame home, reloaded, and the
result was written to a fourth home and read back:

| shape | before | after | GCC 16.2 |
|---|---:|---:|---:|
| `_mm256_fmadd_ps(a,b,c)` | 28 | **2** | 2 |
| `_mm256_fmadd_pd(a,b,c)` | 28 | **2** | 2 |
| `_mm256_fmadd_ps(_mm256_add_ps(x,y), b, x)` | 28 | **5** | 3 |
| three chained `_mm256_fmadd_ps` | — | **6** | 5 |

Fix: `native_fma_256()` in `src/ir/lowering/expr_builtins_lcccsimd.rs` maps a
`v{fnmadd,fnmsub,fmsub,fmadd}{132,213,231}{ps,pd}256` mnemonic to the
register-form op the backend already implements
(`VecMaddF32x8Signed`/`VecMaddF64x4Signed`, parameterised by the two sign
bits), and `try_native_packed_fp_helper` routes it there.  Keying on the
MNEMONIC rather than the opcode is mandatory: one opcode names four sign
structures, and the digit never reaches code selection because
`fma_form_reencode` re-encodes the algebra into whichever 132/213/231 spelling
fits the allocated homes.  All six `ps`/`pd` × 132/213/231 spellings are
bit-exact against `fmaf`/`fma` from libm (that is the value half of the gate,
because FMA3 is a destructive encoding and an operand aliasing the destination
is only observable at run time).

### 6.2 F2 — a direct-load scratch home was read back instead of forwarding (fixed)

`_mm256_loadu_ps(p)` lowers to a POINTER-result intrinsic (`LoaduPs256` writes
32 bytes to a scratch home) plus a register read of that home
(`VecLoadF32x8 [home, 0]`).  `fuse_vector_loads` already forwarded direct
loads into pointer-form intrinsic *arguments*, but a `VecLoad*` base is an
ADDRESS, deliberately excluded by `pointer_vector_arg_width`, so the scratch
home survived as a real frame slot:

```asm
    subq $72, %rsp
    vmovaps %ymm0, %ymm15
    vmovdqu (%rdi), %ymm0        ; load p
    vmovdqu %ymm0, 40(%rsp)      ; write the scratch home
    vfmadd132ps 40(%rsp), %ymm1, %ymm15
    vmovdqa %ymm15, %ymm0
    addq $72, %rsp
    ret
```

Fix: `fuse_in_function` (`src/passes/vector_temp_promotion.rs`) now also
re-points a whole-register read `VecLoad* [home, 0]` at the direct load's
source when the read's width equals the width the load wrote, and drops the
now-unread alloca.  The forward inherits the existing audited alias/volatility
barrier state (`available`, `invalidate_for_value_write`,
`invalidate_for_memory_effects`), and every `VecLoad*` encoding is the
unaligned form (`vmovups`/`vmovupd`/`vmovdqu`), so reading the original —
possibly under-aligned — address is exactly as safe as reading the home.

| shape | pass disabled | now | GCC 16.2 |
|---|---:|---:|---:|
| `return _mm256_loadu_ps(p)` | 10 | **2** | 2 |
| `_mm256_fmadd_ps(_mm256_loadu_ps(p), b, c)` | 12 | **2** | 2 |
| `_mm256_fmadd_ps(a, _mm256_loadu_ps(p), c)` | 12 | **2** | 2 |
| `_mm256_fmadd_ps(a, b, _mm256_loadu_ps(p))` | 13 | **3** | 2 |
| `_mm256_add_epi64(_mm256_loadu_si256(p), b)` | 16 | **8** | 2 |

"pass disabled" is the same tree with `CCC_DISABLE_PASSES=vecpromote`, i.e.
the reproducible counterfactual (and what `--expect-defect`/a stand-in
compiler produces).  The last row is the point of §6.3: the load half is gone,
the integer-op half is not.

The first and second rows are now single instructions
(`vfmadd132ps (%rdi), %ymm1, %ymm0; ret`).

### 6.3 What is still open (measured on this tree, in ROI order)

| ID | shape | now | GCC 16.2 | cause |
|---|---|---:|---:|---|
| G1 | `_mm256_add_epi64(_mm256_loadu_si256(p), b)` | 8 | 2 | packed integer/bitwise map ops are still pointer-form (`Paddq256` takes two home addresses and a `dest_ptr`) |
| G1 | `_mm_add_epi8(_mm_loadu_si128(p), b)` | 6 | 2 | same, 128-bit; the incoming `b` home is the prologue store (`movdqu %xmm0,32(%rsp)`) |
| G2 | `_mm256_fmadd_ps(a, *(const __m256 *)(p+8), c)` | 5 | 2 | FMA emitter rotates operands through two extra register copies (`vmovaps %ymm0,%ymm15`, `vmovdqa %ymm1,%ymm0`) |
| G3 | same shape | (in G2) | — | `p+8` is not folded into the memory operand: `addq $32,%rdi` + `(%rdi)` instead of `32(%rdi)` |
| G4 | `_mm256_fmadd_ps(a, b, _mm256_loadu_ps(p))` | 3 | 2 | an FMA3 memory operand is a multiplicand; folding an addend needs the 213 form (`vfmadd213ps (%rdi), %ymm1, %ymm0`) |
| G5 | `_mm_fmadd_pd(_mm_loadu_pd(p), b, c)` | 11 | 2 | 128-bit double wrapper still crosses the I128 carrier (documented in `try_native_packed_fp_helper`) |
| G6 | any `__m256`/`__m128` parameter | +1 | 0 | the ABI prologue always stores an incoming vector parameter into its home |
| G7 | `_mm256_fmsub_ps`/`_mm256_fnmadd_ps` in a loop body | 13 | 3 | header loop bodies; the SLP contract only fires in the simple shape (§4) |
| G8 | `float h(__m256 a){ return a[3]; }` | 32-byte home round trip | 2 | lane extract goes through the home |
| G9 | `v8 f(v8)` | 12 | 6 | vector return home (§5 budget gate) |

G1 is the largest single win available in this family and the next session's
first item: the register-valued `VecAdd*`/`VecSub*`/`VecAnd*`/`VecOr*`/
`VecXor*`/`VecMul*` ops exist and are exercised by the vectorizer, so the
change is the same mnemonic-keyed routing F1 used, applied to the packed
integer map families at 128 and 256 bits (one mnemonic names several lane
widths, so the table must be keyed on mnemonic + declared operand type, never
on the opcode alone).

### 6.4 Gates

* `tests/simd/vector_fma_homing.c` + `tests/regression/check_vector_fma_homing.sh`
  (new in this session, lane `vector-fma-homing` in `scripts/ci_local.sh`):
  values bit-exact against libm, a detector self-test (the volatile-operand
  shape must still show `andq $-32`), and per-shape instruction budgets for
  the register FMA shapes AND the memory-operand shapes added in §6.2
  (`fma_load_a/b/c/pd`, `load_only`).  Non-vacuity is measured, not asserted:
  with `CCC_DISABLE_PASSES=vecpromote` the same gate reports `load_only` at 9
  instructions with `subq $120` + `andq $-32` and fails.
* Unit tests for the new forward in `src/passes/vector_temp_promotion.rs`:
  the accepted shape, the three rejected shapes (narrower read, non-zero
  offset, double reader) and the aliasing-write refusal.

### 6.5 Environment (so the next session does not re-derive it)

The sandbox can run the complete local mirror, but only after
`engineering/verification/prepare_local_env.sh`.  Two provisioning gaps were
found and fixed there in this session because they turned real gates red:

* the `-m32` gates (`gcc17-builtin-semantics`, `type-attr-static-init`,
  `trapv-boundary-matrix-m32`) failed on missing `bits/libc-header-start.h`
  and could not execute ELF32 at all; the staged sysroot fixes the include
  path but the runner shape needs the script's symlinks (crt/libc/libgcc,
  `/lib/ld-linux.so.2`, `gnu/stubs-32.h`);
* valgrind's driver started but could not find memcheck, because the tool
  binaries live in the ABSOLUTE `/usr/libexec/valgrind`; without that link
  `check_hex_float_oracles.py` fails the hexadecimal-literal oracle instead of
  reporting a Memcheck result.  `check_fma_gating.sh` now passes with
  Memcheck clean (548 literal bit patterns, LCCC/GCC/Clang agreeing).

PyYAML is needed by `check_ci_workflow_shell.py`; `python3 -m pip install
--break-system-packages pyyaml` is the sandbox-local fix.

### 6.6 RH-03 — landed: the packed integer/bitwise map families are register-valued

`try_native_packed_map` (in `src/ir/lowering/expr_builtins_lcccsimd.rs`) routes
65 pointer-form builtin ops through the register-valued twin the SLP
vectorizer already emits.  Measured on this tree at `-O2 -mavx2 -mfma`
(instructions between the function label and `ret`; GCC 16.2 with the same
flags and the same counting rule):

| shape (`a`, `b` register-resident) | pointer form | now | GCC 16.2 |
|---|---:|---:|---:|
| `_mm_add_epi8` / `_mm256_add_epi8` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_add_epi{16,32,64}`, `_mm256_add_epi*` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_sub_epi*`, `_mm256_sub_epi*` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_and_si128`, `_mm256_and_si256` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_or_*`, `_mm_xor_*` (128/256, int and `_ps`) | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_mullo_epi16`, `_mm256_mullo_epi32` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_min_epu8`, `_mm256_max_epi32`, `_mm_min_epi16` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_slli_epi32(a,3)`, `_mm256_srai_epi16(a,5)` | 5 / 8 | **2 / 2** | 2 / 2 |
| `_mm_cmpgt_epi8`, `_mm256_cmpgt_epi64` | 5 / 9 | **2 / 2** | 2 / 2 |
| `_mm_add_epi8(_mm_loadu_si128(p), b)` | 6 | **3** | 2 |
| `_mm256_add_epi64(_mm256_loadu_si256(p), b)` | 8 | **3** | 2 |

The two memory shapes stop one instruction above GCC because the OTHER
operand is a by-value vector parameter and this ABI homes it in a frame slot;
GCC keeps it in `%xmm0`/`%ymm0`.  That is RH-04, not this family: the VEX
memory fold requires the non-memory source in a register, and the emitter
correcty materialises the load when the other source has no register home
(`try_emit_avx_256_memfold`'s "no register source" arm).

#### The two traps, as hit

1. **The key is the op, the guard is the register image.**  `__m128i` is
   `long long __vector(2)` in `include/lcccsimd.h`, so `_mm_add_epi8`'s
   arguments are i64x2 at the C level while the operation is per byte.  A
   table keyed on (opcode, declared lane type) rejects every real call; a
   table that INFERS the lane type from the declared type computes a
   different function (byte add ≠ qword add: carries propagate).  The
   implementation therefore keys on the pointer op (`Paddb128` → bytes) and
   only requires each argument to be a dense 16/32-byte register image of the
   right SIZE — which is exactly what a whole-register `VecLoad`/`VecStore`
   copies.  `tests/simd/x86_int_map_registers.c` fails immediately if a row is
   changed to a wider lane (verified by hand: `Paddb128 → VecAddI16x8` reports
   four disagreeing lanes).
2. **Bitwise twins are type-agnostic; arithmetic twins are not.**  `pand`,
   `por`, `pxor` and the `_ps` bitwise forms can use any lane width (the
   encoding is identical), so they are routed through the dword twins and
   never through an arithmetic one.  Everything else names the twin whose lane
   type is the operation's own.

#### Two shapes deliberately NOT routed

* **Shift counts outside the instruction's defined range.**  `psllw` defines
  1..=15, `pslld` 1..=31, `psllq` 1..=63, and the twin's emitter `assert!`s on
  anything else, while `_mm_slli_epi32(a, 0)` is a legal identity and
  `_mm_slli_epi16(a, 17)` a legal zeroing shift.  Counts 0 and >width keep the
  pointer path; the fixture executes all three cases against a reference that
  implements Intel's over-wide semantics.
* **i686.**  The 32-bit backend implements the value form of only a subset of
  the twins (`emit_intrinsic` panics with "only the dest_ptr form is supported
  here" for `VecMinU8x16` and friends), so the routing is x86-64 only and
  `-m32` keeps the pointer path.  The gate runs the FULL value matrix for
  i686 as well, which is what keeps that path honest.  Giving i686 the value
  form of every twin is RH-03b.

#### Non-vacuity

`CCC_NO_NATIVE_PACKED_MAP=1` reverts the routing at the decision point (same
spirit as `CCC_NO_VLFOLD`).  The gate uses it as a counterfactual on every
run: the pre-fix lowering must cost 5/9 instructions again AND must still
compute the same values.  A gate that could only ever pass would not be a
gate, and a quality switch that changed a value would be a bug in the switch.

#### New defect class found while measuring (RH-11, next)

The pointer ops that have NO twin are worse than "5 instead of 2" — several are
implemented as scalar lane loops in `include/smmintrin.h`, because no builtin
exists for them at 128 bits:

| intrinsic | today | GCC 16.2 | why |
|---|---:|---:|---|
| `_mm_min_epu16` | **103** | 2 | `pminuw` builtin absent; header loops over 8 lanes |
| `_mm_max_epi32` | **48** | 2 | `pmaxsd` absent at 128 bits |
| `_mm_min_epu32` | **37** | 2 | `pminud` absent at 128 bits (`VecMin/MaxU32x4` twins DO exist) |
| `_mm_max_epi8` | 3 | 2 | `pmaxsb` absent; header does compare+blend |
| `_mm_andnot_si128`, `_mm_avg_epu8`, `_mm_mulhi_epu16`, `_mm_adds_epi8`, `_mm_packs_epi16`, `_mm_unpacklo_epi8` | 5 | 2 | pointer form; no twin exists yet |
| `_mm_shuffle_epi32` | 2 | 2 | already at parity |

Fixing them means adding real encodings, not routing: `P{min,max}{uw,ud,sb,sd}{128,256}`
(14 pointer ops), the six 128-bit twins that do not exist yet
(`VecMinU8x16`-style: `VecMinI8x16`, `VecMaxI8x16`, `VecMinI32x4`,
`VecMaxI32x4`, `VecMinU16x8`, `VecMaxU16x8` — the 256-bit twins all exist),
emitter arms (`pminuw`/`pmaxuw`/`pminud`/`pmaxud`/`pminsb`/`pmaxsb`/
`pminsd`/`pmaxsd`, SSE4.1 at 128, AVX2 at 256), the `__lccc_simd{128,256}_i_*`
declarations in `include/lcccsimd.h`, and rewriting the ten header functions.
The header-only workaround (`min(a,b) = a - subs_epu(a,b)`) measured 8 and 11
instructions when this row was written, which is why it was rejected.  That
number was a *different* defect stacked on top of it (see §6.7): the
saturating-difference identity is exact, is what GCC itself falls back to
without AVX-512, and costs 3 instructions once the parameter bridge is fixed.
Comparing against `blendv` is still the wrong answer for the i8/i32 signed and
u32 families — those need the real `pminsb`/`pmaxsd`/`pminud` encodings
(§6.7 "still open").

### 6.7 The other half of RH-11 — landed: inlined vector parameters stop going through memory

§6.6's numbers hid a second defect, and it is the reason the "5 instead of 2"
cases would not move no matter how many twins were added.  Every SSE4.1+
header intrinsic is a `target(...)`-attributed `__always_inline__` function
whose parameters are 16/32-byte vectors *by value*, and LCCC's ABI models such
a parameter as an `Sse,SseUp` aggregate passed as a POINTER
(`struct_arg_sizes: Some(16)`, `arg_types: [Ptr, Ptr]`).  Inlining such a
callee therefore bridged each parameter with a `memcpy` into a fresh home in
the cloned body:

```text
  Alloca 9 (16 B) ;  Memcpy 9 <- %a_home ;  VecLoadI16x8 9, 0     ; one per parameter
```

`VecLoad` from a *parameter home* is what the backend already forwards into
the incoming `%xmm`/`%ymm` (§6.2), but the copy destination is not a parameter
home, so the bridge materialised as a real 16-byte store/load pair, plus a
120-byte frame, plus a register shuffle per operand.  Measured before/after on
`-O2 -mavx2` (instructions between the label and `ret`, GCC 16.2 same rule):

| shape | before | now | GCC 16.2 |
|---|---:|---:|---:|
| `f(__m128i a, __m128i b)` → inlined `_mm_add_epi16(a,b)` | 5 (+120 B frame) | **2** | 2 |
| the same with a `target("sse4.1")` attribute | 5 | **2** | 2 |
| `_mm_andnot_si128`, `_mm_avg_epu8`, `_mm_mulhi_epu16`, `_mm_adds_epi8`, `_mm_packs_epi16`, `_mm_unpacklo_epi8` | 5 | **5 → 2**\* | 2 |
| `_mm_min_epu16` / `_mm_max_epu16` | 103 / 5 | **3 / 3** | 2 / 2 |
| `_mm256_min_epu16` / `_mm256_max_epu16` | did not compile | **3 / 3** | 2 / 2 |

\* the six "5 → 2" rows are the *wrapper* path, which is now at parity: the
5-instruction cost that remains for `_mm_andnot_si128` and friends is the
pointer-form builtin itself (they have no register twin yet), measured
identically to §6.6's pre-fix numbers.  They are RH-11 proper.

**The fix — `forward_memcpy_filled_reads`** in
`src/passes/aggregate_copy_forward.rs`.  When a local alloca is filled exactly
once by a `memcpy` whose source is another local alloca of the entry block, and
every use of the destination is a constant-offset read in the copy's own block
(`Load`, `Load` through a constant GEP of it, or a register-dest `VecLoad*` at
index 0), the reads are re-pointed at the source and the copy is deleted.  The
result is a load off the caller's parameter home, which §6.2's forwarding turns
into the incoming register.

Everything else fails closed, and the list is short enough to audit: a
parameter home as the DESTINATION (its prologue write is invisible — the reason
`forward_store_only_temporaries` refuses those roots), a multi-defined value, a
read in another block, a read whose constant offset plus access width leaves
the copied range, any `Store`/`Memcpy`/opaque memory write between the copy and
its last reader (`is_opaque_memory_write`, whose `Intrinsic` arm already covers
`!is_pure()`, `dest_ptr.is_some()` and `writes_memory_via_args()`), any escape
of the destination, and any memory-dest intrinsic that addresses it.
`CCC_NO_AGG_MEMCPY_FWD=1` disables the sub-pass, the same bisection discipline
the store-forwarding half has carried since its own miscompiles.

#### The transform's own first defect (found by the ABI grid, fixed here)

On i386 a 32-byte by-value vector does NOT arrive as one home: it arrives as
two 16-byte homes, and the callee assembles its object with two `memcpy`s, the
second writing `dest + 16`.  The first version of the transform accepted
`memcpy(half_home, piece, 16)` as a full snapshot, re-pointed a
`GetElementPtr(dest, +16)` at the 16-byte source, and `hash2`/`hashva` in
`tests/regression/ext_vector_abi/abi_grid.c` read past it (`-m32 -O2`: 30 legs
red, 954 examined, all in the by-value provider).

The fix is a rule, not a patch: **the copy must fill the destination object**
(`dest_alloca.size == memcpy.size`, and `src_alloca.size >= memcpy.size`), and
a `Memcpy { src }` that names the destination is a READ like any other -- it
was the one use the first version did not classify, because it looked only at
`Memcpy { dest }`.  Both are in the candidate filter and the use allowlist;
`GetElementPtr` re-pointing is additionally bounded to offsets the copy covers.
`check_ext_vector_abi_grid.sh` is the regression gate: it is the only check in
the tree that passes 16/32-byte vectors by value across the whole Clang 23.1.5
grid on `-m32` and `-m64`, and it is what caught this.

A second shape of the same lesson: the forwarded read must be a
**whole-object** read.  Re-pointing a NARROW read of a parameter home -- `F32`
out of a 16-byte vector home, which is exactly what the i686
`_mm_cvtss_si64` emulation does (`float __x = __a[0]`) -- produced a
miscompile that `tests/regression/sse_cvt_si64_rounding_i686.c` caught
(`-m32 -msse2`: every `cvtss_si64` returned the integer indefinite).  The
backend's direct-load home forwarding has never seen a narrow read of a vector
home, and it is not this transform's job to invent that shape.  Forwarding now
requires `read_width == memcpy.size` at offset 0 for `Load`,
register-dest `VecLoad*` and `Memcpy { src }` alike.

#### Two more defects found on the way

1. **The AVX2 unsigned-16 extrema did not exist.**  `_mm256_min_epu16`,
   `_mm256_max_epu16` (and `_mm256_min/max_epu8/32/64` siblings beyond `epu8`)
   were absent from `include/avx2intrin.h`, so `-mavx2` code that GCC compiles
   failed here with "implicit declaration".  `min/max_epu16` are now defined in
   both widths as the exact saturating-difference identity
   (`subs_epu16` plus one add/sub); the 128-bit pair used to be 8-lane scalar
   loops (`include/smmintrin.h`).  `_mm256_min/max_epu32` and
   `_mm256_min/max_epi8/epi32` are still missing and still need the real
   encodings — same list as RH-11, now recorded with both widths.
2. **`Psubusw{128,256}` had no twin route**, so `_mm_subs_epu16` (and the
   identity above) went through the pointer form and spilled an operand.  One
   row in §6.6's table (`VecSubSatU16x8`/`x16`, which already existed and is
   already emitted as `psubusw`) removes the spill.

#### Still open (RH-11 remainder, unchanged in kind, smaller in size)

* 128-bit signed/unsigned 8- and 32-bit extrema and 32-bit unsigned min/max
  have no builtin at all (`__builtin_ia32_pminud128`, `pmaxsd128`, `pmaxsb128`
  do not exist; only the 256-bit `pmaxsd256`/`pminsd256` do), so
  `_mm_min_epu32` (37), `_mm_max_epi32` (48) and `_mm_max_epi8` (3) need new
  `IntrinsicOp`s + emitters, not routing.
* `_mm_andnot_si128`, `_mm_avg_epu8`, `_mm_mulhi_epu16`, `_mm_adds_epi8`,
  `_mm_packs_epi16`, `_mm_unpacklo_epi8` cost 5 because their pointer builtins
  have no twin; each needs one `Vec*` twin and one emitter arm.
* The memory forms stay one instruction above GCC until RH-04 (the same
  by-value-parameter home, one layer lower, in the call ABI itself).

#### Gate

`tests/regression/check_x86_int_map_registers.sh` gained four
inlined-wrapper shapes (`map_wrap_add16_128`, `map_wrap_maxu16_128`,
`map_wrap_minu16_256`, `map_wrap_and256_256`) with budgets 2/3/3/2 and the
frame-and-stack-slot checks, plus two counterfactuals — the §6.6
`CCC_NO_NATIVE_PACKED_MAP` one and a new `CCC_NO_AGG_MEMCPY_FWD` one that must
put the copies back (non-vacuity: with the forwarding off the same four shapes
cost 6, 8, 29 and 27).  The fixture's execution matrix grew to 84 value shapes
including `_mm_min/max_epu16` at both widths and the wrappers themselves, all
against the scalar reference at `-O0`, `-O1`, `-O2`, `-O3` and on i686.

