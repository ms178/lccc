# FOLLOWUP — 2026-09-07 (v5): position-aware %rdx admission, PF-07 completion, RISC-V immediate model, i686 operand-split canonicalization

Base ref: `ca1c3b349e3bbb20f95897e393b0a16a7b692934` — current `ms178/lccc` `main`
(merge of PR #440). Prior session's work landed as PR #437 (memory barriers +
allocation safety); PRs #436/#438/#439/#440 (vectorization ISA gating, ARX
acc-fold fix + aggregate splitting, loop-rotate Guard G, native rotates) are
upstream and were re-validated as this session's baseline.

## TL;DR

| # | change | headline effect |
|---|---|---|
| A | **Position-aware %rdx admission** (RA Phase 2-x64) | Recovers the register that one cold `a % b` used to cost a whole function: lz4's post-compression `UDiv` evicted %rdx from the entire match-search loop. 11+ values re-homed, frame −32 B. |
| B | **PF-07 post-RA demotion** | Promoted indexed-symbol bases the RA could not home no longer emit their dead `leaq sym(%rip),%rax; movq %rax,slot` pair — 2 instructions + 8 frame bytes per base deleted; consumers take the per-access remat they always took. |
| C | **RISC-V immediate model** for int_const_hoist | The pass never ran on RISC-V (driver gate, not the pass, silenced it — the same under-hoist AArch64 had before fix B). Constants outside imm12 now hoist: 4 multi-instruction `li` materializations removed per iteration of the motivating loop, imm12 (single-`addi` `li`) correctly stays. |
| D | **i686 operand-split canonicalization** | The last four `rfind(',')` sites + two hand-rolled depth walkers now use the shared `last_top_level_comma` / new `split_top_level_commas`. Fixes two latent i686 misparses (SIB index read as destination; SIB index reads hidden from the source check). Proven behavior-preserving: 261 files × 5 opt levels = **1305 compilations, byte-identical asm**. |

Plus: the red-team audit's soundness fixes on the new work (phi-coalesce %rdx
propagation guard, value-based wide-op detection, divrem-pair tail exclusion,
four unclassified %rdx writers added to `x86_inst_fixed_scratch`, two
defense-in-depth guards), and the v4 script-consolidation backlog (P1/P3/T2/R2,
perf_ab preset dedup) executed with real-run validation.

## A. Position-aware %rdx admission (RA Phase 2-x64)

**The defect.** The x86-64 fixed-scratch model
(`x86_inst_fixed_scratch`/`x86_body_fixed_scratch`) drops %rdx (PhysReg 16)
from the caller-saved pool for the WHOLE function whenever ANY instruction
clobbers it — division (rdx:rax), i128 pairs, switch jump tables, cmpxchg /
atomic stores, fixed-scratch intrinsics. The clobbers are position-local, so
one cold division anywhere costs the register supply of every hot loop: lz4's
`main` (fill_source + compress_block + PASSES loop all inlined) has exactly
one `UDiv` — the post-compression total-size math in the cold tail (IR block
60) — and paid for it with %rdx excluded while the match-search loop spilled
long-lived pointers.

**The fix.** A dedicated hazard-filtered wave in `allocate_registers`
(`regalloc.rs`, "Phase 2-x64"), mirroring the i686 ecx/edx hazard waves:

* Runs AFTER the general Phase-2 waves and BEFORE Phase 2c. %rdx homes are
  refused by `const_offset_fold_reg_base_ok` (PhysReg 10|16 exclusion), so
  %rdx must be an overflow net, never a first pick — running the wave first
  measurably stole fold-eligible homes (lz4 −3%, since reverted by the
  ordering). Before 2c because 2c's callee-saved overflow costs a push/pop
  pair while a caller-saved %rdx home is free.
* Candidates: the Phase-2 `base_ok` gates (inlined — no home yet, not
  call-spanning, param/riscv guards) + `later_arg_values` exclusion (the
  call-argument staging contract) + `!overlaps_inclusive_skip_birth(iv,
  clobber_points)`: a value live across a clobber point is refused (inclusive
  of the point where the clobbering instruction still READS it — a divisor
  staged before `cqo` zeroes %edx), while a value BORN at the point (the
  div/rem result) is admitted — the optimal shape, the remainder is produced
  in %rdx.
