# Follow-up — 2026-09-06: kernel-boot blockers C1/C5, memcpy staging, prologue ABI reads

Session base: upstream `main` 6af3436d (post-#430, Rust 2024). Deliverable
`ms178-1.patch` = commits S04 + S05 (see `artifacts/SNAPSHOT_LEDGER.md`).
All gates green locally: `cargo fmt --check`, `cargo clippy --all-targets -D
warnings`, `cargo test --all-targets` (2019/0), `run_regression_suite.sh`
646/0 (A/B diff 0), `run_regression.py` 678/0, inline-asm UTF-8, strict
recip, MachInst wide-copy checks, toolchain selector.

## 1. Accomplished

### S04 — memcpy/memmove parallel-copy staging (defect "mc2") + C1
* **Root cause.** `ArchCodegen::emit_memcpy` (traits.rs default) staged
  `dest`→`%rdi` then `src`→`%rsi`, each through `%rcx`. With parameters left
  in ABI registers (`x86_param_caller_homes_safe`), `f(struct S *src,
  struct S *dst)` has `src∈%rdi`, `dst∈%rsi`; the first move destroyed
  `src`, the copy degenerated to `(%rsi)→(%rsi)` (a no-op the peephole then
  folded further). Silent wrong data at every `-O1+`.
* **Fix.** `X86Codegen` overrides `emit_memcpy` →
  `memory.rs::emit_memcpy_ir_impl` → `stage_copy_operands`, a two-node
  parallel-move schedule over a `CopyAddrSource` classification (GPR home,
  XMM home, slot-address incl. over-aligned alloca and i128/F128/Vec direct
  slots, slot load, generic). Cross → `xchgq`; one-sided alias → aliased
  operand first; otherwise any order. `__builtin_memcpy`/`memmove` inline
  expansions share it (memmove previously used sequential
  `operand_to_reg`).
* **C1 (kernel #UD in `__update_freelist_fast`).** MachInst `Mov128`
  relayed 16-byte integer moves through `xmm0` regardless of `-mno-sse`.
  `set_no_sse` now clears `isel::SSE_INTEGER_MOVES`, the typed route falls
  back to the `rax:rdx` GPR pair. Verified: 0 xmm references for i128
  load/store/copy under `-mno-sse -O1/-O2`.
* zstd oracle scripts: `MODE=inplace|separate|both`, BMI2 masking (host has
  BMI2, qemu64 guest runs `_default` clones — oracles must mask), `IN_OFF`.
* Test: `tests/regression/memcpy_param_home_swap.c` (fails pre-fix).

### S05 — late ABI parameter reads + unified fixed-GPR scratch model
* **Defect (1) — at1/at2/at3 SIGSEGV.** A register parameter whose
  ParamRef is unhomed (`remove_ineligible_operands` strips atomic ptr
  operands, `CallIndirect` fptr, VaArg, non-ParamRef memcpy operands …) and
  has no alloca slot is read from its incoming ABI register *at the
  ParamRef site*. The prologue's parallel copy had already written another
  parameter into that register (`movq %rcx,%rdi`). **Fix:**
  `emit_store_params` materialises every such read at entry (exact
  `emit_param_ref_impl` sequence into the value's slot) and marks the
  parameter pre-stored; text and MachInst ParamRef paths both emit nothing
  then. A register-homed ParamRef *sharing* a home cannot be materialised
  early → prologue asserts its ABI register is not a pre-store target.
  `count_value_uses` now runs before `emit_store_params` (generation.rs)
  so `store_rax_to` keeps skipping dead destinations.
* **Defect (2) — wrong sums (at1 681≠672, at2 289≠301).** cmpxchg-loop RMWs
  (`emit_x86_atomic_op_loop`) clobber `%rdx`+`%rdi`; `cmpxchg` stages
  `desired` in `%rdx`; atomic stores stage in `%rdx`; `rdtsc` writes
  `%rdx`, `rdtscp` also `%rdi`. None were in the prologue census nor the
  caller-home gate; the two hand-maintained rdx lists had already drifted
  (C4: `Load{I128}`). **Fix:** `regalloc::X86FixedScratch`,
  `x86_inst_fixed_scratch(inst)`, `x86_body_fixed_scratch(func)` — single
  model consumed by the census (removes rdi/rdx from the caller-saved pool)
  and by `x86_param_caller_homes_safe_with_config`.
* Test: `tests/regression/param_prestore_abi_read.c` (8 shapes; fails
  pre-fix at every -O level with and without `-mno-sse`).

## 2. Deferred to the next session (ordered)
1. **Kernel boot re-run** with S04+S05: `scripts/prepare_kernel_tree.sh`,
   `JOBS=2 scripts/build_kernel_vm.sh`, xmm/ymm audit of the vmlinux
   (`objdump -d | grep -c xmm`), `scripts/qemu_boot_test.sh`. C2 (auto-
   vectorizer emits AVX2 regardless of `-mno-avx/-mno-sse/-march=x86-64`;
   `identify_cpu`, `chacha_block_generic`) is **still open** — ISA gating
   must follow the actual `-m` flags, not the documented x86-64-v3 project
   baseline (`simplify.rs set_has_fma3`).
2. **Per-point fixed-GPR hazard table** (finer than the whole-function
   census): seed `reg_occupancy` at the exact program points of fixed-GPR
   emitters (template: i686 `regalloc.rs` P2 hazard seeding +
   `overlaps_inclusive_skip_birth`), so rdx/rdi stay allocatable away from
   the hazard. The census fix is sound but pessimistic for functions with a
   single `rdtsc`/atomic.
3. Register `Memcpy`/`AtomicRmw`/`Cmpxchg` operand eligibility: with the
   parallel staging in place, `remove_ineligible_operands` no longer needs
   to strip memcpy dest/src at all (arm/riscv already sequence safely) —
   measure code size on the benchmark corpus before/after.
4. `divconst` I128 mulhi bloat (`/tmp/k2/d10.c` shape: mulq + slot traffic)
   — perf item; compare against GCC's `mulq`-only sequence.
5. `live_range.rs` `register_touches_only` doc comment triplicated.
6. `scripts/` consolidation (README covers ~26 of ~81), 11 pre-existing
   failing `check_*.sh`, LK-28 objtool, LK-30 missing system-instruction
   encodings.
7. Unit tests for `x86_inst_fixed_scratch` (table-driven over every
   `Instruction`/`IntrinsicOp` variant) and for `stage_copy_operands`
   (all 3×3 home permutations at the asm level).

## 3. Repro corpus (recreate under /tmp/k2; volatile)
`run.sh` compares lccc vs gcc across `-O0..-Os × {,-mno-sse}`; sources for
at1/at2/at3/tsc1/mc2 are embedded in the two regression tests above.
Debug: `CCC_DEBUG_RA_INTERVALS=1 CCC_DEBUG_LIVE_FUNC=f`, `LCCC_DUMP_IR=1`,
`CCC_DEBUG_PARAM_STORE=1`.

## 4. Harness-reset notes
`/home/user` survives without `.git`, `target/`, `/swapfile`, `/tmp`.
Recovery = `git clone artifacts/lccc.bundle` + re-add `origin` +
`scripts/arena_session_restore.sh` (swap, toolchain, fastbuild). Worktree
edits DID survive this time (S05 was re-applied from the surviving tree).
