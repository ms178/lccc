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

### 6.6 RH-03 implementation plan (next session's starting point)

The pointer-form packed integer/bitwise families are the remaining large
round trip: `Paddb128`/`Paddq256`/... take one ADDRESS per operand and write
through a `dest_ptr`, so each operand home (including the ABI's parameter
home) is a real frame slot and each intermediate result materialises a fresh
one.  The register-valued twins already exist and are exercised by the
vectorizer — every op below was verified present in `src/ir/intrinsics.rs`
by exact-name grep on this tree:

| pointer form (128 / 256) | register twin (128 / 256) |
|---|---|
| `Paddb`, `Paddw`, `Paddd`, `Paddq` | `VecAddI8x16`/`VecAddI8x32`, `VecAddI16x8`/`VecAddI16x16`, `VecAddI32x4`/`VecAddI32x8`, `VecAddI64x2`/`VecAddI64x4` |
| `Psubb`, `Psubw`, `Psubd`, `Psubq` | `VecSubI8x16`/`VecSubI8x32`, `VecSubI16x8`/`VecSubI16x16`, `VecSubI32x4`/`VecSubI32x8`, `VecSubI64x2`/`VecSubI64x4` |
| `Pand`, `Por`, `Pxor` | `VecAndI32x4`/`VecAndI32x8`, `VecOrI32x4`/`VecOrI32x8`, `VecXorI32x4`/`VecXorI32x8` |
| `Pmullw`, `Pmulld` | `VecMulI16x8`/`VecMulI16x16`, `VecMulI32x4`/`VecMulI32x8` |
| `Pmaxub`, `Pminub`, `Pmaxsd`, `Pminsd` | `VecMaxU8x16`/`VecMaxU8x32`, `VecMinU8x16`/`VecMinU8x32`, `VecMaxI32x8`, `VecMinI32x8` |

Shape of the change: a `native_integer_map_128(op) -> Option<(IntrinsicOp,
IrType, usize)>` / `..._256(...)` table next to `native_fma_256`, consulted by
the same `try_native_packed_fp_helper` mechanism (rename it), with the
existing operand-shape/volatile/address-space guards reused verbatim.  Two
traps that already cost time elsewhere in this family:

1. **Key on the opcode AND the declared C type, never the opcode alone.**
   One pointer op names one exact lane structure (`Paddb128` is bytes,
   `Paddd128` is dwords), but the *Builtin* layer maps `_mm_add_epi8` and
   `_mm_add_epi16` to different builtins that can share an opcode path; the
   shape guard (`simd_shape` bytes/elem) is what keeps the operand types
   honest, and it must stay BEFORE any operand is evaluated.
2. **`Pand`/`Por`/`Pxor` are type-agnostic bitwise ops.**  A register twin of
   any lane width emits the identical encoding, so pick the twin that matches
   the declared element type rather than introducing a cast; mixing widths
   here would compile but break the 128-bit double path (G5) further.

Acceptance for RH-03 is the same four receipts as RH-01/RH-02 plus one extra:
the packed-integer gate must run every lane width (i8/i16/i32/i64) × 128/256
against a scalar reference, because a wrong lane width in the twin table is
invisible to an instruction-count budget.
