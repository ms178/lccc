# Session 38 — cross-target bit-test canonicalization

**Date:** 2026-08-21 UTC  
**Base:** `ms178/lccc` `75d16f90469ad1f92d07a9444c9ecc55a9992eb3`  
**Focus:** replace the prior x86-only text-peephole bit-test solution with a broad IR-level operation used by every backend.

## Problem

Session 37 solved `(x >> i) & 1` with an x86-64 text peephole (`fold_variable_bit_test`) that recognized one already-assembled sequence and rewrote it to `btq`. That was intentionally narrow: it did not help AArch64, RISC-V or i686, and it was hidden behind backend assembly text. The requested complete solution is to make bit testing a first-class operation, canonicalize to it in the IR optimizer, and lower it in every backend.

## Implementation

### IR

`src/ir/ops.rs` adds `IrBinOp::BitTest`:

```text
BitTest(base, index) == ((base >> index) & 1)
```

It is always an integer operation. Constant evaluation and loop-unroll classification treat it like a shift/AND classifier:

- `src/passes/constant_fold.rs` evaluates it at compile time.
- `src/passes/loop_unroll.rs` recognizes it as bit-iteration work.

### Canonicalization

`src/passes/simplify.rs` now recognizes both:

```text
(base >> index) & 1
1 & (base >> index)
```

and rewrites them to `BitTest(base, index)`.

The recognizer also:

- follows same-width integer casts, so `(int)(u32)(signed >> index) & 1` still canonicalizes (this covers the Expat signed/unsigned promotion shape);
- tracks all binary definitions, not only constant-operand ones, because bit tests have variable indexes;
- treats arithmetic and logical shifts identically when isolating bit zero;
- folds `BitTest(_, 0)` to zero;
- leaves the result type at the shift's natural width. This lets 32-bit classifiers use 32-bit BT/UBFX/shift operations and avoids forcing 64-bit shifts on 32-bit values.

### Backend lowering

- **x86-64** (`src/backend/x86/codegen/alu.rs`): register-dest results use `btl/btq` with SETcc directly into the destination byte register, then zero-extend with `movzbl`. Stack/no-home results use the accumulator fallback. Constant indexes use `bt $imm` directly. MachInst declines BitTest because the text backend already provides the native lowering.
- **i686** (`src/backend/i686/codegen/alu.rs`): uses `btl %ecx,%eax` or `btl $imm,%eax`, then SETcc/MOVZBL.
- **AArch64** (`src/backend/arm/codegen/alu.rs`): constant indexes use `ubfx`; variable indexes use `lsr`/`asr` plus `and #1`.
- **RISC-V** (`src/backend/riscv/codegen/alu.rs`): uses `srlw/srl` plus `andi`, preserving the existing 32-bit zero-extension convention.

The old session-37 x86 text peephole was removed from:

- `src/backend/x86/codegen/peephole/passes/local_patterns.rs`
- `src/backend/x86/codegen/peephole/passes/mod.rs`

The canonical IR path now produces the same BT before peepholes run, so the text-only pattern is redundant.

## Examples

```asm
; x86-64: unsigned var(unsigned x, unsigned i)
bittest:
    movl %esi, %edx
    movl %edi, %r8d
    btl %rdx, %r8
    setc %r8b
    movzbl %r8b, %r8d
    movl %r8d, %eax
    ret
```

```asm
; AArch64: constbit(unsigned x)
constbit:
    ...
    ubfx w0, w1, #3, #1
    ...
```

```asm
; RISC-V: bittest
srlw t0, t1, s8
andi t0, t0, 1
slli t0, t0, 32
srli t0, t0, 32
```

## Validation

- `cargo test --profile fastbuild -j2`: **990 passed** after removing the obsolete text-peephole unit test.
- Regression suite at `-O3`: **383 passed, 0 failed**.
- Regression suite at `-O2 --compare-gcc`: **383 passed, 0 failed**.
- Correctness suite: **50 passed, 0 failed**.
- New regression: `tests/regression/check_bit_test_canonical.sh` verifies x86 BT, AArch64 UBFX and RISC-V shift/AND, plus an x86 runtime harness over variable/constant indexes.
- Targeted build/run checks:
  - `adler_inline_tail.c`
  - `expat_xml_scan.c` (output remains `626766774715194881`)

## Remaining work

The cross-target BitTest primitive is in place. The larger Expat classifier is still a branchy range-membership tree. Future work can map DNF range sets to one or more BitTest operations (or AArch64 TST/UBFX combinations) now that every backend has a canonical destination for the final bit test.
