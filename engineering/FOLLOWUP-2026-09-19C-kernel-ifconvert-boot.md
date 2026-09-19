# Follow-up 2026-09-19C — kernel if-conversion corruption and boot validation

Base: upstream `main` `56858cbc4d4192a62317c56d2f6f37056e150a62`.
Target: patched CachyMod Linux 6.18.52, target code built exclusively by
`target/fastbuild/lccc` and `target/fastbuild/lccc-ld`; host-only Kbuild tools
remain GCC-built build machines. LCCC itself used the required `fastbuild`
profile (`-O1`, no LTO, `-j2`). A 4 GiB `/swapfile` was active throughout.

## Accomplished

### 1. Fixed the remaining workqueue/SCSI kernel miscompile

The prior GDB watchpoint result was reproduced in the newly built minimal
kernel. `llc_populate_cpu_shard_id()` emitted:

```asm
movslq c, %r10
shlq   $16, %r10
movl   %eax, cpu_shard_id(,%r10)
```

For `c == 1`, this writes 64 KiB beyond `cpu_shard_id`, exactly where the broad
configuration placed a pointer in `scsi_static_device_list`.

Pass-by-pass IR dumping localized the first corruption to the vectorizer's
if-conversion prepass. `sink_conditional_stores()` rebuilt a nested
`Cast(index) -> Shl(2) -> GEP` address in the merge block. Its recursive cloner
reserved the parent destination *after* cloning children. Parent and every
child consequently received the same SSA ID:

```text
v469 = cast v26
v469 = cast v469
v469 = shl v469, 2
```

The simplifier then correctly composed what appeared to be cascaded shifts on
each optimizer fixpoint iteration: `2 -> 4 -> 8 -> 16`. The defect was not an
instruction-selector scale decision; malformed non-SSA IR had already encoded
16 before code generation.

The fix reserves each node's fresh value before recursive descent. Children
are emitted before parents, preserving def-before-use ordering, while all
nodes have unique IDs. A failed speculative clone may leave an unused ID,
which is safe; an ID is never reused.

The unit regression now uses the nested cast/shift/GEP chain and proves:

* all cloned destinations are unique;
* the shift remains exactly two;
* two conditional stores become one phi-driven store.

The execution regression distills the kernel shape into
`branch_index_store()`. Before the fix LCCC advances the store address by 16
bytes and fails against GCC; after the fix it emits scale-4 SIB addressing and
passes under default, SSE-only, and the kernel's complete no-SIMD flag set.
The real kernel function now contains `leaq 0(,%rbx,4)` and no `shlq $16`.

### 2. Closed i686 short-branch addend coverage and defect

The requested 32-bit counterpart of the relocation-addend corpus exposed a
real independent bug: `loop .L+1` and `jecxz .L+1` produced zero displacements
instead of GNU-as-identical `ff` / `fd`. These branches are patched directly by
the shared relaxation engine and never emit an ELF relocation, so normalizing
only at relocation creation could not help them.

`JumpInfo` now stores a canonical base label and checked source addend
separately. Both relaxation distance classification and final disp8 patching
apply the addend exactly once, without overflow wrapping. The whole-object
ELF32 differential covers positive/negative bare-memory addends, loads,
stores, a non-mov bit operation, `loop`, and `jecxz`:

```text
=== asm-diff: 21 passed, 0 failed (21 cases, oracle=as) ===
```

The existing ELF64 addend corpus remains byte/relocation/symbol identical.

### 3. Made compiler changes invalidate compiled kernel objects

Kbuild fingerprints command lines, not executable contents. The old combined
LCCC/lccc-ld stamp purged only link products after *either* tool changed, so a
fixed compiler at the same path could relink stale miscompiled `.o` files.

`scripts/kernel_tool_identity.sh` now tracks compiler and linker SHA-256 values
independently:

* compiler change -> authoritative `make ARCH=x86_64 clean` (preserves config);
* linker-only change -> remove link products, preserve expensive objects;
* unchanged tools -> strict no-op;
* stamps are atomically installed; the obsolete combined stamp is retired.

The hermetic regression uses a fake Kbuild and tests first observation,
unchanged tools, linker-only invalidation, simultaneous compiler/linker change,
full SHA-256 stamps, and old-stamp retirement. It is wired into local and
GitHub CI. The post-fix kernel build observed the real compiler hash change and
printed `cleaning all Kbuild products` before rebuilding every target object.

### 4. Hardened harness-wipe recovery

A mid-session harness wipe retained `.lccc-prepared` and generated headers but
removed `scripts/Kbuild.include`. The former canary set accepted this truncated
~55k-file tree; Kbuild then failed misleadingly before compilation.

The preparation canaries now include distributed source/build files
(`Makefile`, `scripts/Kbuild.include`, `init/main.c`, `kernel/workqueue.c`) in
addition to generated and architecture files. `build_kernel_vm.sh` invokes the
idempotent preparation helper itself. The exact truncated tree was detected,
regenerated, patched 28/28, and prepared successfully.

