# Session 65 — GNU C nested functions complete; computed-goto labels; 15 more torture tests

Date: 2026-08-23

Upstream base: `b7ce7d8c` (latest `ms178/lccc` main at session start; PR #215).
Build: `cargo build --profile fastbuild --locked -j2` (Rust 1.98.0). No swap
available in this sandbox (`ensure_swap.sh` warns and continues).

## Mandate

Continue the nested-functions work, then prioritize clusters D (bitfields)
and B (computed goto); keep the deliverable current; re-base on latest main.

## Delivered: 15 of the 21 remaining torture failures fixed

### Cluster A — GNU C nested functions: 11/11

`20000822-1, 20010209-1, 20010605-1, 20030501-1, 20040520-1, 20061220-1,
20090219-1, 920415-1, 920428-2, 920501-7, 920612-2` — all pass at -O2.

Full implementation across the whole stack:

* **Parser** (`nested_functions.rs`): speculative parse with complete state
  rollback (position, attrs, typedefs/shadowing, pragma stack, enum
  constants, error count); handles typed, implicit-int (`main(){...}`), and
  K&R (`f(a) int a; {...}`) forms; rejects pointer-to-function declarators.
* **Sema**: nested functions analyzed with proper lexical scoping (params
  shadow enclosing locals), mangled `parent.name` signature registration,
  symbol declared in the enclosing block scope.
* **IR**: five new instructions — `GetStaticChain`, `SetStaticChain`,
  `InitTrampoline`, `NonlocalGotoSave`, `NonlocalGoto` — with complete
  visitor coverage across all 14 passes that enumerate instructions
  (liveness, GVN, inline, unroll, outline-switch, copy-prop, IPCP, LICM,
  loop-memory-promote, split-ranges, tail-call-elim, vector-temp-promotion,
  DSE classification, codegen).
* **Lowering** (`ir/lowering/nested_functions.rs`, ~1100 LOC):
  * Retroactive framing: captured enclosing locals move into a frame struct
    in the parent; uses (including pending `instrs`) rewritten; frame
    members are 16-byte aligned; the alloca size patches at finalize.
  * **Memoized framing** — a second nested function capturing the same
    local reuses the identical offset (the siblings bug: re-framing
    allocated a never-written second member).
  * Transitive captures: grandparent variables reached by walking frame
    link words; every nested function eagerly creates a frame with an
    initialized link (unconditionally — a nested function's frame is the
    chain hop-through point for deeper descendants).
  * **Late SetStaticChain**: the chain register load is spliced immediately
    before the Call instruction (backward scan) — emitting it before
    argument evaluation lost the chain whenever an argument contained
    another nested call (`add_base(add_base(x) - base)`).
  * Self-recursion registration before the body is lowered (`void y(a)
    { ... y(a-1); }`).
  * Sibling/uncle visibility inheritance (child copies the parent's
    nested-function map).
* **x86 backend** (`codegen/nested_fn.rs`): static chain in `%r10` (GCC
  convention); 24-byte stack trampoline `movabs chain,%r10;
  jmp *[rip+0]; .quad func` — an ABSOLUTE indirect jump, required because
  with ASLR the stack and text segments are far beyond ±2 GiB so a rel32
  `jmp` from the stack cannot reach the function (this exact bug shipped as
  the first trampoline version); `NonlocalGotoSave/NonlocalGoto` restore
  the FULL frame state — rbp, rsp, and all SysV callee-saved GPRs (rbx,
  r12–r15; 7-slot save area), because code after the landing label reads
  callee-saved values the abandoned nested call chain left clobbered
  (920501-7: `a` in %rbx).
* **Cross-function labels**: block IDs are renumbered per function (global
  uniqueness pass) and target blocks can be merged away, so non-local gotos
  and `&&label` from nested functions use named `.LNLG.<fn>.<label>`
  aliases emitted by the parent AS AN INLINEASM LABEL AT THE LABEL
  STATEMENT — a real instruction that survives every optimization. The
  alias target block is kept alive via `global_init_label_blocks` (the
  existing externally-referenced-label keep-alive).
* **Linker**: `PT_GNU_STACK` becomes RWE when any input object's
  `.note.GNU-stack` has SHF_EXECINSTR (GNU ld semantics) — required for
  trampolines.
* **Sema**: bare `return;` in a non-void function downgraded to a warning
  (gcc89 torture code relies on it).
* **Codegen correctness fixes found on the way**: `GetStaticChain` needed
  regalloc eligibility + dead-dest handling; alloca-coalescing needed the
  chain instructions classified as USES (not escapes) so the frame pointer
  stays alive; non-local goto must TERMINATE its block (the instruction
  buffer would otherwise be cleared by start_block).

Regression: `tests/regression/nested_functions.c` (7 shapes: capture-write,
VLA-param, inline, write-then-read, loop capture, two-level nesting with
transitive captures, sibling calls) at -O2.

### Cluster B — computed goto static labels: 4/4

`20071220-1, 20041214-1, 20071210-1, 920302-1` — all pass at -O2.

Root cause: the global block-label renumbering in `driver/pipeline.rs`
updated every BlockId reference but NOT the label NAMES baked into static
initializer data. `static void *t[] = { &&lbl };` lowers to
`GlobalInit::GlobalAddr(".LBBn")`; the emitted `.quad` kept pointing at the
pre-renumber id while the function emitted the renumbered label — the
indirect jump landed on a wrong or nonexistent block. Fix: the renumbering
rewrites `.LBB<n>` names inside `GlobalInit::GlobalAddr` (recursing through
Array/Compound initializers) via the same renumber_map.

Regression: `tests/regression/computed_goto_static_labels.c` (static
&&label table, distinct targets, O0/O1/O2).

### Pre-existing issues

* `check_fma_dest_coalesce_codegen` fixture FIXED: it demanded the guarded
  xmm0 fallback for the aliasing functions, but the compiler now emits the
  better copy-then-accumulate form. The check now validates the semantic
  property (no destructive FMA into an un-copied ABI source register).
* `check_arm_csinc_select`: requires aarch64 libc headers
  (`/usr/include/aarch64-linux-gnu/bits/...`) which this sandbox lacks —
  environment limitation, not a code defect (only `asm/` headers exist).
* i686 trio (rc=159): no 32-bit ELF interpreter on the host — same class.

## Remaining torture failures (6, root-caused)

| Test(s) | Root cause (diagnosed) |
| --- | --- |
| `20040709-{1,2,3}` | `struct W/X/Y/Z { long double l; unsigned k:N, ... }` by-value sret call: the IR is CORRECT (`args=[sret_temp, &y]`, `is_sret`) but the x86 backend passes `%rdi = &struct_arg` instead of the sret temp — the return value then memcpy's garbage over the callee's result. Bisected precisely to struct W (bisect_bitfield.py); the failing check is `((v+a)&mask) != r` in fn1. Suspect: `emit_call_stack_args`' `StructByValStack` path leaves `%rax` = struct pointer and the register-arg phase's accumulator-cache handling for the sret temp operand goes stale. struct A–V (all non-long-double shapes, incl. 64-bit bitfields) pass. |
| `20020412-1` | VLA-struct `va_arg` (`va_arg(ap, typeof(d))` with runtime-size struct) — needs the 20070919-1 runtime-memcpy machinery extended into `va_arg` aggregate handling. |
| `20230630-{2,4}` | `__attribute__((scalar_storage_order("big-endian")))` bitfield load/store byte order — layout metadata + reversed field access not implemented. |

## Validation

```
cargo test --profile fastbuild --lib        # 1101 passed, 0 failed
tests/regression/run_regression.sh          # 461 passed, 2 skipped, 4 failed
                                            # (i686 loader x3, aarch64 headers
                                            #  — all environment limits)
tests/correctness/run_correctness.py        # 50/50 vs GCC
torture clusters A+B                        # 15/15
session-62/63/64 fix lists re-verified      # stable
```

## Follow-up (next session)

1. **20040709 sret bug** (highest value — plain `long double` member triggers
   it): instrument `emit_call_reg_args`' accumulator cache around the
   StructByValStack phase; the working 32-byte double-struct case (`big.c`
   passes) vs the failing W case differ only in the member type.
2. VLA varargs (20020412-1) — extend VaArgStruct with a runtime size operand.
3. SSO bitfields (20230630) — layout attribute + byte-swapped accessors.
4. `arm_csinc_select` etc. need a cross-libc environment or header-free test.

## Do not

- Do NOT emit trampoline jumps as rel32 (ASLR: stack→text exceeds ±2 GiB).
- Do NOT clear `dirty_upper_ymm` in `invalidate_vec_peephole` (physical state).
- Do NOT key cross-function label references on BlockIds (per-function
  renumbering) or on (block-id → alias) maps (blocks merge away) — the
  InlineAsm-label-at-the-label-statement form is the load-bearing mechanism.
- Do NOT frame a captured variable twice (memoize by name).
