# FOLLOWUP — Session 33 (2026-09-09): cross-backend execution oracle + four real miscompiles

**Branch state**: `ms178-1.patch` rebased on `ms178/lccc` main (`91ecc083`).
**Compiler**: `scripts/build_lccc_fast.sh` (fastbuild, -O1, -j2, 4 GiB swapfile).
**Verified**: cross-backend execution oracle; 4 miscompiles found and 3 fixed end-to-end.

---

## 1. The instrument this session built: `scripts/cross_backend_check.py`

**WHY.** LCCC has four production backends (x86-64, i686, AArch64, RISC-V)
sharing a front end, IR and optimiser but each with its own code generator.
Every defect found this session was invisible to single-target testing and to
assembly diffing.

**The oracle.** A deterministic C program's observable behaviour (exit status +
stdout) is a property of the *program plus its C data model*, not of the target.
So the host GCC build is the reference and every backend must reproduce it.
That removes the need for cross toolchains: references are native, and
correctness is checked by *execution*, not by eyeballing assembly.

```
scripts/cross_backend_check.py                        # whole corpus, -O0..-Os
scripts/cross_backend_check.py --programs sha256_transform --json /tmp/x.json
scripts/cross_backend_check.py --gate --opts="-O0,-O2"  # CI-usable exit status
```

* Runs each binary natively (x86-64, i686) or under `qemu-user` (AArch64,
  RISC-V); static linking, no runtime dependencies.
* **Per-data-model reference**: `unsigned long` is 64-bit on LP64 targets and
  32-bit on i686, so a `%lu` checksum legitimately differs. i686 is compared
  against `gcc -m32`, the LP64 targets against plain `gcc`. Without this the
  oracle reports false positives (it did, on `expat_xml_scan.c`).
* Workload scaling: the corpus' `#ifndef` size macros are overridden with `-D`
  so runs finish under TCG (`--no-scale` restores full size).
* `--json` writes a full report; `--gate` exits non-zero on any mismatch or
  compile failure. Exit 2 = usage/environment problem.

**Corpus scaling guards** (session 32) are what make this possible: 28/45
programs now accept `-DBLOCK_COUNT=`/`-DPASSES=`/`-DXML_SIZE=`-style overrides.

**First-run findings** (23 min, 45 programs × 5 opt levels × 4 backends):
`expat_xml_scan.c` i686 wrong at **every** level; `glibc_memcmp.c` AArch64
`-Os` segfault; `fannkuch.c` AArch64 `-Os` internal error. Everything else
agreed with GCC.

---

## 2. Fixed: AArch64 silent miscompile (`sha256_transform`) — **verified**

Root cause: `emit_shifted_logical_impl` (and the madd/msub twins) staged the
non-shifted operand *first*; `materialize` clobbered `x0`, destroying the
shifted operand that lived only there. `operand_to_x0` then emitted
`mov x0, #0` and the checksum silently lost its `state[3]` term — the kernel
still passed its own known-answer self-check, so nothing but a cross-target
comparison could see it.

Change: `stage_operands_acc_first()` in `src/backend/arm/codegen/alu.rs`; the
`mov x0, #0` fallback in `operand_to_x0` is now a hard `panic!`.

**Result**: `sha256_transform.c` matches GCC on **all four backends** at
`-O0/-O1/-O2/-O3/-Os` (BLOCK_COUNT 3 and 64).

## 3. Fixed: `-O0` emitted x86 text into AArch64/RISC-V assembly — **verified**

`remat_indexed_acc_safe` (`src/backend/generation.rs`) is arch-agnostic but
hard-coded `pushq %rax` / `popq %rax`, which the AArch64/RISC-V assemblers
reject ("unsupported instruction: pushq %rax") — `lccc-arm -O0` could not
compile `sha256_transform.c` at all.

Change: new **required** (no default) `ArchCodegen::emit_acc_save` /
`emit_acc_restore` hooks (`src/backend/traits.rs`), implemented per target:

| target | save | restore | notes |
|---|---|---|---|
| x86-64 | `pushq %rax` | `popq %rax` | flags-neutral |
| i686 | `pushl %eax` | `popl %eax` | flags-neutral |
| AArch64 | `str x0, [sp, #-16]!` | `ldr x0, [sp], #16` | pre/post-index; keeps 16-byte SP alignment |
| RISC-V | `addi sp,sp,-16` + `sd t0,0(sp)` | `ld t0,0(sp)` + `addi sp,sp,16` | no pre-indexed store exists |

