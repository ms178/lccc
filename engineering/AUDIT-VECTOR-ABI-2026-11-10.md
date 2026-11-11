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
