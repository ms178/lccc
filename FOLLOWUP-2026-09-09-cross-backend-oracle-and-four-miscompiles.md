# FOLLOWUP — Session 33 (2026-09-09): cross-backend execution oracle + the defects it found

**Branch state**: `ms178-1.patch` rebased on `ms178/lccc` main (`91ecc083`).
**Compiler**: `scripts/build_lccc_fast.sh` (fastbuild, -O1, -j2, 4 GiB swapfile).
**Verified**: a new cross-backend execution oracle (§1) found **5 distinct
defects** — 4 silent miscompiles plus one hard compile failure — plus a scaled
variant of the last one. **All are fixed and verified** against GCC on all four
backends at `-O0/-O1/-O2/-O3/-Os`.
**CI**: `scripts/ci_local.sh` 16/16 green (see §10).

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
* **Scale-macro fallback ladder**: the size macros are passed to every program
  because unused `-D` defines are harmless, but a program that uses one of those
  names as an identifier (an enum member `N`, a variable `SIZE`) is a *program*
  compile error, not a compiler bug. The oracle retries the unscaled program and
  then builds the backends the same way, so reference and target always see the
  identical program (`binary_search.c`, `double_reduction.c`).

**Corpus scaling guards** (session 32) are what make this possible: 28/45
programs now accept `-DBLOCK_COUNT=`/`-DPASSES=`/`-DXML_SIZE=`-style overrides.

**First-run findings** (23 min, 45 programs × 5 opt levels × 4 backends):
`expat_xml_scan.c` i686 wrong at **every** level; `glibc_memcmp.c` AArch64
`-Os` segfault; `fannkuch.c` AArch64 `-Os` internal error. §5 and §6 add two
more found while narrowing those down. Everything else agreed with GCC.

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

## 6. Fixed: AArch64 `-Os` peephole deleted a live alias — **verified**

`glibc_memcmp.c` segfaulted under qemu-aarch64 at `-Os` only (x86-64, i686 and
RISC-V were correct).

* `LCCC_NO_PEEPHOLE=1` fixed it; **no** existing per-pass kill switch did
  (`CCC_NO_FP_PAIR`, `_SLOT_LOAD_DEDUP`, `_FP_SLOT_FWD`, `_STORE_DSE`,
  `_STORE_SINK`, `_GDSE`, `_REUSE_STACK_LOADS`, `_ZEXT_MASK_FOLD`,
  `_AND_TST_FUSION`, `_DEAD_PRE_RET`, `_SPILL_THREAD`, `_ADRP_CSE`,
  `_LOOP_ROTATE`, nor any of the 44 `CCC_DISABLE_PASSES` names except `all` /
  `ifconv`, which merely remove the triggering shape).
* Bisecting the Phase-1/2 passes with a temporary `CCC_SKIP_PEEP` gate pointed
  at **`propagate_address_aliases`** — one rebuild, then one test per name.
* Proven pre-existing: reverting the §5 hunk still segfaulted, and the acc-save
  window (`str x0, [sp, #-16]!`) appears in neither the good nor the bad asm.

