# 2026-08-18 (session 7) — i686 accumulator-bypass + kernel boot-code gate

**Base (re-rebased):** `origin/main` @ `38d3b11a` (user landed three new
"Update copy_coalescing.rs" commits — 2e7e287f, 9aaa715f6, 38d3b11a)
**Snapshot:** `/home/user/ms178-1.patch` (tags `S01-i686-bypass`, `S02-boot-pipeline`, `S03-upstream-refit`)

## Rebase + rebuild turn (the user's "I landed a new change upstream" request)

Rebased the two session commits onto `38d3b11a` and re-validated. The new
upstream commits:
- did NOT compile (borrow-checker E0502 in `compute_immediately_consumed`'s
  `result.retain` closure) — fixed upstream in `38d3b11a` with a two-phase
  retain; my local equivalent fix was discarded.
- carried a failing unit test (`immediately_consumed_picks_adjacent_cast`) and
  two x86-64 MISCOMPILES that the existing `alu_peepholes` regression catches
  (335→334 passing before the fixes below; 336 after).

## Fixed this turn (each validated: 903 unit, 50/50 correctness, 336 regression, 18/18 i686)

1. **Return-operand homes** (`copy_coalescing.rs`): a value whose single use
   is `Return` must keep its home. The pre-existing `is_sole_operand_of_terminator`
   branch put it in the immediately-consumed skip set; the new upstream test
   pins the opposite. Now `is_sole_operand_of_terminator` returns false for
   `Return(_)`.
2. **MachInst buffer vs default-path fusion ordering** (`generation.rs`): the
   mul-add / shifted-logical / cmp-select fusions emitted via the default
   accumulator path WITHOUT flushing the MachInst buffer. A buffered `Xor`
   (`(h ^ v) * MAGIC + K`) was read before emission — `imulq` consumed stale
   `%r11`, the `xorq` landed after it (mix() wrong hash). Flush before each
   fused default emission.
3. **Immediately-consumed skip over-approximation** (`copy_coalescing.rs`):
   `is_safe_sole_consumer` accepted a value paired with a constant on EITHER
   side. Only the LHS is consumed from the accumulator on lhs_first_binop
   backends; `Sub(Const(0), v)` loads `v` into `%rcx`/memory from its home, so
   skipping the home read zero (`sdivm3` = `0-(v/3)` returned 0). Restrict to
   LHS-with-const-RHS. `unique_value_with_const` deleted; the two-sided form
   was unsound.
4. New `tests/regression/mix_machinst_fusion.c` (loop-carried mix-hash chain
   over 64-bit edge values; wrong pre-fix, exact post-fix).

## Environment protocol (unchanged, harness wipes everything)
**Mission:** continue the codegen quest; build the linux-cachymod-6.18.44 boot
code with lccc + lccc-ld and close the 32 KiB "Setup too big!" gate.

## Accomplished (all validated)

1. **i686 accumulator-bypass for Load / Store / Cmp** (`src/backend/i686/codegen/memory.rs`, `comparison.rs`).
   - Loads and stores now emit straight into/from an allocated GPR
     (`movzbl (%esi), %edi` instead of `movzbl (%esi), %eax; movl %eax, %edi`),
     and compares test register-resident operands in place
     (`testl %edi, %edi` instead of `movl %edi, %eax; testl %eax, %eax`).
   - Because `%eax` is never allocatable on i686 (reserved accumulator), none
     of these paths touch the accumulator, so the reg_cache stays valid.
   - Realmode corpus (20 arch/x86/boot C files, REALMODE_CFLAGS):
     **29517 → 29079 text bytes** (-1.5%). x86-64 untouched (parity: x86-64
     already had direct load/store).
2. **Fixed a pre-existing i686 miscompile** (correctness, the priority is
   *hard*): the memory-operand folds in `i686/codegen/peephole.rs`
   (`fold_memory_operands` and the compare-with-memory fold) treated
   `movzbl/movsbl/movzwl/movswl` as 4-byte loads because `parse_load_from_ebp`
   maps them to `MoveSize::L` (their DEST register is 32-bit). Result:
   `structs_bitfields`' union check `uu.b[0] != 0x04` compiled to
   `cmpl $4, 176(%esp)` — comparing the whole union word 0x01020304. Both fold
   sites now require a genuine `movl` (`is_full_width_load`). x86-64 is immune
   (its `strip_mov_prefix` does not classify movz* as foldable loads).
3. **Fixed a rustdoc doctest** in `x86/codegen/peephole/passes/memory_fold.rs`
   (assembly example was parsed as Rust → `cargo test` failed).
4. **Boot-code build pipeline end-to-end**: `scripts/build_kernel_boot.sh`
   compiles all 23 arch/x86/boot objects (C + .S, incl. header.S with the
   CONFIG_EFI_MIXED stub-offset assert) with lccc and links setup.elf with
   **lccc-ld** against `setup.ld`. lccc-ld's script engine evaluates the
   ASSERTs and reports the overflow correctly.
5. **Realmode measurement harness**: `scripts/realmode_corpus.sh` (lccc vs gcc
   text size per boot C file).
6. **Linker oracle update**: `tests/linker/setup_oracles.sh` now builds the
   pinned **binutils 2.47** (gas + bfd only) when the system bfd is not 2.47,
   honouring the "GAS/bfd 2.47" build/version preference (alongside mold/wild
   from git HEAD with `-DMOLD_TARGETS='X86_64;I386'`).

