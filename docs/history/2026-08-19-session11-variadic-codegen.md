# 2026-08-19 (session 11) — x86 variadic codegen audit + oracle rewrite

**Scope:** `src/backend/x86/codegen/variadic.rs` (va_arg / va_start / va_copy) +
its two identified follow-ups (prologue `%al` gating, struct `alignof` threading),
plus the three pre-existing gaps found during validation (`_Float128` lowering,
caller-side over-aligned stack args, struct param alloca alignment).
**Oracle:** Compiler Explorer (godbolt.org) — GCC 15.3, Clang 21.1, ICX latest,
`-O2 -m64` (`/oracle/ORACLE_FINDINGS.md` in the scratch area records the shapes).
**Build:** `scripts/build_lccc_fast.sh` (fastbuild profile, `-D warnings`).

## Bugs found and fixed (validated bit-exact against GCC)

| # | Defect | Fix |
|---|--------|-----|
| 1 | `va_arg(ap, long double)` did `fldt → fstpl → %rax`, silently truncating the 80-bit x87 value to f64 (wrong ABI value, precision loss) | `fldt` from a 16-aligned overflow slot + `emit_f128_load_finish` (fstpt to dest, f64 shadow in %rax). Bit-exact 80-bit result confirmed vs GCC on `2^63 + 1`. |
| 2 | `va_arg(ap, __int128)` read 8 bytes and advanced `gp_offset` by 8 | 2 GP slots (`cmpl $32, ja`), 16-byte-aligned overflow fallback; `rax:rdx` low/high per the i128 accumulator convention. |
| 3 | `va_start` `fp_offset` counted only `FloatReg` named params | New `ParamClass::fp_reg_count()` also counts `F128SseReg`, `F128FpReg`, `StructSseReg` (1–2), and `StructMixed*`. GCC oracle: named `_Float128`/`struct dd`/`struct dl`/`struct {double}` shift fp_offset to 64/80; LCCC now matches (validated: `after_dd`/`after_dl`/`after_d1` == GCC). |
| 4 | MEMORY structs / `__int128` / `long double` never 16-aligned the overflow pointer when `alignof > 8` | Threaded `align` through `VaArgStruct` (IR field) → trait → all four backends; overflow path aligns to `align` when `align > 8`. |
| 5 | Scalar FP bounced through the GPR domain (`movsd; movq %xmm0,%rax`) | FP stays in the SSE domain end-to-end (`store_xmm_to`); matches GCC/ICX. |
| 6 | Hot path used `jb + jmp` (cold fall-through) and reloaded offsets | `ja → mem` with the register path falling through; the compared offset is reused as the index (`movl %eax,%edx`) and `addq 16(%rcx),%rdx` folds reg_save_area into the address (GCC/ICX shape). |
| 7 | `-mno-sse` FP `va_arg` emitted SSE instructions | Under `no_sse`, FP varargs fetch the raw bit pattern in the GPR domain (`movq`/`movl`); zero SSE emitted (what kernel `-mno-sse` files need). (Note: FP *varargs* under `-mno-sse` remain semantically broken because the caller still passes FP in XMM — pre-existing, out of scope.) |
| 8 | `va_copy` = 6× `movq` | `movdqu` 16-byte move + `movq` (4 instr), GCC/ICX shape; `movq` fallback under `no_sse`. |
| 9 | MEMORY struct copy = unrolled `movq` only | 16-byte `movdqu` pairs, then 8-byte `movq`, then a byte tail (GCC/ICX/Clang shape). |

## Oracle-distilled shapes (now emitted 1:1)

- Scalar GP/FP: `movl off(%rcx),%eax; cmpl $40/$160,%eax; ja mem; movl %eax,%edx;
  addl $8/$16,%eax; addq 16(%rcx),%rdx; movl %eax,off(%rcx); movX (%rdx),…`
  (fall-through register path, offset reused, reg_save_area folded).
- Struct register path (ICX shape, no SSE-domain crossing): both classes checked
  (`ja` to a common mem label), FP eightbytes fetched first into `%xmm0`, the GP
  eightbyte LAST into `%rax`, then `addl $8,(%rcx); addl $16,4(%rcx)` memory RMWs.
- `long double`/`__int128` overflow: `addq $15; andq $-16` then fetch/advance 16.

## Follow-up #1: prologue register-save area (done)

- Save only the GP **tail** (`rdi..r9` from `num_named_int_params`; the named
  params' registers can never be read back by va_arg).
- Save only the XMM **tail** (`xmm[num_named_fp_params..7]`), gated on
  `testb %al,%al; je` — the caller sets `%al` = #SSE regs used (LCCC's caller
  already does this in `calls.rs`). Integer-only varargs (printf/printk
  wrappers) skip all eight 16-byte saves at runtime.

## Follow-up #2: struct `alignof` threading (done)

`Instruction::VaArgStruct` gained an `align` field, set in the lowering
(`alignof_type`), remapped by `inline.rs`/`outline_switch.rs`, and threaded
through `emit_va_arg_struct`/`emit_va_arg_struct_ex` on x86 (ARM/i686/RISC-V
signatures updated; their ABI paths unchanged).

