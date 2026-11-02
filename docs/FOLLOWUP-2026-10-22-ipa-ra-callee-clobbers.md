# FOLLOWUP 2026-10-22 — IPA-RA callee-clobber refinement is the largest remaining runtime gap

> **Status 2026-10-24 (correction):** IPA-RA is implemented. The premise below, that callee clobbers were the
> largest remaining `hot_web_loop` gap, was wrong. With IPA-RA disabled (`CCC_DISABLE_PASSES=ipa_ra`),
> `hot_web_loop` reaches 841.2M Ir (-30%, tying GCC 16.2, Clang 23.1.3 and ICX 2026.2) through secondary
> latch-copy phi coalescing. IPA-RA itself changes two functions statically and is runtime-neutral on the
> corpus. See `FOLLOWUP-2026-10-24-latch-coalescing-and-ipa-ra.md`.

## Summary

`hot_web_loop` is the worst benchmark in the corpus at **1.549× slower than GCC
16.2** on a 99.8 ms median, so the measurement is well clear of the 20 ms noise
floor. The gap is not an algebraic missed optimization and not a scheduling
detail. It is the absence of **IPA-RA**: lccc has no per-callee register-clobber
information, so every value live across a call must occupy a callee-saved
register. GCC knows what `helper` actually touches and therefore keeps the four
hot recurrence values in *caller*-saved registers, which frees enough
callee-saved registers to eliminate every spill.

Confirmed by inspection, not inference:

```
$ grep -rn 'ipa_ra|callee_clobber|call_clobbers|clobber_set|actual_clobbers' src/
(no matches)
```

The only clobber helpers in the tree are instruction-local
(`src/backend/arm/codegen/peephole.rs:5072 instr_clobbers_reg`,
`src/backend/generation.rs:6282 clobber_after_call_like`). Nothing models a
*known callee's* register usage.

## The evidence

Source: `tests/benchmark/programs/hot_web_loop.c`. `helper` is
`static __attribute__((noinline))` and its whole body is
`(x ^ salt) * 0xd6e8feb86659fd93` — it provably touches only `%rax`/`%rdx` and
the flags. `exercise` carries ten 64-bit values live across the `call`.

### GCC 16.2 `-O2` — 13 instructions, zero spills, zero stack references

```asm
.L5:
	call	helper
	rolq	$7, %rcx          # d
	addq	%rax, %r9         # a += h
	movq	%rax, %rdx
	leaq	(%r9,%r8,8), %rax # a + (b << 3)   -- one scaled LEA
	xorq	%rax, %r8
	movq	%rsi, %rax
	shrq	$5, %rax
	xorq	%r8, %rax
	addq	%rax, %rsi        # c += b ^ (c >> 5)
	xorq	%rsi, %rcx        # d ^= c
	testb	%dl, %dl
	jne	.L4
```

`a`,`b`,`c`,`d` live in `%r9`,`%r8`,`%rsi`,`%rcx` — all caller-saved, all
computed **in place**. The six cold invariants `p..u` take `%r10`,`%r13`,
`%r12`,`%rbp`,`%rbx`,`%r11`. Whole function: **0 stack references**, 5
callee-saved registers used.

### lccc `-O2` — 29 instructions on the same path, 18 stack references

```asm
.LBB3:
	movl	%r15d, %edi
	call	helper
	movq	%rax, %r10
	leaq	(%r12, %r10, 1), %r8
	leaq	0(,%r14,8), %r9
	leaq	(%r8, %r9, 1), %rdi
	movq	%r14, %rsi
	xorq	%rdi, %rsi
	movq	%rbp, %rdx
	shrq	$5, %rdx
	xorq	%rsi, %rdx
	leaq	(%rbp, %rdx, 1), %r9
	movq	%r13, %rdi
	rolq	$7, %rdi
	xorq	%r9, %rdi
	andq	$255, %r10
	je	.LBB5
.LBB4:
	movq	%r9, %rdx
	movq	%rsi, %r11
	movq	%rdi, %r10
	movq	%r8, 56(%rsp)      # <-- a  STORED every iteration
	jmp	.LBB6
.LBB5:
	...                          # rare path (h & 255 == 0)
	movq	%r8, 56(%rsp)      # <-- a  STORED every iteration
	...
.LBB6:
	addl	$1, %r15d
	movq	%rdx, %rbp
	movq	%r11, %r14
	movq	%r10, %r13
	movq	56(%rsp), %r12     # <-- a  RELOADED every iteration
	cmpl	$40000000, %r15d
	jb	.LBB3
```

All six callee-saved registers are consumed (`%rbx`=`p`, `%rbp`=`c`, `%r12`=`a`,
`%r13`=`d`, `%r14`=`b`, `%r15`=`i`), and the five remaining cold invariants go
to `16..48(%rsp)`.

### The single defect that IPA-RA removes

`%r12` is read exactly once in the loop, at `leaq (%r12, %r10, 1), %r8`, and
written exactly once, at `movq 56(%rsp), %r12`. Verified:

```
$ sed -n '/^\.LBB3:/,/^\.LBB7:/p' h_l.s | grep -n 'r12'
7:    leaq (%r12, %r10, 1), %r8
44:   movq 56(%rsp), %r12
```

So slot `56(%rsp)` is a **pure phi-spill slot**: `%r8` is stored on both
incoming edges and reloaded at the join, while its destination register sits
dead the whole way. `movq %r8, %r12` on each edge — or, with coalescing, no move
at all — is equivalent. Per iteration this is one store, one load, and a
store-to-load dependency inside the recurrence, all of it avoidable.