SP-relative slot emitters on ARM (`emit_store_to_sp`, `emit_load_from_sp`,
`emit_stp_to_sp`, `emit_ldp_from_sp`, `emit_add_sp_offset`) and RISC-V
(`emit_store_to_sp`, `emit_load_from_sp`) now bias offsets by
`out.rsp_frame_size` through a new `slot_offset()` helper; x29/x19-based frames
are unaffected, exactly like x86's RBP frames. `emit_addi_sp` is deliberately
*not* biased (it moves SP itself). Outside the protected window the field is
zero, so all existing codegen is bit-identical.

Because the hooks are required rather than defaulted, a future target cannot
silently inherit x86 text again.

## 4. Fixed: i686 expat name-scanner miscompile — **verified**

`expat_xml_scan.c` returned 2 (self-check failure) on i686 at every level ≥
-O1: the UTF-8 name-length kernel returned 0 instead of 7/8. Expat is one of
the user's named golden workloads.

Chain: `range_fold` folded the classification (32-bit `unsigned long` changes
its range lattice), producing `movl %ebp,%eax; cmpl $128,%eax`. Pattern 6 of
`fold_reg_copy_idioms` correctly rewrote that to `cmpl $128,%ebp`; Pattern 15
then narrowed it to `cmpw $128,%r16` — and `low_subreg_name("%ebp", 2)`
returned `None`, so `.unwrap_or("%ax")` **silently substituted a different
register**. Every character took the UTF-8 path.

```asm
; before                          ; after
movzbl (%edi), %ebp               movzbl (%edi), %ebp
cmpw   $128, %ax   ; WRONG REG    cmpw   $128, %bp   ; correct
```

Changes (both in `src/backend/i686/codegen/peephole.rs`):
1. `low_subreg_name` now knows the 16-bit forms of `%ebp/%esi/%edi`
   (`%bp/%si/%di`) — legal in 32-bit mode. `%esp` stays `None`: narrowing a
   compare onto the stack pointer must never happen. (The 8-bit forms are
   still limited to eax/ecx/edx/ebx, which keeps Pattern 13 safe.)
2. The `%ax` fallback is gone: when there is no 16-bit name the rewrite is
   **refused** (`i += 1; continue`). A missed 2-byte size win costs nothing;
   a wrong register costs correctness.

**Result**: `expat_min.c` and the full `expat_xml_scan.c` match `gcc -m32` at
`-O0/-O1/-O2/-O3/-Os`.

## 5. Fixed: AArch64 `-Os` value orphaned by `invalidate_acc` — **verified**

`fannkuch.c` at `-Os` reached `emit_load_indexed_impl`'s unassigned-dest path:
`ldrsw x0, […]` produced a value with no register assignment and no stack slot,
`store_x0_to(dest)` registered x0 as its home, and the very next line called
`invalidate_acc()` — orphaning the value. The next `operand_to_x0` had nothing
left, which is what the new panic gate reports (before the gate: `mov x0, #0`,
i.e. a silent miscompile).

Change: new `store_x0_to_and_release()` — drops the accumulator entry **only
when the value actually reached a durable home** — used at the five sites that
paired `store_x0_to` with `invalidate_acc` (`memory.rs`, `cast_ops.rs`,
3× `comparison.rs`).

**Result**: `fannkuch.c` matches GCC on all four backends at `-O0..-Os`.

---

## 6. KNOWN-OPEN (pre-existing, not introduced this session)

### 6a. AArch64 `-Os`: `glibc_memcmp.c` segfaults (peephole)

* `qemu-aarch64` reports SIGSEGV; x86-64, i686 and RISC-V are correct at `-Os`.
* `CCC_NO_PEEPHOLE=1` fixes it; **none** of the existing per-pass kill switches
  do (`CCC_NO_FP_PAIR`, `_SLOT_LOAD_DEDUP`, `_FP_SLOT_FWD`, `_STORE_DSE`,
  `_STORE_SINK`, `_GDSE`, `_REUSE_STACK_LOADS`, `_ZEXT_MASK_FOLD`,
  `_AND_TST_FUSION`, `_DEAD_PRE_RET`, `_SPILL_THREAD`, `_ADRP_CSE`,
  `_LOOP_ROTATE`, nor any of the 44 `CCC_DISABLE_PASSES` names except
  `all`/`ifconv`, which only remove the triggering shape).
