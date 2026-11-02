# Pointer-IV collapse and zero-guard count promotion

Two instruction-selection changes landed together because they share a cause:
the IR knew something the emitted code never asked about.

## `ptr_iv_collapse` — several pointer induction variables, one index

A scan over several arrays keeps one pointer induction variable per array plus
a counter for the exit test. Every one of them is an independent induction
variable, so each iteration pays a 64-bit increment per array on top of SIB
addresses that would have performed the same arithmetic in the operand.

zlib-ng's `compare256_64` (`arch/generic/compare256_p.h`) is the shape:

```
; before: three induction variables, three adds
    movq (%rdi), %r10
    xorq (%rsi), %r10
    jne  .LBB5
    addq $8, %rdi
    addq $8, %rsi
    addl $8, %edx
    cmpl $256, %edx
    jb   .LBB1

; after: one counter, addresses folded into the operands
    movq (%rdi,%rdx), %r10
    xorq (%rsi,%rdx), %r10
    jne  .LBB5
    addl $8, %edx
    cmpl $256, %edx
    jb   .LBB1
```

Measured on the pinned kernel (`-O2`, `taskset -c 0`, paired rounds):
**66.35 → 42.85 cycles/call**. The `zlib_ng_compare256` benchmark's
lccc/GCC median ratio moves **1.219 → 1.069**; the residual gap is loop
layout, not address arithmetic.

### Legality

The rewrite is applied only when all of the following hold, and every
condition is checked against the IR rather than assumed:

* every pointer phi in the header has exactly two incomings — an invariant
  base and a latch `add phi, K` used by that phi alone;
* every pointer in the group steps by the same `K`;
* each pointer's remaining uses are the `ptr` operand of a header
  `Load`/`Store` (or a header `Copy` of the phi);
* the integer index phi starts at `0` and steps by the same `K` in the latch.
  The latch must branch directly to the header on a comparison of that index
  or its exact increment against a positive literal no larger than `1 << 31`.
  The comparison and branch direction must prove termination before wrapping;
  for `!=`, the bound must be reachable by the stride. A checked bound also
  prevents crossing the sign bit when the comparison or index extension is
  signed. A literal alone is not a trip-count proof;
* every base is a parameter or is defined outside and dominates the header
  (`CfgAnalysis::idom` plus `analysis::dominates_idom`), so the address can
  be inserted once, after the phis;
* at most four pointer phis per group.

A variable bound, a second use of a pointer value, a different stride per
array, or a latch shared with another loop all leave the loop untouched.
`tests/regression/ptr_iv_collapse_scan.c` checks the declined shapes for
correctness as well as the accepted ones.

### Schedule and switches

The pass runs inside the GVN/LICM/IVSR/UNIVSR driver, after `univsr`. IVSR is
first-iteration only and the pointer phis this pass consumes appear only after
mem2reg, so it rides the canonical-CFG schedule and reaches its fixpoint on
the second iteration.

* `CCC_NO_PTR_IV_COLLAPSE=1` — disable the pass for a whole compilation.
* `CCC_DISABLE_PASSES=ptr_iv_collapse` — the same thing through the generic
  pass kill switch.

`tests/regression/check_ptr_iv_collapse_codegen.sh` is the instruction-
selection half of the test: the collapsed loop must address both arrays
through one index register and carry exactly one step instruction, and the
same translation unit built with the pass disabled must not.

## `promote_guarded_counts` — the zero guard is a proof, not a branch

`__builtin_clz(x | 1)` and `__builtin_ctz(x | 0x80000000)` exist to make the
undefined input defined: the OR forces a bit on, so the operand can never be
zero — for every `x`, including the `x == 0` case the guard was written for.
CVP previously derived that only from a dominating predicate, and its body
requires two blocks, so a straight-line leaf never got it and the baseline x86
lowering emitted the defined-zero diamond around a `bsr` whose zero arm is
unreachable.

The proof now also walks the definition — copies, zero-preserving casts, and
`Or` with a resolvable nonzero constant — and runs before CVP's two-block
bail-out. `byte_bit_length` in `llvm_word_counts.c`
(`(uint8_t)((clz(x|1) ^ 31) + 1)`) drops from 14 instructions to 5, matching
GCC's 5; `count_mixed` drops from 14 to 10.

`And` with a nonzero constant is deliberately **not** admitted: a bitwise AND
can still clear every bit, so it proves nothing about zero.
`tests/regression/clz_zero_guard.c` covers the differential, and
`check_ptr_iv_collapse_codegen.sh` also pins that the unguarded counts keep
their defined-zero fix-up while the guarded forms do not.