* Clobber points: `collect_x64_rdx_clobber_points` derives them FROM
  `x86_inst_fixed_scratch` (plus Switch terminators) in the flat liveness
  numbering — the allocator's %rdx view can never drift from the pool gate's
  view (the i686 divrem-pair "derive from the same IR" discipline).
* Completeness: clobber-free bodies already admit %rdx today, so every
  non-clobber-class emitter is production-exercised with %rdx-homed values
  (`emit_save_acc_impl` switches to %r11 dynamically, register-direct paths
  are home-generic, the divrem pair fusion carries a %rdx-home screening).
* Wide (I128/U128) bodies keep the whole-function exclusion — VALUE-based
  detection (`x86_body_has_wide_ops`, fixpoint over typed defs + Copy/Phi
  chains), because an i128 `Copy` (phi-elimination materializes exactly those)
  writes the %rax:%rdx pair while carrying no type field.
* Kill switch: `CCC_NO_RDX_HAZARD` restores the whole-function exclusion
  for A/B (wired into the parser-coverage switch table).

**Audit-hardened after the red team** (each was a real hole in the first
cut, found before shipping):

1. **Phi-coalesce propagation** (`apply_phi_coalesce_assignments_with_config`)
   copies a phi dest's home to its backedge source with no knowledge of the
   hazard points — a rdx-homed dest whose source's interval crosses a clobber
   would have handed `cqo` a live victim. Guard added (lazy clobber-point
   computation, debug output, no cost for clean bodies).
2. **Divrem-pair tails** emit nothing at their own IR point (the head stored
   both results), so a tail dest's modeled interval starts at the tail while
   the value is physically in %rdx from the head. Interleaved pairs
   (`a/b; c/d; a%b; c%d`) could corrupt it invisibly. Tails are now excluded
   from the wave (same exclusion the accumulator analysis applies).
3. **Four unclassified %rdx writers** added to `x86_inst_fixed_scratch`:
   `RestoreApplyResult` / `DoBuiltinApply` / `BuiltinLongjmp` (the GCC
   __builtin_apply family reads/writes the raw %rdx; DoBuiltinApply also
   performs a real `call *%r11` the IR does not model as a call point — a
   pre-existing latent bug class for caller-saved homes, now at least not
   reachable with %rdx), and F128 stores (the non-direct-slot arms of
   `emit_f128_store_f64_via_x87` stage through `movq %rax,%rdx`).
4. Defense-in-depth: MachInst isel div lowering refuses a PhysReg(RDX)
   divisor (stages through rcx like an immediate) so no future
   home-propagation path can resurrect the cqto-destroys-divisor miscompile;
   the dynamic arg-hazard `written_gpr` marking now covers
   `I128RegPair`/`StructByValReg` (both registers of the pair).

**Measured.** lz4: 11+ values rdx-homed, frame 168→136 B, 5 instructions
removed total, match-search and init loops instruction-identical. The A/B
harness (lccc-ld link) shows a paired lz4 delta of −3..−5% — **proven to be a
code-alignment artifact**: the 5 removed prologue instructions shift the hot
loop by exactly 36 bytes and re-roll the loop-fetch lottery. With both
binaries assembled/linked by the same toolchain and the loop address restored
to the byte (36 nop bytes of padding), the fix binary is **+2.6% faster**
(paired mean CI [1.0154, 1.0395], 20 rounds). No other benchmark regressed
(all paired CIs straddle 1; arith_loop/chacha20/sha256/histogram/hash_table
unchanged — they are clobber-free and keep the historical pool). GCC-verified
shapes preserved; the codegen identity gate stays within tolerance for every
golden workload.

**Follow-up surfaced (not attempted here):** lccc has no hot-loop alignment
pass. GCC/Clang emit `.p2align` before hot loops precisely to stop the
alignment lottery this session had to control for by hand — on the lz4 loop
the lottery is worth ±3–5%, which is larger than most of the wins we chase.
A loop-alignment pass (align loop headers to 16/32 B when the loop is hot
enough) is the single highest-leverage remaining micro-optimization.

