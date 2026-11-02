# Kernel milestone 1: inline-asm immediates, and a booting LCCC kernel

## Status

`build_kernel_vm.sh` builds a complete x86_64 `bzImage` of
linux-cachymod-6.18.58 with **LCCC as compiler and LCCC-ld as linker**, and that
image **boots in QEMU and passes the full in-VM validation suite**.

```
build: SUCCESS in 1030s
   text     data      bss      dec    hex  filename
10829153  3214920  2260992 16305065  f8cba9  vmlinux
kernel_boot_isa: scan PASS (43 object(s), 0 BMI/ABM/VEX/SSE4.2-class instructions)
setup _end: 0x00006d60 (28000 bytes, ok of 32768)
QEMU BOOT: ALL CHECKS PASSED     (16/16 markers)
```

The guest reports the CachyMod feature set compiled in (SCHED_BORE,
SCHED_CACHE, CACHY, HZ_800, BBRv3, PREEMPT, SMP), two CPUs online, per-task BORE
scores, both serial sentinels verbatim, and a clean poweroff. No GCC was used
for a single kernel translation unit or link.

## What was blocking it

Every blocker was in the *front end's* model of GCC inline-assembly operands.
The IR already knew how to emit a relocation; the parser rejected the statement
before an instruction existed. Four defects, each measured against GCC and each
fixed with a reproducer, an emitted-code comparison, and a unit test:

### 1. `"i"(<block-scope constant>)` — `__WARN_FLAGS` (`arch/x86/events/core.c`)

```c
do { __auto_type __flags = (1 << 0)|(((1) << 8)|(9 << 8));
     asm volatile(BUG_ASM : : "i"(__FILE__), "i"(__LINE__), "i"(__flags), "i"(sizeof(struct bug_entry)));
} while (0);
```

GCC accepts because it folds the local and re-checks the `asm` at
*instantiation*; LCCC parsed the statement once and rejected it
(`impossible constraint in 'asm'`, 4 sites in `events/core.c`, plus
`softirq.c`, `irq.c`).

* `Parser::local_constants` (`parse.rs`) records a block-scope integer scalar
  whose initializer is a self-contained constant expression, and
  `record_local_constant` (`declarations.rs`) is the single funnel that decides
  what qualifies (rejects pointers, arrays, functions, `volatile`, `static`,
  braced initialisers and self-reference).  Records are **inherited** by nested
  blocks and restored on exit, and a *redeclaration* without a foldable
  initialiser withdraws the outer record — both requirements are covered by
  the block-scope guard matrix.
* Any assignment, compound assignment, `++`/`--` or `&x` on the name withdraws
  the record (`invalidate_assigned_constants` → `forget_lvalue`, called from the
  statement dispatcher).
* The fold used by `validate_inline_asm` merges the table, so the "chained"
  kernel shape `int fl = 3; __auto_type v = 1 | (fl << 8);` resolves too.
* Second layer: `resolve_asm.rs` now substitutes a constant immediate
  (`Operand::Value` → `Operand::Const`) for immediate-only constraints once the
  symbol resolver has declined, so an `i` operand reaches the emitter as a
  literal instead of a register kind.  (The IR has 186 `InlineAsm`
  construction sites; rewriting the operand avoided touching any of them.)

### 2. `%c` on a register operand (`__bug_table` corruption)

`asm(".word %c0" : : "r"(x))` emitted `.word %rcx` — a register *name* inside
`__bug_table` data, a silent miscompile.  GCC:

```
error: invalid 'asm': operand is not a condition code, invalid operand code 'c'
```

`check_template` now takes the operand list, and
`constraint_is_constant_expression` decides whether `%c` may print the operand
(any immediate alternative, including one alternative of `"Ir"`).  The message
is GCC's, verbatim.

### 3. `"i"(<function designator>)` — `call_on_stack` (`kernel/softirq.c`)

