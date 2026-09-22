# Follow-up: i128 constant multiply — a truncation miscompile, a fast path, and two measured dead ends

Session of 2026-09-22, rebased on `origin/main` `e08565c`.

The work started from an oracle sweep: ten kernel-idiomatic functions compiled
with lccc and with GCC 16.2, Clang 23.1, ICX and ICC through
`scripts/godbolt.py`. lccc emitted 121 instructions where Clang emitted 55.
Per-function attribution put almost all of the gap in two functions, and both
turned out to sit on the same 128-bit multiply path.

## 1. The miscompile: `to_i64()` truncation in `const_power_of_two`

`x * 0xffffffffffffffff0000000000000001` returned `x` unchanged, at `-O1` and
above. `-O0` was correct, and GCC, Clang and ICX are correct.

`simplify::const_power_of_two` read the multiplier through
`IrConst::to_i64()`, which narrows `IrConst::I128(v)` to `v as i64`. The
constant above truncates to `1`, which passes the power-of-two test with
`trailing_zeros() == 0`, so `try_mul_power_of_two` rewrote `x * K` into
`x << 0`, and a later pass folded the zero shift into a copy.

The blast radius is every `__int128` multiply by a constant whose low 64 bits
happen to be a power of two while the high half is non-zero. The same signed
read also hid the single set bit of 2^63 in an unsigned 64-bit multiplier, so
`x * 0x8000000000000000u` missed the shift entirely — a missed optimisation
rather than a miscompile, but the same line of code.

The fix reads the constant as a bit pattern and masks it to the operation's
width before the test. This is the third instance of this exact bug class in
the tree: `simplify::is_all_ones`, `simplify::is_neg_one` and
`sccp::is_all_ones` all carry comments saying they must not go through
`to_i64()` for 128-bit constants. Those three were fixed individually; the
predicate that strength-reduces multiplies was missed.

## 2. The fast path: constant multipliers

`emit_i128_mul_impl` expanded every 128-bit multiply into a full 128x128
schoolbook product — 13 instructions, including a `pushq`/`popq` pair used
purely as a register move. Constant multipliers dominate real i128 arithmetic:
strength-reduced division is `mulhi(x, M)` where the magic `M` always has a
zero high half, so one of the three product terms is identically zero and the
constant was being materialised into the accumulator pair first anyway.

Backends now receive the multiplier's halves, following the pattern the
constant-shift path already uses. The default implementation ignores the split
and runs the generic sequence, so ARM, i686 and RISC-V are unchanged. On
x86-64 the sequence collapses to five instructions for the `hi == 0` shape.

Measured on the oracle probe:

| function | lccc before | lccc after | GCC 16.2 | Clang 23.1 | ICX |
|---|---|---|---|---|---|
| `div_const` (`x / 7u`) | 35 | 20 | 7 | 8 | 8 |
| `mod_const` (`x % 1000`) | 38 | 23 | 11 | 11 | 11 |
| probe corpus (10 functions) | 121 | 91 | 58 | 55 | 74 |

lccc now beats ICC (80) and ICX (74) on the corpus. The remaining gap to GCC
and Clang is concentrated in the two division functions.

## 3. The test that caught a live bug

`tests/regression/i128_const_multiply.c` covers 13 multipliers x 14 left
operands = 182 cases against a limb-wise reference built from 32-bit pieces,
so the oracle shares no codegen with the sequence under test.

It earned its place immediately. The first version of the fast path was wrong
in the `hi != 0` branch: the second cross term consumes `lhs.lo`, which `mulq`
has already overwritten in `%rax`, so the low half has to be parked before the
multiply. The corruption lands only in the high 64 bits of the product, which
**no division test can see**, because division keeps only `mulhi`. The
pre-existing `x86_fpo_i128_div_push_depth` caught it by luck, through a
constant-folded `q * b`; the new test catches it by construction, 24 of 182
cases.

Both halves of the fast path are covered deliberately. `hi == 0` is the common
shape and would have passed with the bug present.

## 4. Two measured dead ends — recorded so they are not retried

**`cmov` destination coalescing.** Branchless clamp emits two redundant
`movl`s:

```
cmpl %edx, %edi          cmpl %edx, %edi
movq %rdi, %r9           movl %esi, %eax
cmovgl %edx, %r9d        cmovle %edi, %edx
cmpl %esi, %edi          cmpl %esi, %edi
movl %r9d, %r11d         cmovge %edx, %eax
cmovll %esi, %r11d       ret
movl %r11d, %eax
ret                      lccc 8, GCC/Clang/ICX 6
```

The obvious fix is to coalesce a `Select` destination with its `false_val`,
since `lower_select` preloads the destination from exactly that operand. That
was implemented — in both `build_copy_alias_map` and, after the first attempt
proved inert, in the `build_cfg_copy_alias_map` affinity graph that is the
default path. **It changed nothing: 172249 instructions across 819 files before
and after, bit-identical.** The reason is visible in the listing above: the
emitted form is `cmovgl` after `cmpl`, whereas `lower_select` emits `cmovne`
after `test`. This code does not come from `lower_select` at all, so the IR
edge never reaches the emitter that produces the redundant moves. The change
was reverted rather than shipped as a no-op. Whoever picks this up should start
from the MachInst builder that emits a `Cmov` with a compare-derived condition
code, not from the IR.

**Dead frames.** 48 of 446 functions that reserve a frame (10.8%, 1040 bytes)
never touch it. Worth a look, but the detector counts `push`/`pop` as a use, so
the interesting cases — like `div7`'s 112-byte frame around a single
`pushq %rbx` — are not in that number. The real cost there is that the
allocator reaches for a callee-saved register where GCC keeps everything in
`%rax`/`%rdx`/`%rdi`; that is register-allocation pressure, not frame sizing.

## 5. Test infrastructure

`tests/regression/i686_nonlocal_goto_callee_saved` was failing on
`undefined symbols: abort`. It is now freestanding like its siblings, entering
the i386 syscalls directly, and keeps PIC because PIC is what puts the GOT base
in `%ebx` — the entire exposure mechanism. Verified not weakened: reverting the
NonlocalGoto pool-clear in `i686/codegen/prologue.rs` fails it with `rc=-11`.

`run_regression.py` no longer blames lccc for a missing multilib. A `-m32`
compile failure is only downgraded to a skip when the host's own GCC fails the
same translation unit identically, so a `-m32` test that GCC compiles and lccc
does not stays a hard failure.

## 6. State

Suite 774 passed / 3 failed at session start, **776 passed / 0 failed** at the
end. The three former failures were two environment limitations and one real
link defect hidden behind them.

Open, in rough order of value:

* `div_const`/`mod_const` are still 20/23 against GCC's 7/11. GCC recognises
  `x * (2^128 - 2^64 + 1)` and reduces it algebraically to `x.hi - x.lo`;
  lccc runs the general sequence. A `mulhi`-aware fold at the IR level would
  close most of the remaining oracle gap on this probe.
* The i128 value round-trips through memory in `mul_hi_ones`
  (`movq %rdi,72(%rsp); movdqu …; movq 48(%rsp),%rax`) even though both halves
  stay in registers.
* `CCC_NO_SELECT_COALESCE` is not a real env flag — it was introduced and
  reverted with the dead-end change above.
* The kernel build/boot gate could not be re-run: the linux-cachymod tree is
  not present in this workspace.