## Follow-up #3: PGO branch hints (documented, not landed)

The register/overflow split has a static fall-through layout (register path
first), which is what the branch predictor sees. True per-call-site value
hints need the PGO value-profiling infrastructure (`ideas/medium_pgo_register_hints.txt`);
no IR plumbing exists for va_arg yet.

## Follow-up #4: differential/regression tests (done)

- `/oracle/difftest.c` — va_arg of int/long/double, long double (80-bit),
  __int128/u128, struct {double,long}/{long,double}/{double,double}/{long,long},
  32-byte MEMORY struct, `_Alignas(16)` struct, va_list escaping, va_copy:
  **byte-identical output vs GCC -O2**.
- `tests/regression/va_arg_wide_struct.c` (new) — self-checking coverage of all
  the above + named-SSE-struct `fp_offset` cases; passes lccc-only and
  `--compare-gcc`.

## Validation

| metric | result |
|---|---:|
| Rust library tests | **922 passed** (910 + 12 upstream PR #132/133) |
| Correctness (GCC oracle) | 50/50 |
| Compat suite | 20/20 |
| Regression (lccc-only) | **353 passed** (346 + 7 upstream, incl. `va_arg_wide_struct`) |
| va_arg differential vs GCC | byte-identical (incl. long double 80-bit, `__int128`, structs, `_Alignas(16)`, va_list escape, va_copy) |
| `_Float128` (init/assign/return/global/cast/builtin) | matches GCC (`100.0`, `__builtin_huge_valf128`, `__extenddftf2` inlining) |
| `_Alignas(16)` stack-arg alignment (caller) | 16-aligned, matches GCC |
| i686 `-m32` varargs | identical to `gcc -m32` |
| ARM / RISC-V | compile clean (signature-only change) |
| fastbuild (`-D warnings`) | clean, `scripts/build_lccc_fast.sh` |

## Pre-existing gaps found and fixed (best technically possible)

1. **`_Float128` lowering** (carrier `U128` is ambiguous with `unsigned
   __int128`, so the IR bit-cast machinery produced garbage bit patterns:
   `_Float128 a = 100.0` stored `{100,0}` instead of binary128 100.0, and
   `(double)1.5F128` folded the binary128 bits as an integer):
   - `eval_const_cast` declines folding both directions (falls back to the
     `__extend{s,d}ftf2`/`__trunctf{s,d}f2` soft-float path, exactly like GCC).
   - New `convert_scalar_ctype` helper routes the scalar-init, assignment, and
     return paths through `convert_to/from_f128`; a source that is already the
     128-bit carrier (I128 const payload / U128-typed value) is copied
     bit-exactly (covers `1.5F128` literals and `__builtin_*f128` results).
   - Global `_Float128` initializers use the exact `long_double`-module binary128
     conversions (`f64_to_f128_bytes_lossless`, `i64/u64/i128_to_f128_bytes`).
   - `FunctionBuildState` gained `return_ctype` so `return` routing is CType-aware.
   - The inliner and switch-outliner no longer clear `ret_is_f128_sse` /
     `struct_arg_is_f128_sse` when remapping a `CallInfo` (the inlined
     `__extenddftf2` used to be re-classified as an `[Sse,Sse]` i128-style
     xmm0:xmm1 return and corrupted the binary128 payload).
2. **Caller-side over-aligned stack args**: threaded `struct_arg_aligns` into
   `compute_stack_arg_padding` / `compute_stack_push_bytes` and the
   `emit_call_compute_stack_space` / `emit_call_stack_args` trait methods;
   `>8`-aligned stack struct args are now 16-aligned (`_Alignas(16)` verified
   vs GCC). `>16`-aligned args additionally need dynamic `%rsp` alignment
   (`andq $-32,%rsp`, the GCC 4.6 ABI change) — still a caller-side follow-up.
3. **Struct parameter alloca alignment**: parameter allocas now carry
   `struct_align` (a by-value `_Alignas(16)` struct parameter previously landed
   in an 8-aligned slot).

## Still open (documented, orthogonal to this file)

- **FPO over-aligned locals**: `slot_ref`'s virtual base is `entry_rsp ≡ 8
  (mod 16)`, but 16-aligned slots are assigned offsets ≡ 0 (mod 16), so an
  `_Alignas(16)` local/param in an `-fomit-frame-pointer` function lands
  8-aligned. Pre-existing stack-layout issue (affects all over-aligned locals,
  not just varargs); correct in `-fno-omit-frame-pointer` builds.
- `-fno-omit-frame-pointer` is ignored for leaf functions without dyn-alloca/
  inline-asm-FP/vector intrinsics (`generation.rs` overrides the CLI flag).
- Variadic FPO (GCC omits the frame pointer for variadic functions).
- Dead-save elimination per body va_arg class requires va_list escape analysis;
  the `%al` gate recovers most of the win for integer-only varargs.