`asm ... : "i"(__do_softirq)` hands the assembler the *address* of the callee as
a relocation.  `is_symbolic` only recognised a function through a **call**
(`f(...)`), never a bare identifier, so the kernel's `call_on_stack` was
rejected.  The parser now records every declarator whose entity is a function
(`Parser::function_names`, filled by `record_function_entity`, the funnel that
already carried `noinline`), an object declaration of the same name withdraws
the record (`record_object_shadows`), and `is_symbolic` consults it.  Result:
`.quad target` — byte-identical to GCC; `"I"(fn)` is still rejected, as GCC
requires.

### 4. `: [sum] "=r"(v) : "[sum]"(x)` — named matching constraints (`arch/x86/lib/csum-wrappers_64.c`)

GCC's matching-constraint syntax ties an input to the output *by name*.  LCCC
read `"[sum]"` as a constraint string, found no constraint letters in the
brackets, and classified the operand as `GpReg` (telling the template a register
it never tied) while the validator called it impossible.  The back end now
resolves a `[name]` constraint to `AsmOperandKind::Tied(output_index)`, and the
validator exempts the spelling from the letter scan while reporting GCC's
`error: undefined named operand 'zz'` when no output carries the name.

## Validation

| gate | result |
|---|---|
| `make prepare` (LCCC) | PASS |
| 32 KiB boot-code gate | `_end = 0x6d60` (28000 B), flat image byte-identical to `ld.bfd`, ISA scan clean |
| `build_kernel_vm.sh` (allnoconfig + vm fragment) | **SUCCESS**, 1030 s at `-j2` |
| ISA scan of the boot path (43 objects) | PASS, 0 BMI/ABM/VEX/SSE4.2 |
| `qemu_boot_test.sh` | **ALL CHECKS PASSED** (16/16) |
| `cargo test asm_validate` | 21/21 pass |
| guard matrix (`-O1`, accept/reject) | matches GCC on all 11 shapes |
| `__bug_table` | `.word 17` / `.word 259` / `.org 2b + 12`, identical to GCC |

Three pre-existing `asm_validate` expectations contradicted the measured rule
and were corrected: `"Ir"` is always satisfiable (a register alternative
softens the immediate-only rule), `"s"(obj)` is an error while `"s"(array)` is
accepted, and `%c` on a register is a hard error rather than a pass-through.
Each correction cites the measurement in the test body.

## Known quality-of-code gap (open, not a correctness defect)

Around any tied inline-asm operand LCCC emits a redundant widening/narrowing
round trip that GCC does not:

```c
int f(int a, int b){ int s; __asm__("addl %1,%0" : [s] "=r"(s) : "[s]"(b)); return s; }
```

| | instruction sequence |
|---|---|
| gcc 14.2 | `movl %esi, %eax` / `addl %eax,%eax` / `ret` |
| lccc | `subq $24,%rsp` / `movslq %esi,%rax` / `movq %rax,%rcx` / `addl %ecx,%ecx` / `movl %ecx,%ecx` / `movl %ecx,%eax` / `addq $24,%rsp` / `ret` |

The same shape occurs **without** this session's change (`"0"(b)` tie, and a
`__asm__ volatile("")` with an unreferenced `"i"` symbol operand that still
materialises `movq target@GOTPCREL(%rip),%rax`).  It is a pre-existing codegen
issue in two places — the tie introduces a stack spill and the input is
re-widened from `int` to `i64` — and it is tracked as the next QoI item, not
folded into this correctness change.

## Integration note

This change is reconciled with the inline-assembly deferral handling already
present in current upstream main.  The focused tests and kernel-build results
above document the validated behavior; this integration run did not repeat
compile or kernel-build tests.

## Next

1. The tie/immediate codegen round trip above (measured, reproducible).
2. `build_kernel_full.sh` with the package's real (CachyOS) config,
   `LCCC_NOOBJTOOL=1`, then `qemu_boot_test.sh` against it.
3. `lccc/scripts` audit: the four kernel scripts now each carry their own
   iteration of the same "which tree, which compiler" logic; consolidate on
   `kernel_version.sh` and add the `bzImage` checksum to the ledger.