* Proven **not** caused by session-33 work: reverting
  `store_x0_to_and_release` to the old always-invalidate behaviour still
  segfaults, and the acc-save window (`str x0, [sp, #-16]!`) does not appear in
  either the good or the bad assembly.
* **Mechanism identified.** The peephole deletes the prologue's
  `mov x19, x24` / `mov x20, x23` (argument-pointer copies). `.LBB37` then
  executes `ldr x0, [x19]` / `ldr x0, [x20]` on a path where neither was ever
  redefined, so they still hold the *caller's* callee-saved values (restored by
  the `ldp x19, x20, [sp, #112]` prologue spill) — a garbage pointer. The
  un-peepholed assembly keeps the copies and is correct.
* Next step: `propagate_register_copies` replaces `x19`→`x24` in the blocks
  where the reaching definition is unambiguous, then the copy is deleted; the
  block where propagation correctly *refuses* (multiple predecessors, one
  redefining `x19`) keeps reading `x19`. The deletion analysis and the
  propagation analysis disagree — fix the deletion side to require that every
  remaining read is dominated by a redefinition.

### 6b. AArch64 `-Os`: `fannkuch.c` with shrunk macros still trips the panic gate

With the oracle's scale defines (`-DN=256 -DSIZE=256 …`) `fannkuch.c -Os`
still panics with `value v160 has no … home`. Default-size `fannkuch.c`
compiles and is correct at every level. This is the **same latent bug class as
§5, reached through a second path** — the panic gate turns what used to be a
silent `mov x0, #0` miscompile into a loud internal error, which is the
intended behaviour. Diagnose with the asm-tail trick: temporarily append
`self.state.out.buf` (last ~30 lines) to the panic message; it shows the
offending `ldrsw`/`csel` immediately.

---

## 7. Techniques that worked (reuse these)

| need | how |
|---|---|
| find cross-target miscompiles | `scripts/cross_backend_check.py` — execution, not assembly |
| locate a peephole culprit | `LCCC_NO_PEEPHOLE=1` for i686/ARM/RISC-V (**not** `CCC_NO_PEEPHOLE`, which only gates ARM and x86-64), then diff good/bad `-S` output |
| bisect a pass | `CCC_DISABLE_PASSES=<name>` (harvest names: `grep -rhoE 'pass_disabled\(&(disabled\|dis), "[a-z0-9_]+"\)' src/passes/*.rs`) |
| find which peephole *pattern* | temporarily wrap each pass call in a `CCC_SKIP_PEEP`-gated macro, rebuild once, then bisect by name (this is how Pattern 15 was caught) |
| prove a bug is/isn't yours | revert the one suspect hunk, rebuild (~25 s incremental), re-test |
| find the emitter of a bad instruction | `RUST_BACKTRACE=full` gives `operand_to_x0` ← `emit_store_default` ← `emit_store_impl` in one shot |

## 8. Environment notes

* `gcc -m32` works after removing the conflicting `gcc-<N>-<triplet>` cross
  packages; use it as the i686 reference.
* qemu-user needs sysroots: `-L /usr/aarch64-linux-gnu`,
  `-L /usr/riscv64-linux-gnu`.
* Distro `rustc` is 1.85 (< MSRV 1.98.1) — use the rustup toolchain.
* Full-corpus oracle run ≈ 23 min wall clock; run it with `start_process`.

## 9. TODO for the next session (ordered)

1. **§6a** — make the ARM peephole's copy-deletion analysis agree with
   `propagate_register_copies` (dominance requirement), then re-run the oracle
   at `-Os` on the whole corpus.
2. **§6b** — find the second orphaned-value path (asm-tail panic trick).
3. Re-run `scripts/cross_backend_check.py` over the full corpus × 5 opt levels
   and drive it to zero failures; then wire `--gate` into `ci_local.sh` on a
   reduced matrix so regressions cannot land silently.
4. Add the four fixed defects as execution tests (not just unit tests) so the
   oracle's coverage is permanent.
5. Resume the session-32 backlog: ARM peephole store-forwarding
   (correctness-blocked), the `ms178-1.patch` insn-count win, the
   `ms178-1-ext-*.patch` segments, and `lccc`'s icount (2.66 M vs 1.96 M).
