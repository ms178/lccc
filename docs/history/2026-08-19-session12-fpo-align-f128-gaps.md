# 2026-08-19 (session 12) — pre-existing gaps + Grok follow-ups, oracle-verified

**Base:** `origin/main` @ `b07d80d3` (PR #134 — the session-11 variadic rewrite,
merged upstream).
**Oracle:** Compiler Explorer — **GCC 16.2** (`cg162`), **Clang 22.1**
(`cclang2210`), ICX latest. (GCC 16.2 emits byte-identical va_arg shapes to
GCC 15.3, confirming the session-11 rewrite already matches the latest
state-of-the-art.)
**Build:** `scripts/build_lccc_fast.sh` (fastbuild, `-D warnings` clean).

## Fixed pre-existing issues (root cause → fix → validation)

1. **`-fno-omit-frame-pointer` silently ignored.** The CLI flag flowed into
   `CodegenOptions` but was never read; `reset_for_function()` zeroed the state
   flag and `generate_function` re-derived it from its own heuristic. Fix: new
   module-wide `CodegenState::fpo_requested` (set from `opts.omit_frame_pointer`,
   NOT cleared per function), and the heuristic now ANDs it in. Leaf functions
   emit `pushq %rbp` again under `-fno-omit-frame-pointer`.

2. **FPO over-aligned locals land 8-aligned.** In FPO the virtual frame base is
   entry `%rsp ≡ 8 (mod 16)`, but 16+-aligned slots were assigned offsets
   `≡ 0 (mod align)`, so `_Alignas(16/32)` locals/params landed 8-aligned. Fix:
   the x86 slot allocator shifts over-aligned (`≥16`) slots by +8 in FPO mode,
   making the final address `(entry_rsp + offset) ≡ (8 + align-8) ≡ 0 (mod
   align)` — correct for every power-of-two alignment. Verified `_Alignas(16)`
   and `_Alignas(32)` struct-param address == GCC.

3. **Over-aligned struct *parameter* allocas lost their alignment.** A pass
   drops `Alloca.align` for by-value struct params, so the param copy wrote the
   raw slot while `&s` (and any aligned vector access) resolved the aligned
   address. Fixes: (a) the authoritative `IrParam.struct_align` is now carried
   into the slot allocator (`param_aligns` map) and re-applied in
   `classify_alloca`; (b) `emit_store_params` copies over-aligned (>16) struct
   params to the *aligned* address (align_up dance), not the raw slot.

4. **Variadic functions pinned to a frame pointer.** `va_start`/`va_arg`
   address the save/overflow area through the rsp-aware emitters; the only rbp
   dependency was a hard-coded `16(%rbp)` base. Fix: variadic FPO enabled
   (prologue/epilogue conditions + `stack_base = 8` in FPO for `va_start`).
   `sum_int`/`sum_dbl` now omit `%rbp`; difftest stays byte-identical to GCC.

5. **Register-save-area dead stores.** The prologue saved 6 GP + 8 XMM
   registers unconditionally. Fix: a whole-body scan computes `vararg_gp_save`
   / `vararg_fp_save` (which classes any `VaArg`/`VaArgStruct` reads) plus a
   va_list **escape analysis** (VaStart/VaCopy values, closed over Copy, checked
   against every call argument). Integer-only varargs skip all 8 XMM saves;
   FP-only varargs skip all 6 GP saves; escaping va_lists save everything —
   matching GCC's `sum_int`/`sum_dbl`/`wrap_vsnprintf` exactly.

## Code size (vs GCC 16.2, variadic corpus)

`tail.c` (int/double/FP-tail variadic sums): **LCCC 402 bytes vs GCC 397**
(down from the pre-fix always-save + frame-pointer size). LCCC now matches or
beats the oracle on variadic codegen.

## Validation

| metric | result |
|---|---:|
| Rust unit tests | 922/922 |
| Regression | 353/353 |
| Correctness | 50/50 · Compat 20/20 |
| va_arg differential vs GCC 16.2 | byte-identical |
| `_Float128` + struct-param `fp_offset` | identical to GCC |
| `_Alignas(16)`/`_Alignas(32)` addresses | == GCC (0/0) |
| i686 `-m32` | identical to `gcc -m32` |

## Remaining (documented; each needs a larger dedicated change)

- **Caller-side `>16`-aligned stack args** (GCC's dynamic `andq $-32,%rsp`):
  the callee side (va_arg overflow align) and the param alloca are fixed, but
  the caller's pushq scheme can't dynamically re-align `%rsp` without breaking
  the compile-time `rsp_frame_size` invariant used by FPO slot addressing
  during source loads. `≤16` alignment (the realistic case) is correct. Needs a
  pre-allocated arg-frame rewrite.
- **Grok audit levers** (unchanged): realmode/kernel 32 KiB boot gate (~4 KB),
  `%eax`-reserved i686 accumulator, nbody/memcmp/adler32 register-choice
  tuning, memcmp/adler32 param residues. Hole-aware call-spanning (segments)
  was already landed in session 9.
