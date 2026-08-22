# Session 61 — 25 complete-unroll substitution defects

Date: 2026-08-22

Upstream base: `fdeea471` (latest merged `ms178/lccc` main).
All compiler builds used `scripts/build_lccc_fast.sh` only.

## Root cause

The complete-unroll path cloned loop instructions with two handwritten value
substitution functions. They understood a small algebraic subset of the IR.
Every omitted use position retained the original iteration's SSA value after a
clone, or retained the loop-header IV after the loop disappeared. This is a
miscompile, not merely a missed optimization.

The first concrete GCC torture reproducer was `20040706-1.c`:

```c
int i;
for (i = 0; i < 10; ++i)
    continue;
if (i < 10)
    abort();
```

Complete unrolling removed the loop but left the post-loop use bound to the
initial IV, making `abort()` unconditional. The same defect corrupted raw
pointer fields in `20010910-1.c` and `20050502-2.c`.

## Architectural correction

Manual variant switches were deleted. Unroll cloning now uses the canonical IR
visitors:

```text
Instruction::for_each_operand_mut
Instruction::for_each_value_use_mut
Terminator::for_each_operand_mut
```

The final loop IV is explicitly replaced by `init + trip * step` in every block
outside the removed natural loop, including terminators. This makes the
transformation complete by construction when new IR variants are added: the
central visitor contract, already tested by the IR layer, is the single source
of truth.

## Twenty-five distinct fixed use positions

The old complete-unroll substitution omitted each of these independently
observable positions. A focused unit test constructs all 25 and proves every
one is renamed:

1. `Memcpy.dest`
2. `Memcpy.src`
3. `AtomicCmpxchg.ptr`
4. `AtomicCmpxchg.expected`
5. `AtomicCmpxchg.desired`
6. `VaArgStruct.dest_ptr`
7. `VaArgStruct.va_list_ptr`
8. `VaCopy.dest_ptr`
9. `VaCopy.src_ptr`
10. `AtomicRmw.ptr`
11. `AtomicRmw.val`
12. `AtomicStore.ptr`
13. `AtomicStore.val`
14. first `Phi.incoming` operand
15. second `Phi.incoming` operand
16. `VaArg.va_list_ptr`
17. `VaStart.va_list_ptr`
18. `VaEnd.va_list_ptr`
19. `SetReturnF64Second.src`
20. `SetReturnF32Second.src`
21. `SetReturnF128Second.src`
22. `AtomicInc.ptr`
23. `AtomicLoad.ptr`
24. post-loop `Return` operand
25. post-loop `CondBranch.cond`

These are 25 separate def-use edges with different memory, atomic, ABI,
variadic, complex-return, phi, and control-flow semantics. The fix is shared
because duplicating 25 ad-hoc repairs would recreate the design error.

## Reproducers and measured impact

A maintained runtime regression, `complete_unroll_exit_iv.c`, covers the empty
body/final-IV case on both x86 and AArch64.

GCC C torture, first 500 sorted execute tests:

```text
before: 454 PASS, 46 FAIL
 after: 457 PASS, 43 FAIL
```

Newly passing external tests:

* `20010910-1.c` — unrolled pointer-field initialization/verification;
* `20040706-1.c` — final IV after an empty-body loop;
* `20050502-2.c` — cloned non-algebraic value uses.

The full 1694-file screening remains an explicit backlog rather than a hidden
claim: prior baseline was 1535 pass / 152 fail / 7 skip; the remaining failures
contain frontend nested-function gaps, ABI/complex/varargs defects, and runtime
miscompilations.

## Validation

```text
Focused 25-position unit test : PASS
Rust library tests            : 1090 PASS, 6 ignored, 0 FAIL
Correctness suite             : 50/50 PASS
x86 regression                : 438 PASS, 0 FAIL
ARM regression                : 269 PASS, 51 retained FAIL, 69 SKIP
GCC torture first 500         : 457 PASS, 43 FAIL
GAS                           : GNU assembler 2.47.20260726
Linker suite                  : 159 PASS, 1 pre-existing version-script FAIL
```

The ARM pass count increased by one due to the new regression; failure identity
was unchanged outside the fixed unroll cases.

## Performance and lccc-ld screening

Fastbuild LCCC/integrated `lccc-ld` produced correct binaries for every measured
benchmark. Pinned randomized VM ratios versus GCC:

```text
aarch64_select_patterns  1.3646
gzip CRC-32              1.0641
zlib-ng Adler-32         1.5246
Expat XML scan           2.0562
geomean                   1.4607
```

No runtime win is claimed from noisy host-VM measurements. Complete unrolling
retains the same profitability budget; the change repairs value flow and does
not add generated instructions to legal transforms.