## Validation evidence

- `cargo test`: **897 passed / 0 failed** (incl. 2 new peephole unit tests).
- `tests/correctness/run_correctness.py`: **50/50**.
- `tests/regression/run_regression.sh --compare-gcc`: **335 passed / 6 failed**
  — the 6 are pre-existing GCC-oracle artifacts, byte-identical on the
  baseline (proven by stash+rerun): gcc-14 PIE-link fails
  (code16_realmode_encoding), gcc-14 lacks C23 `typeof_unqual`
  (kernel_flags_and_builtins), gcc-14 needs `-lm` (sqrt_vex_scalar), gcc
  attribute disagreement (has_attribute_in_code), x86-64 FP rounding delta
  (fp_domain_crossing), CPUID feature set (builtin_cpu_supports_raptor).
- i686 runtime (lccc-i686 -m32 vs gcc -m32, 19 tests): **18/18 pass** after the
  fix (structs_bitfields was the failing one; bitfields skips on gcc-14 -m32).
- Official build with `-D warnings`: clean.

## Kernel gate status (the 32 KiB blocker)

setup.elf object total (text+data+bss) measured with lccc on 2026-08-18:

```
text = 32478   data = 216   bss = 4915   TOTAL = 37609   limit = 32768
OVER BY 4841 bytes (14.8%)
```

lccc-ld link of setup.ld reports: `ASSERT failed: The setup must be at most 64
sectors in size` (== `_end <= 0x8000`). bss is dominated by main.o's 4108-byte
`boot_params`-adjacent statics (bss counts toward `_end`; it does not consume
file size). Text budget must come down to ~27.6 KiB.

## Remaining levers (ranked, with evidence)

1. **Register allocator: make %eax allocatable (or widen the caller-saved
   pool).** The single biggest structural handicap. `I686_CALLER_SAVED =
   [%ecx,%edx]`; `%eax` is a reserved accumulator, so leaf functions spill to
   slots and leaf params get callee-saved homes. `sub_char(a,b){return *a-*b;}`
   is 13 instructions vs GCC's 3; the 3 push/pop pairs + param capture are all
   consequences. This is a multi-session rework: the accumulator paths
   (operand_to_eax/store_eax_to, casts, calls) must be made tolerant of %eax
   being live. Start with a prototype behind a flag; measure the realmode
   corpus + tests/benchmark.
2. **Param capture to ABI/caller-saved homes.** Even without full %eax
   allocation, a param arriving in %edx should get home %edx (no capture move,
   no push/pop) and an %eax param should prefer %ecx/%edx over %ebx..%ebp.
   `emit_store_params_impl` two-phase capture already exists; the allocator
   just never prefers the ABI register. High value, moderate risk.
3. **Cast accumulator-bypass.** `movl %edi,%eax; movsbl %al,%eax` round-trips
   on every `(int)(char)` widening of a register-resident value (strcmp loads
   `movsbl (%ebx),%edi` then re-extends). Requires proving the canonical-form
   invariant ("an I8/U8/I16/U16 value in a GPR is sign/zero-extended") — audit
   constant emission (`IrConst::I8` for U8 constants is sign-extended, NOT
   zero-extended — see `operand_to_eax`), then `Cast{IntWiden,IntNarrow}` of a
   register-resident src becomes a plain `movl` copy or width-matched
   `movzbl %bl,%ebx` when the sub-register is encodable (byte regs only on
   %ebx/%ecx/%edx in 32-bit mode).
4. **Leaf-return %edx staging.** `subl %eax,%edx; movl %edx,%eax; ret` — bias
   the return value's home toward %eax-class, or teach the return path to
   recognize a single-use register-resident result and compute it in %eax.
5. **Redundant GEP recomputation / push-pop churn** (from the 2026-08-18
   kernel-codesize session notes; still open).

## Environment protocol (unchanged, harness wipes everything)

swap 8G first (`scripts/ensure_swap.sh`, default now 8G); rustup stable minimal (installed
under /opt so the ~15k toolchain files do not blow the 10k-file workspace
snapshot cap); `cargo build --profile fastbuild --locked -j2`;
apt: bison flex bc cpio libelf-dev libssl-dev kmod gcc-multilib
libc6-dev-i386 qemu-system-x86 busybox-static cmake ninja-build lld;
kernel = linux-6.18.44 + 26 patches from ms178/archpkgbuilds (PKGBUILD order;
`make olddefconfig` with CC=lccc because switching CC re-triggers syncconfig).
Boot corpus = `scripts/realmode_corpus.sh`; full boot build =
`scripts/build_kernel_boot.sh` (stub zoffset.h is size-neutral; header.o needs
`ZO_efi32_stub_entry == ZO_efi64_stub_entry - 0x200` under CONFIG_EFI_MIXED).
mkcapflags.sh is `mkcapflags.sh <OUT> <cpufeatures.h> <vmxfeatures.h>` — do NOT
reverse the args, it clobbers cpufeatures.h.

## Oracle status

`scripts/godbolt.py` verified working: `list`, `compile gcc16.2 …`, and
`compare … --oracles gcc16.2,clang,icc,icx` (GCC 16.2 = `cg162`, clang 22.1.0,
icc 2021.10.0, icx-latest via CE). Local bfd 2.47 built by
`tests/linker/setup_oracles.sh` (pin added this session).