The phi coalescer sees the shape and declines it:

```
[PHI_COALESCE] BLOCKED phi_dest=Value(130) src=Value(136): multi-def source outside the Part-2 chain shape
[PHI_COALESCE] BLOCKED phi_dest=Value(132) src=Value(137): multi-def source outside the Part-2 chain shape
[PHI_COALESCE] BLOCKED phi_dest=Value(133) src=Value(138): multi-def source outside the Part-2 chain shape
[PHI_COALESCE] BLOCKED phi_dest=Value(134) src=Value(139): multi-def source outside the Part-2 chain shape
```

## Why this is the right target and not a local one

`hot_web_loop` is a deliberately hard allocator stress case, but the mechanism
is universal: **any value live across a call to a function whose body is in the
same module pays full caller-saved clobber cost**. The corpus-wide census for
this run shows the general symptom — lccc emits **1151 register-to-register
copies against GCC's 894 (1.29×)** over the 67 benchmark programs, while its
total instruction count is actually *lower* (10 025 vs 10 544). lccc is not
generating more work; it is generating more register shuffling to work around
clobber pessimism.

## Design sketch

1. **Collect.** For each function definition in the module, after allocation,
   record the set of physical registers it writes — its own uses plus the union
   of its callees' sets plus argument-staging and return registers.
2. **Order.** Allocate in reverse topological order of the call graph so a
   callee's set exists before a caller needs it. Any call-graph cycle, and any
   call to a declaration without a definition, an indirect call, or a function
   whose address escapes, falls back to the full caller-saved clobber set. That
   fallback must be the default, not the exception.
3. **Consume.** Replace the fixed caller-saved clobber set at a direct call to a
   known callee with its recorded set. This must not weaken the ABI for
   externally visible functions: a function with external linkage is still
   callable from elsewhere, so its *own* allocation must keep honouring the
   full convention even though callers inside the module may rely on the
   narrower set.
4. **Preserve soundness under later changes.** The recorded set is a property of
   the callee's final code. Any pass that can add register uses after
   allocation must invalidate it, or the sets must be recomputed. This is the
   part most likely to cause a silent miscompile and needs the most test
   coverage.

## What would count as done

- A red test on current main pinning the store/reload round-trip above and the
  callee-saved saturation, which fails before the change and passes after.
- `hot_web_loop` measured on the canonical output-checked paired runner with
  matched flags, complete raw rounds, and a median comfortably above 20 ms.
- The full 49-benchmark corpus in **one** runner invocation, base vs candidate,
  with no aggregate regression. A change that regresses the corpus aggregate
  must not ship as a performance PR.
- `phi_cfg` and the wider differential fuzz engines clean against GCC 16.2.

## Baseline recorded with this note

Canonical paired runner, `--compilers lccc,gcc --gcc gcc-16 --opt=-O2 --reps 7
--warmup 2`, GCC 16.2 as oracle, 49 benchmarks, no correctness warnings:

- **geometric mean 0.7884**, arithmetic mean 0.9684
- geomean over the 31 benchmarks with a median above 20 ms: **1.0098**
- best `fib` 0.015 (66.53× faster); worst `hot_web_loop` 1.549

Worst offenders (lccc/GCC, lccc median):

| benchmark | ratio | lccc ms |
| --- | ---: | ---: |
| hot_web_loop | 1.549 | 99.8 |
| byte_load_reassembly | 1.505 | 5.3 (short) |
| arith_loop | 1.174 | 117.9 |
| expat_xml_scan | 1.172 | 47.8 |
| aarch64_select_patterns | 1.169 | 115.7 |
| zstd_count | 1.154 | 13.7 (short) |
| nbody | 1.148 | 376.4 |
| linux_find_bit_scaled | 1.113 | 908.1 |
| glibc_strstr | 1.113 | 4641.1 |
| fannkuch | 1.105 | 2688.2 |

## Secondary observations from the same comparison

These are separate from IPA-RA and were **not** implemented here; they are
recorded so they are not rediscovered from scratch.

- **Scalar min/max/abs algebra.** LCCC's `IrBinOp` has no `Min`/`Max`, so
  min/max exist as `Cmp`+`Select` and none of the usual identities fire. On
  twelve reduced witnesses (LLVM #187057, #78891, #167003, #41552, #82414,
  #129947 and relatives) lccc emits more instructions than GCC 16.2 on **11 of
  12** and more than Clang 23.1.3 on **all 12**. Example, `max(0, a-b)`:

  ```asm
  # lccc                                    # gcc-16
  subl %esi, %edi                           subl %esi, %edi
  testl %edi, %edi        # redundant       movl $0, %eax
  movl $0, %ecx                             cmovns %edi, %eax
  movl %ecx, %eax         # redundant copy
  cmovgel %edi, %eax
  ```

  The `testl` duplicates flags the `subl` already set, and `movl $0,%ecx;
  movl %ecx,%eax` is a constant routed through a dead register.
- **Adjacent dead copies.** `mov X,%a ; mov %a,%b` with `%a` dead afterwards
  occurs **207 times across 81 of the 1203** regression/benchmark programs that
  compile cleanly at `-O2`. Straight-line only; the general form is larger.

Both are real, but each is an order of magnitude below the IPA-RA gap on the
benchmark that matters most.