## B. PF-07 post-RA demotion (dead-materialisation cleanup)

PF-07 (landed with the rotates series) promotes PIC indexed-symbol bases out
of the never-materialized set so the RA can home them and SIB accesses read
the home — `gzip_crc32_update` demonstrably gets the GCC shape. When the RA
does NOT home a promoted base (lz4's three bases: 42 Phase-1 candidates for 6
callee-saved registers), the promotion is pure dead weight: the GlobalAddr
emission writes `leaq sym(%rip),%rax; movq %rax,slot` once, and no consumer
ever reads the slot (indexed folds consult register homes only, so every
consumer takes the per-access `leaq sym(%rip),%rcx` symbol arm regardless).

The demotion (x86 prologue, right after the RA returns, before the MachInst
busy map / cmp-replay prunes / stack layout): a promoted base with no
register home goes back into `never_materialized_values` and out of
`promoted_global_addr_homes`, restoring exactly the pre-promotion code shape
for that value (every pre-promotion set member is foldable/rematable by
construction). Bases that DID win a register home keep the hoisted shape —
the entire point of PF-07. lz4: the block-0 dead pair and its never-read slot
deleted (−8 frame bytes on top of the wave's savings).

## C. RISC-V immediate model for int_const_hoist

The pass's thread-local `AARCH64: Cell<bool>` became an `ImmModel` enum
(X86_64 / Aarch64 / Riscv). The RISC-V arm: the ALU emitters are
register-form-only, so every integer constant pays a `li` materialization per
iteration; `li` is one `addi` inside imm12 ([-2048, 2047] — note 4095 is OUT:
the signed 12-bit immediate tops at 2047) and `lui`+`addi` outside it. The
free range is therefore imm12 at every operand position (uniform — the
`needs_reg`/`imm32_exact` axes are x86/AArch64 distinctions), and `Mul` is
NOT a needs_reg position on RISC-V (no immediate forms exist anywhere).

The driver gate (passes/mod.rs) previously whitelisted only
Aarch64|X86_64 — RISC-V under-hoisted exactly the way AArch64 did before fix
B, because the gate, not the pass, was what silenced it there.

**Measured on the motivating shape** (`mult_acc` with a 1000003 multiplier
and the xxHash gold ratio inside a loop): 4 multi-instruction `li`
materializations per iteration removed (constants hoisted to preheader
registers `a2`/`a3`); `li t0, 29`-class imm12 constants correctly stay in
place. No RISC-V execution capability in this sandbox (no qemu) — validated
by unit tests (5 new, model-exact), the IR-rewrite's existing x86-64/AArch64
coverage (the rewrite is target-independent), and asm inspection.

X86_64 and AArch64 behavior preserved: **AArch64 256/256 corpus files
byte-identical**, x86-64 unchanged (the x86-64 arm is line-for-line the old
boolean logic under the new enum).

## D. i686 operand-split canonicalization

The four remaining `rfind(',')` sites (parse_dest_reg 469, combined_local_pass
1471/1603, line_reads_dest_source 2730) and two hand-rolled depth walkers
(replace_att_operand_reg / replace_att_reg_with_text) now use the shared
`last_top_level_comma` and the new `split_top_level_commas` (peephole_common,
exact transcription of the historical walker semantics: chars() iteration,
plain i32 depth, whitespace retention).

Two of the four sites were latently wrong, not just unauditable:
`parse_dest_reg` on `movl %ebx,(%esi,%eax)` returned the SIB **index** as the
destination, and `line_reads_dest_source` missed SIB index reads on
`movl %ebx, (%esi,%eax)` (the exact x86-64 `line_writes_memory` defect class,
closed there in the rotates series).

**Proof of behavior preservation** (the conversions are conservative at the
guarded sites, but proof beats argument): every i686-compilable file in the
regression + benchmark corpora (261; the rest need 32-bit libc headers that
this host lacks — identical failures on both binaries), × {-O0, -O1, -O2,
-Os, -O3} = **1305 compilations, zero asm differences**.

## Script consolidation (v4 backlog, executed this session)

* **P1**: `codegen_scoreboard.py` absorbed into `codegen_oracle.py --rank`
  (scoreboard deleted). One metric implementation now feeds both modes —
  the scoreboard's superset SIMD classifier (21 packed-SSE mnemonics the
  oracle's old check missed) became the shared `_stats`; the remote-compile
  cache and the per-thread compiler-catalogue fetch race were fixed in the
  merge.
* **P3**: `peephole_ab.py` absorbed into `perf_ab.py --metric insns`
  (peephole_ab deleted): new `peephole_skip` preset (the 14-pass
  CCC_PEEPHOLE_SKIP list), `--skip PASS`, per-function + total insn counts,
  and the behaviour-identity half (exit 1 on any stdout/exit divergence).
* **T2**: `--from-list FILE` for both gcc-torture runners (failure-focused
  re-runs; unknown names warn+skip, `from_list` recorded in the JSON).
* **R2**: the 9 tests/fuzz engines absorbed into `fuzz_diff.py`'s registry —
  3 as generator-config engines (`differential`, `phi_cfg`,
  `intcmp_thread`), 6 as forwarding engines (`m32`, `regparm`, `slot_rmw`,
  `alias_m32`, `alu_torture`, `aarch64`). `--check-engines` now covers 13/13.
  Host finding: this sandbox links+execs ELF32 but blocks `int $0x80`
  (SIGSYS) — the m32 fuzzers pass vacuously here; the engines now probe and
  label compile-only fallback instead of false confidence.
* `perf_ab.py`: the duplicated `vecreg`/`vecreg_ops` presets unified
  (identical env+kernel lists; `vecreg_ops` kept, no external users of the
  other spelling).

## Validation summary (all on the final tree, main @ ca1c3b34 + this patch)

* `cargo test --lib`: **2069 passed / 0 failed / 6 ignored** (5 new RISC-V
  model tests).
* `RUSTFLAGS=-D warnings cargo test --all-targets` (CI command): green.
* `cargo clippy --all-targets -- -D warnings`: clean. `cargo fmt --check`:
  clean.
* Feature matrix (gcc_linker, gcc_assembler, both) × {check, lib-test}:
  zero errors/unfulfilled expectations, 2069/0/6 each.
* Regression suite: **PASS=647 FAIL=0 SKIP=15** (AB-diff failures: 0) —
  identical counts to the session baseline.
* `ci-codegen-gate.py`: all golden workloads within tolerance.
* `ci-bench.py --strict`: **39/39 kernels match-baseline** across 6 chunks.
* Release build (`build_lccc_o1_j2.sh`, warnings denied): exit 0.
* AArch64: 256/256 corpus files byte-identical vs main.
* i686: 1305/1305 compilations byte-identical vs main.
* RISC-V: 17 files differ, all hoisting-shaped (constants leaving loops);
  238 files byte-identical.
* x86-64: diffs confined to the wave/demotion-eligible functions; the delta
  is the intended rdx homes + dead-pair removal (hot loops
  instruction-identical).

## Remaining backlog (updated)

1. **Hot-loop alignment pass** (new, this session's alignment-lottery
   finding): `.p2align` loop headers; the lz4 loop alone moves ±3–5%.
2. **RA stack-traffic wall** (carried): `spill_weight = priority/live_span`
   still structurally under-ranks long-lived loop-carried values; the rdx
   wave recovered one register of supply, but the 6-callee-saved budget
   remains the binding constraint for lz4-class functions (49–51 frame movs
   vs GCC's 10). Reload-at-next-use / live-range splitting remain the
   architectural answers.
3. **RISC-V `addi` immediate folds** (new): the backend stages every constant
   through `li`; teaching `emit_int_binop_impl` the addi/andi/ori/xori/slti
   forms would grow the free set beyond imm12 (and the int_const_hoist RISC-V
   premise comment flags exactly this interaction).
4. **i686 phi-coalesce hazard hole** (pre-existing, mirrored from the x86-64
   guard added here): the i686 ecx/edx hazard waves have the same
   home-propagation gap the audit found on x86-64 — fix by porting the guard
   once an i686 execution oracle exists to prove it.
5. AArch64 int_const_hoist doc/code mismatch (doc claims shifted-imm12
   support; code accepts ±4095 only) — separate small fix.