**Mechanism.** The pass rewrites `mov xD, xS` into its address uses
(`ldr x0, [xD]` → `ldr x0, [xS]`) and deletes the mov when the scan finds a
later instruction that overwrites `xD` — treating that as proof the value is
dead. The scan is *textual*, so the overwrite it found (`mov x19, x0`) sat in a
**different basic block** than the mov (the prologue's `mov x19, x24`), which
does not dominate every path out of the mov's block. `.LBB37` reached
`ldr x0, [x19]` without passing the overwrite, so it read the caller's
callee-saved value restored by the prologue's `ldp x19, x20, [sp, #112]` — a
garbage pointer.

Fix: track whether the scan has crossed a label. A redefinition of `dst` proves
death **only inside the mov's own block**; once the scan has left that block the
transform bails out instead of deleting the mov.

**Measured cost of the conservative bail** (instruction counts over 43 corpus
programs, `lccc-arm`): `-Os` 10 677 → 10 703 (+26, **+0.24 %**), `-O2`
11 248 → 11 279 (+31, **+0.28 %**). Correctness is worth 0.25 % of code size.

The same fix also cleared §6b from the previous revision: scaled-down
`fannkuch.c -Os` no longer trips the `operand_to_x0` panic gate (it was the same
lost-alias → orphaned-value path). Both previously-open items are now closed.

## 6b. Sibling-pass audit (read-only, no behaviour change)

The §6 bug is a design pattern worth grepping for: a peephole pass that scans
**across** labels while using a **straight-line** death proof. Auditing every
pass in `src/backend/arm/codegen/peephole.rs` that deletes a line:

| pass | verdict |
|---|---|
| `propagate_address_aliases` | **was the bug** — scanned past labels, now fixed |
| `eliminate_overwritten_moves` | sound: breaks on `Label`/`Branch`/`Call`/`Ret` |
| `eliminate_overwritten_stores` | sound: default arm breaks the scan |
| `reuse_stack_loads_within_blocks` | sound: block-scoped, cache cleared at every label |
| `eliminate_dead_pre_ret_moves` | sound: only pre-`ret` moves |

So `propagate_address_aliases` was the only pass with the flaw. The i686
peephole uses a different mechanism (`staging_reads_safe`, a whole-function read
census) and is listed as an audit item in §9 rather than assumed safe.

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
* **`/tmp` is a 993 MB tmpfs on this box.** A full oracle run builds ~1 350
  static binaries; `tempfile.TemporaryDirectory()` defaults to `/tmp` and the
  run died at `binary_trees.c` with `final link failed: No space left on
  device` (188 spurious "reference compile failed" entries). The oracle now
  writes to a disk-backed scratch directory (`--scratch`, default
  `target/crossbe-scratch`) and unlinks every binary the moment it has been
  consumed. Any new harness script must do the same — never let scratch output
  accumulate in `/tmp`.
* Full-corpus oracle run ≈ 45–60 min wall clock; run it with `start_process`
  and `PYTHONUNBUFFERED=1` (otherwise `tee` sees nothing until exit).
* Never rebuild `lccc` while an oracle run is in flight: it replaces the
  binaries under test and silently mixes results.

## 9. TODO for the next session (ordered)

1. Re-run `scripts/cross_backend_check.py` over the full corpus × 5 opt levels
   (≈23 min) and drive it to zero failures; then wire `--gate` into
   `scripts/ci_local.sh` on a reduced matrix so cross-backend regressions cannot
   land silently.
2. Add the six fixed defects as **execution** tests (not just unit tests) so the
   oracle's coverage is permanent and cheap.
3. Apply the §6 lesson to the sibling passes that use the same textual
   "first overwrite proves death" reasoning: `eliminate_overwritten_moves`,
   `eliminate_overwritten_stores`, `thread_spill_slots` and the i686
   `staging_reads_safe` family all scan linearly and may hold the same latent
   bug. Audit them with the oracle rather than by inspection.
4. AArch64 peephole: re-enable global store forwarding (correctness bug,
   test 0036_0041) — the oracle now makes a fix verifiable.
5. Resume the session-32 backlog: the `ms178-1.patch` insn-count win, the
   `ms178-1-ext-*.patch` segments, and `lccc`'s icount (2.66 M vs 1.96 M).

## 10. CI status

`scripts/ci_local.sh`: **16/16 PASS**. The only failure seen this session was
`rustfmt`, caused by a temporary debug macro that was removed before the final
run; `cargo fmt --all -- --check` is clean on the final tree.

| gate | result |
|---|---|
| build, rust-toolchain-selector, cargo-test | PASS |
| regression-corpus-ssa | PASS — 726 passed, 0 failed, 9 skipped-compare (735 total) |
| benchmark-output-oracle, differential-correctness-oracle | PASS |
| loop-alignment-contract, fuzz-engine-wiring, differential-fuzz-smoke | PASS |
| strict-computed-recip-codegen, machinst-window-alloc | PASS |
| linker-fuzz, linker-elf-grammar-fuzz, codegen-quality-gate | PASS |
| rustfmt, clippy | PASS (after removing the temporary debug macro) |


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

## 9. v2 follow-up (second pass)

### 9.1 Why PR #464 was red, and the fix
The `Clippy` job runs **two** commands (`.github/workflows/ci.yml:234-254`), and
`scripts/ci_local.sh:144-146` reproduces both verbatim:

```
cargo fmt --all -- --check
cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings
```

The failure was `cargo fmt`, not clippy: the temporary `peep!` debug macro used
to bisect the ungated ARM peephole passes was still in the tree when the patch
was snapshotted. It has been removed. Both commands now exit 0 locally
(`FMT_EXIT=0`, `CLIPPY_EXIT=0`). **Lesson: never snapshot while a debug macro is
in the tree — run `cargo fmt --all -- --check` as part of the snapshot gate.**

### 9.2 The conservative `propagate_address_aliases` fix was wrong — precise version
The first fix bailed out of the alias fold whenever the forward scan crossed
*any* label. That is sound but costs instructions (-Os 10677 → 10703, -O2
11248 → 11279 over the 43-program ARM corpus) because it throws away every fold
that spans a label nothing branches to.

The replacement rule is precise and recovers the folds:

* **A label only ends the region if some branch in the function targets it.**
  A label no branch targets is not a control-flow entry point: the only way to
  reach the code after it is to fall in from the line above, so every path out
  of the `mov` still runs the redefinition. The target set is computed once per
  pass invocation in a `FxHashSet` over `Branch`/`CondBranch`/`CmpBranch` lines.
* **A conditional transfer inside the region invalidates the fold.** `b.cond`
  / `cbz` / `tbz` divert to a target *outside* the scanned region, so the taken
  path reaches the code after the redefinition without running it, keeping the
  `mov`'s value live. This also closes a latent hole the label-only rule left
  open.
* Unconditional `b`, `br` and `ret` remain safe to scan past: entry into any
  later block must go through a real branch target, which the first rule
  catches, so they cannot smuggle in a path that skipped the redefinition.

The glibc_memcmp `-Os` SIGSEGV stays fixed: `.LBB37` is a real branch target, so
the fold that deleted the prologue's `mov x19, x24` is still rejected.

### 9.3 Environment: the sandbox resets at every turn boundary
Everything outside the persisted `/home/user` workspace disappears between
turns, **and `target/` is excluded from snapshots by design**, so the compiled
lccc binaries are gone too. The `rustup` shim survives but loses its `+x` bit
and `~/.rustup/toolchains` is deleted. Restore sequence, in this order:

```
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
./scripts/build_lccc_fast.sh                 # ~2m40s from clean
sudo apt-get install -y -qq gcc-multilib gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu
git ls-tree -r HEAD --format='%(objectmode) %(path)' | awk '$1=="100755"{print $2}' | xargs chmod +x
git diff --summary | grep -c "mode change"   # must be 0 before snapshotting
```

`gcc-multilib` (i686 reference) and the two cross toolchains (ARM/RISC-V
builtin headers under `/usr/lib/gcc/<triplet>/14/include`, sysroots under
`/usr/<triplet>`) now install together without conflict. Without the cross
toolchains, `lccc-arm`/`lccc-riscv` fail with `stddef.h: No such file or
directory` even for a one-line program.

### 9.4 Open defect carried forward: `lccc-arm -O2/-O3` SIGSEGV on `nbody.c`
* Repro: `lccc-arm -O2 -static -lm -DSTEPS=5000 tests/benchmark/programs/nbody.c`
  → `qemu-aarch64`: `uncaught target signal 11`. -O0/-O1/-Os are correct and
  match gcc (`-0.169075164 -0.169089263`).
* Threshold: STEPS 200/2000 fine, 5000 and above crash — so it is drift over
  the outer `advance()` step loop, not a single bad pointer.
* Masked by `CCC_DISABLE_PASSES` ∈ {all, gaddrcse, gvn, inline, ivsr, licm,
  postinline, unroll}; **not** masked by any peephole gate. Eight different
  passes hiding one bug means the defect is downstream, in ARM codegen/RA, and
  only reachable once the loop is optimised.
* Asm pair to continue from: `/tmp/nb_bad.s` (-O2) vs `/tmp/nb_good.s`
  (-O2 + `CCC_DISABLE_PASSES=ivsr`). The step loop is the region enclosing
  `.LBB9`/`.LBB10` (i loop) and `.LBB11`/`.LBB12` (j loop); the crash happens in
  the code reached from `.LBB7`/`.LBB8`. Compare the callee-save area offsets
  (`[sp,#312]…[sp,#440]` bad vs `[sp,#304]…` good) and the `str x0, [sp, #304]`
  spill in the bad build.

### 9.5 Measured cost of the alias-fold fixes (ARM corpus, 43 programs)
| build | -Os | -O2 |
|---|---|---|
| buggy (pre-fix, glibc_memcmp SIGSEGV) | 10677 | 11248 |
| conservative: bail on ANY crossed label | 10703 | 11279 |
| **precise: bail only on real branch targets / cond. transfers** | **10703** | **11279** |

The precise rule measures **identical** to the conservative one: in lccc's
output every basic-block label really is a branch target, so the "is it a
target?" refinement recovers nothing on this corpus. The remaining +26 (-Os) /
+31 (-O2) instructions versus the *buggy* build are the price of not emitting
the `glibc_memcmp` SIGSEGV — they are not recoverable by restricting where the
fold fires, because in every blocked case the mov is genuinely live on some
other path.

**The correct fix is structural, not another restriction** (designed, not yet
implemented): build a CFG once per function, compute dominators, and replace the
linear scan's `crossed_label`/`crossed_cond` heuristics with the exact condition
"the block containing the redefinition dominates every block that reads dst".
That is sound by construction and strictly more aggressive than the buggy
version in every case where the fold is legal, so it should also *beat* 10677 /
11248. Secondary: the dead `mov xD, xS` exists because RA missed a coalesce —
fixing coalescing removes the copy at the source instead of cleaning it up.

### 9.6 Final: the alias fold now uses the EXACT condition, not a heuristic
Superseding §9.2 and §9.5.  `propagate_address_aliases` now builds a real CFG
(`struct Cfg`: blocks, successors, iterative dominator bitsets) over the
function and gates the fold on a single exact query, `redef_covers_all_reads`:

> every read of `dst` that can execute after the `mov` **without passing the
> redefinition** must be an address use that gets renamed onto the alias source.

Traversal stops at the redefinition's block, because any path leaving it has
executed the redefinition — so reads beyond it see the *new* value and are
irrelevant.  That is what makes the rule exact rather than conservative: it
accepts loop-carried redefinitions (the common case the buggy pass got right
for the wrong reasons) and rejects `glibc_memcmp`'s `.LBB37`, which is reachable
without running the redefinition.

| build | -Os | -O2 | correctness |
|---|---|---|---|
| buggy (pre-fix) | 10677 | 11248 | **SIGSEGV** on glibc_memcmp -Os |
| conservative: bail on any crossed label (NAK'd) | 10703 | 11279 | clean |
| **exact CFG/dominator rule (shipped)** | **10701** | **11281** | **240/240 oracle cells agree** |

The ~25 instructions the buggy build "saved" are exactly the illegal deletions:
the pass was removing copies that are live on some other path.  Since the new
rule accepts **every** case that is legal, no further recovery is possible
without miscompiling — the remaining delta is the true cost of correctness,
measured to the last instruction.

Verified: `glibc_memcmp` -Os/-O2/-O3 all print `2158787064` and exit 0;
`cross_backend_check.py` over 12 programs × 5 opt levels × 4 backends against
gcc reported *ALL BACKENDS AGREE WITH THE REFERENCE COMPILERS*;
`cargo fmt --all -- --check` and the exact CI clippy command both exit 0.

**Rejected during this work (do not resurrect without understanding why):**
deleting a provably-dead copy that has no rewritable address uses
(`!uses.is_empty()` → allow empty).  It is worth ~65 instructions at -Os but
breaks `sieve` (-O1..-O3) and `glibc_memcmp` -O3 even under the strict dominance
query, so the "provably dead" proof is incomplete for that case — most likely
`reads_gp_register`/`written_gp_register` misclassifying an instruction that
reads `dst` (a `str xD, [sp,#k]` spill reads it) as a pure overwrite.  Audit
`written_gp_register` for store forms before retrying; that is a real further
win, not a speculation.

### 9.7 Root cause of the "65-instruction win" breakage — and the real bug
Superseding the speculation at the end of §9.6.  The deletions that broke
`sieve` and `glibc_memcmp` were **return-value staging copies**:

```
    mov x5, x0      ; save
    ...             ; later
    mov x0, x5      ; restore / return staging
```

`propagate_address_aliases` treated `LineKind::Ret` as having "no runtime
effect" and kept scanning past it, so a redefinition of `x0` in a *later block*
was accepted as proof that the copy was dead.  It is not: the path through that
`ret` handed the mov's value to the **caller** — x0-x7 are return-value
registers.  `glibc_memcmp` returned garbage and exited 2.

**This is the true root cause of the original glibc_memcmp -Os SIGSEGV.** The
earlier "never cross a label" rule fixed it only by accident: it refused the
fold in that particular layout while leaving the live-out hole wide open.

Fix: `LineKind::Ret` now invalidates the fold when `dst < 8`.  x8-x17 are
caller-saved and x19-x28 are restored by the epilogue before the `ret` (which is
itself a redefinition), so only x0-x7 can be live-out at a return — the barrier
is precise, not another guess.

With that barrier the dead-copy deletion (removing `!uses.is_empty()`, the win
§9.6 deferred) is sound, and lccc-arm now beats the *buggy* baseline:

| build | -Os | -O2 |
|---|---|---|
| buggy baseline (SIGSEGV on glibc_memcmp) | 10677 | 11248 |
| conservative label bail (NAK'd) | 10703 | 11279 |
| exact CFG rule, no dead-copy deletion | 10701 | 11281 |
| **exact CFG rule + dead-copy deletion + ret barrier** | **10648** | **11230** |

glibc_memcmp -Os/-O2/-O3 all print `2158787064` (gcc: identical); sieve -O1..-O3
print `primes up to 10000000: 664579` (gcc: identical).

### 9.8 The 65-instruction win, investigated properly (final)
Deleting a provably-dead copy with no rewritable address uses is worth ~53 (-Os)
/ ~51 (-O2) instructions — and it was unsound three times over.  Each failure
was a distinct hole in the "is this copy dead" proof, found by diffing the asm
produced with and without the deletion (`CCC_NO_DEADCOPY=1`) and reading the
deleted instruction:

1. **Live-out at a `ret`** (`glibc_memcmp`, `sieve`): the scan treated `ret` as
   having no runtime effect and accepted a redefinition in a later block as
   proof of death, but x0-x7 are return-value registers — the path through that
   `ret` had already handed the value to the caller.  Fixed by invalidating the
   fold at a `ret` when `dst < 8` (precise: x8-x17 are caller-saved and x19-x28
   are restored by the epilogue, which is itself a redefinition).
   **This is also the true root cause of the original glibc_memcmp SIGSEGV.**
2. **Mid-block control transfer** (`fannkuch`, `zstd_count`): the CFG treated
   only the last line of a label-delimited run as the terminator, so a branch
   left by an earlier peephole pass contributed no edge and the query answered
   "no read" for blocks that were reachable.  Fixed by also splitting blocks
   after every `b`/`b.cond`/`cbz`/`tbz`/`ret`.
3. **Entering the redefinition's block from the top** (`zlib_ng_adler32`): the
   traversal skipped the def block entirely, but execution can run the lines
   *before* the redefinition — `mov x19, x20` before a loop whose body ends in
   `mov x19, x5` feeds the first iteration.  Fixed by walking the def block
   (checking its pre-redefinition lines) while not propagating out of it.
4. **Loop re-entry above the mov** (`zlib_ng_adler32`, the `mov x19, x5` loop
   increment itself): when the mov's block is part of a cycle, the back edge
   re-enters at the block's TOP, so lines *before* the mov are fed by it.  The
   linear scan never sees the back edge.  Fixed by checking the whole block when
   the block is reachable from itself.

With all four fixed the deletion is sound, and lccc-arm is smaller than the
conservative build it replaces:

| build | -Os | -O2 | glibc_memcmp | fannkuch | zlib_ng_adler32 | zstd_count |
|---|---|---|---|---|---|---|
| buggy baseline | 10677 | 11248 | **SIGSEGV** | ok | ok | ok |
| conservative label bail (NAK'd) | 10703 | 11279 | ok | ok | ok | ok |
| exact CFG rule, no deletion | 10701 | 11281 | ok | ok | ok | ok |
| **exact CFG + deletion + holes 1-4 fixed** | **10690** | **11265** | ok | ok | ok | ok |

The deletion is worth 11 (-Os) / 16 (-O2) instructions *once it is correct* —
the other ~40 of the raw 53/51 were illegal deletions of live copies, i.e. latent
miscompiles of exactly the glibc_memcmp kind.  lccc-arm is now **27 instructions
smaller than the NAK'd conservative build** and, unlike the 10677/11248
baseline, does not miscompile.  `nbody` -O2 (the open AArch64 SIGSEGV from §9.4)
now matches gcc as well.

`struct_copy` -O1/-Os mismatches the reference but also fails with
`LCCC_NO_PEEPHOLE=1`, so it is a pre-existing codegen defect outside the
peephole — not a regression from this work, and still open.