### 5. Removed the redundant CLZ/CTZ zero machinery (2026-09-20)

The earlier 23-instruction reduction treated `__builtin_ctzl()` as an internal
operation with defined `Ctz(0) == width` semantics. ISO C instead makes every
CLZ/CTZ builtin undefined for zero. The backend consequently emitted a test,
conditional branch, width fallback, join branch, and an allocator preload even
inside `while (mask)`.

The IR now distinguishes defined-zero `Clz`/`Ctz` operations used by internal
transforms from `ClzNonZero`/`CtzNonZero`. Frontend builtins carry their language
precondition directly. Correlated value propagation also specializes an
internal defined operation when a dominating edge proves its source nonzero;
the false/zero edge deliberately retains the defined operation. Baseline x86
and i686 use BSR/BSF without zero repair on that domain, while targets whose
native instruction already defines zero share their existing lowering.

The x86 direct form consumes allocator source/destination registers rather than
staging through `%rax`. A tightly bounded peephole removes a conservative
allocator preload only when the following BSR/BSF has exactly the same source,
destination, and width. Unit refusals cover changed sources, destinations, and
widths. Execution sweeps every 32-bit power of two and a coprime permutation of
all 64-bit powers under default, SSE-only, and kernel no-SIMD configurations.
Constant folding refuses to assign a value to the nonzero opcode at zero.

The CE baseline result improves from 23 to 17 instructions: LCCC beats Clang
23.1 (19) and ICC 2021.10 (20), but remains one instruction behind GCC 16.2
and current ICX (16). At x86-64-v3 LCCC emits 16 versus GCC 14, ICX 16,
Clang 19, and ICC 20.
A guarded phi-diamond preinitialisation then removes the unconditional join
branch when both arms are one move, the fall-through label has no incoming
edge, and the alternate source cannot observe the destination. Refusal tests
cover targeted fall-through labels and destination-dependent addresses. The
remaining gap is signed-IV extension, not CTZ or CFG lowering. Reproducible evidence is under
`engineering/evidence/kernel/2026-09-20-bitcount-nonzero/`.

## Validation matrix

| Gate | Result |
|---|---:|
| Swap | PASS — 4 GiB active |
| LCCC build policy | PASS — fastbuild, Rust `-O1`, `-j2` |
| Boot setup-only build | PASS — `_end=31168`, 1600 B under 32 KiB; byte-identical to `ld.bfd` |
| Focused i686 addend differential | PASS |
| Focused ELF64 addend differential | PASS |
| If-conversion unit regression | PASS |
| Vectorizer/ISA execution + codegen gate | PASS |
| Real workqueue assembly | PASS — scale 4, no shift 16 |
| Clean full minimal CachyMod kernel rebuild | PASS — compiler hash forced clean |
| `bzImage` / `vmlinux` | 5,071,872 B / 20,577,712 B |
| Final setup `_end` | 29,856 B — 2,912 B headroom |
| QEMU/TCG SMP boot | PASS — all 16 gates, clean poweroff |
| `scripts/ci_local.sh --fast` | PASS — 46 passed, 0 failed, 3 intentionally skipped |
| CE oracle audit | PASS — GCC 16.2, Clang 23.1, ICC 2021.10, ICX latest |

Final clean-build bzImage SHA-256:
`9cda91cec2de172780ccc29d8287308c3ffc44a8fab77023bbb4b17b9d384417`.

## Compiler Explorer code quality

The original evidence under
`engineering/evidence/kernel/2026-09-19-ifconvert/` records the correctness
baseline: LCCC 23, GCC/ICX 16, Clang 19, ICC 20. The 2026-09-20 follow-up
removes the zero machinery and dead preload, reaching 17 instructions. This
beats Clang and ICC and narrows the best-oracle gap from seven to one without
changing the source or relying on target-specific undefined behavior beyond
the C builtin's actual nonzero contract.

## Remaining follow-up

1. Eliminate the final baseline instruction using a general range-backed IV
   transformation: widen the nonnegative bounded destination induction variable
   so array indexing needs no per-iteration `movslq`. Current counts are 17 vs
   best 16 at baseline and 16 vs best 14 at v3; the guarded phi-diamond
   preinitialisation has already removed the unconditional join branch.
2. Run the broad package desktop config from a clean tree on a host with enough
   persisted disk/time. The original SCSI corruption's proven writer is fixed,
   but the broad milestone should still be booted and stress-tested rather
   than inferred from the minimal boot.
3. Add CPU-hotplug stress beyond the two-vCPU boot suite and retain compact
   serial/QMP evidence.
4. Generalize the IR verifier's unique-definition check into an always-on
   debug-CI gate around mutating passes. `CCC_VALIDATE_SSA` localized this bug,
   but malformed SSA should fail immediately after the responsible pass.
5. Investigate the harmless ACPI HPET AML warning visible under QEMU separately;
   it is firmware-table behavior and did not affect any boot validation gate.
