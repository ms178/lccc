# S16 — Census-zero follow-ups, upstream a0e0314 residual audit, #503 carry-table + implicit-operand red team

Date: 2026-09-12. Base: `a0e0314` ("Merge PR #505"). Companion to
`AUDIT-2026-09-12-S15-redteam.md`.

## Scope

After the two-part census regression root-cause fix (function-local pinning
of address-taken frame slots + the `fold_store_alu_memop` CFG-join barrier),
the whole-corpus static census (`scripts/census_full_delta.sh OLD NEW`,
5 opt levels × 807 TUs) was reduced to a handful of residuals. This session
closed every one of them and finished the PR #503 red-team audit (carry
tables, implicit-operand classifier, rename-window rollback).

## Finding 1 — Two more upstream peephole residuals with structural root causes

### 1a. `machinst_window_alloc_spill_base.c` (+1 stack ref at -O1/-O2/-O3/-Os)

Pre-peephole assembly was byte-identical between candidate and upstream; the
divergence was inside `fold_store_alu_memop`. The S15 CFG-join barrier
correctly refuses to cross a label when a branch targets it, but the
`movq %rax,16(%rsp)` store in this TU feeds a compare at the head of an
**if-guard block that no branch targets** (a fall-through-only pre-loop
condition; the self-loop is the later `.LBB2`):

```
movq 200(%rsp), %rax
movq %rax, 16(%rsp)
xorl %r11d,%r11d
xorl %r10d,%r10d
.LBB1:                     # no incoming branch anywhere in the TU
cmpq 16(%rsp), %r10        # upstream: cmpq %rax, %r10 ; delete store
jge .LBB3
.LBB2: … loop body …
jl .LBB2
.LBB3:
```

A label without incoming branches starts a block **dominated** by the code
before it, so crossing it is exactly as sound as crossing straight-line
code. The existing `dead_writes::label_is_fallthrough_only` already computes
this predicate (exact predecessor scan over every `jmp`/`jcc`/indirect jump).
It was promoted to `pub(super)` and the no-operand label arm of
`fold_store_alu_memop` now allows the scan to continue only for that case;
every targeted label still terminates the scan.

Guard tests (`store_alu_cross_join_tests`):
`folds_across_untargeted_guard_label` (positive) and
`refuses_across_targeted_loop_header_label` (adversarial twin).

### 1b. `loop_carried_store_load_forward.c` / `ra_folded_index_wave_follow.c` (+1 at -O0)

Root cause is an interaction between the phase-1 `reuse_redundant_loads`
pass (PR #505) and the phase-2 `fold_memory_operands` backward cascade:

* Raw `-O0` code reloads a scratch register (`%rcx`) before each
  `addq %rcx,%rax` in a loop (`movq slot,%rcx; addq %rcx,%rax`, repeated).
* `fold_memory_operands` turns each adjacent pair into `addq slot,%rax` and
  deletes the load; members earlier in the chain are reached by subsequent
  iterations of the convergence loops.
* PR #505's same-destination reload deletion removes the *redundant* reloads
  first; with the reload gone, later consumers textually read the surviving
  (non-adjacent, barrier-separated) load, whose register is no longer provably
  dead at the fold point — so the remaining members stop folding. Result:
  one materialized reload stays live (`addq %rcx,%rax` relay instead of a
  memory operand), +1 instruction per reload chain.

Two structural changes:

1. **Fold target resolution across destination-establishing copies**
   (`resolve_alu_memfold_target` + `is_dst_establishing_move`):
   `movq -32(%rbp),%rcx; movq %rdx,%rax; addq %rcx,%rax` now folds to
   `movq %rdx,%rax; addq -32(%rbp),%rax` — the intervening pure GP copy only
   establishes the ALU destination and is as sound as the adjacent case.
   The existing RSP-shift / register-rewrite / same-slot-store / width /
   deadness guards all run over the extended window. Refusal is fail-closed:
   the copy must be a plain `movq/movl/movzbq/movslq`, no memory operand, no
   read of the loaded register; the ALU destination must equal the copy
   destination; at most three copies; any label/barrier stops the scan.
2. **Reload preservation predicate** (`memory_fold::load_would_memfold`,
   consulted by `reuse_redundant_loads` before it deletes a same-destination
   reload): a reload that is about to be consumed by the stack-load→ALU
   memory fold is kept; the fold deletes it one phase later. This restores
   the backward cascade without weakening either pass's soundness.

   The deadness check uses a twin helper `scratch_dead_after_relaxed`,
   transparent to control-neutral `push`/`pop` of *other* registers
   (pre-push/pop-elimination -O0 output); real CFG barriers still fail closed.

After: `loop_carried_store_load_forward -O0` is **one instruction smaller
than upstream**; `ra_folded_index_wave_follow -O0` matches upstream's
instruction counts and `-O1` is one store smaller than upstream (a genuinely
dead `movl %eax,8(%rsp)` — slot never read nor address-taken — that upstream
keeps; output verified against GCC).

Tests: `memfold_dst_move_tests` (positive + three adversarial refusals),
`dead_writes::tests::reload_feeding_memfold_consumer_is_preserved` and
`reload_without_memfold_consumer_still_deleted` (near-miss).

### Function-local pinning test-harness fallback

The per-function `.cfi_startproc`/`.cfi_endproc` scoping of
`pin_address_taken_stack_slots` (S15) left unit-test snippets (no CFI
boundaries) unpinned, failing
`regression_tests::test_combined_preserves_address_taken_rsp_slot`.
The pass now falls back to a whole-buffer scan
(`pin_address_taken_global`) only when no `.cfi_startproc` exists; real TUs
always keep the function-local scope.

## Finding 2 — `ra09_selfop_xor.c -O1` "residual" is an UPSTREAM MISCOMPILE

Upstream emits, across a rotl diamond:

```
.LBB26: … shrq %cl,%r14 ; orq %r14,%rax ; movq %rax,120(%rsp) ; jmp .LBB28
.LBB27: movq %r14,120(%rsp)
.LBB28: xorq 120(%rsp),%r11        # upstream: xorq %r14,%r11  (WRONG)
```

LBB26 stores the rotated value through `%rax`; LBB27 stores the identity in
`%r14`. Upstream forwards LBB27's register across the join and reads the
identity unconditionally. Runtime verdict:

| binary at -O1 | stdout (first 16 hex digits) |
| --- | --- |
| candidate / GCC 16 -O1 | `226f0a7b8d22d0af` |
| upstream a0e0314 | `4f1ac1880e71d175` (wrong) |

The candidate's +1 stack reference at -O1 is the correct output; byte-identical
to upstream at every other opt level. The regression harness only compiles
this TU at the default -O2, which is why the upstream bug is invisible there.

## Finding 3 — PR #503 carry audit: missing combined CF+ZF predicates

`relay_and_lea.rs::shift_cf_safe_after` proves the divergent carry from a
`movl; shlq $32` cross-width fold is dead before the next CF reader. The
reader table listed the CF-only predicates (`setb/setc/…`, `cmovb/…`,
`jb/…`) but omitted the four combined CF+ZF predicates in each family:

* `seta`/`setnbe`, `setbe`/`setna`,
* `cmova`/`cmovnbe`, `cmovbe`/`cmovna`,
* `ja`/`jnbe`, `jbe`/`jna`.

The conditional-jump forms are caught independently (CondJmp is a CFG
barrier, checked first), but **setcc/cmovcc are not barrier lines**, and the
`set`/`cmov` prefix-skip rule then actively skipped them — so `seta` after
the shift received the wrong carry on a folded `shlq $32`. Proven by
temporarily removing the new entries: the `seta`/`setbe` refusal tests fail on
the old table. The x87 `fcmov{b,be,a,ae,…}` carry predicates (post-FCOMI)
were added as well (default-deny covered them before, so precision-only).

Clobber credits were audited **on silicon** (i7-class Xeon, STC/CLC + SETC
probe, see S16 evidence commands below). The pre-merge table's PDEP/PEXT
placement was verified correct and a transient "clear CF" claim rejected:

| instruction | CF after STC | CF after CLC | verdict |
| --- | --- | --- | --- |
| `pdepl` / `pextl` | 1 | 0 | **CF preserved → skip, never clobber** |
| `blsil` | 1 | 1 | writes CF=1 (clobber credit sound) |
| `blsmskl` / `blsrl` / `andnl` / `bextrl` / `bzhil` | 0 | 0 | clear CF |
| `ptest`/`vptest`, `pcmpistri/estri/istrm/estrm` | written | written | clobber |
| `comiss/ucomiss/vcomiss/vucomiss`, `fcomi/fcomip/fucomi/fucomip` | written | written | clobber |
| `rdrand`/`rdseed` | written (validity flag) | — | clobber |

Suffixed tokens were added where GAS emits them (`andnl/q`, `bextrl/q`,
`bzhil/q`, `blsil/q`, `blsmskl/q`, `blsrl/q`, `rdrand{w,l,q}`,
`rdseed{w,l,q}`). Tests: five new RMW-coalescer cases (refuse seta/cmova/setbe;
fold after andnl/ptest; refuse across pdep; fold across blsi).

## Finding 4 — implicit-operand classifier completed and unified

`types.rs::classify_implicit_operands` is the exact read/write oracle;
`helpers.rs::has_bare_implicit_gp_effect` was a hand-maintained duplicate
that had already started drifting. The boolean now delegates to the write
projection (`implicit_write_refs`) plus two explicit residual sets, preserving
the designed contract that implicit-READ-only lines (`xsetbv`, `monitor`,
`mwait`, `wrpkru`, `monitorx`, `mwaitx`, `invlpga`) stay transparent to value
windows — family-specific reads are already visible via `LineInfo::reg_refs`
(which unions the read half), and the renaming passes consult the full
read|write mask themselves.

ISA rows added to the exact table (all GAS-validated where a CPU path
exists; i686-only rows marked):

* suffix-less string op tokens (`movs/stos/lods/cmps/scas`), bare `ins`/`outs`
  (`insq`/`outsq` do not exist as GAS mnemonics — verified);
* i686 BCD accumulator ops `aaa/aas/aam/aad/daa/das` (RAX);
* `pushaw/popal`-class (`pushaw/popaw`, `pushal/popal`, AT&T spellings;
  RSP + the eight legacy GPRs, POPAD skips ESP);
* `uiret`, `int1/int3/into` (unknown handler ABI, conservative union),
  `sysexitl/sysexitq`, `sysretl`;
* `rsm` (writes all 16 GP families), `pconfig`, `rdpkru/wrpkru`, `wrmsrns`;
* `vmcall/vmmcall/tdcall/seamcall` (unknown hypervisor ABI, caller-saved
  clobber modeled like `syscall`), `getsec/encls/enclu/enclv`;
* `monitorx/mwaitx` (read-only), `invlpga` (read-only), `skinit` (control
  exit, conservative veto via the residual token set).

Helpers tests: `misc_implicit_gp_effects_flagged` extended (28 tokens),
new `implicit_read_only_system_ops_stay_transparent`, and the pre-existing
`sign_extends_and_reads_stay_unflagged` continues to pin the read/write
contract (it failed when the union was tried, which is how the contract was
re-derived).

## Rename-window rollback review (PR #503)

`coalesce_copy_into_rmw`'s apply-prove-rollback transaction was reviewed
end-to-end and found sound:

* every rewritten line's RAW text is captured before mutation;
* rollback restores text *and* reclassifies `LineInfo` via `replace_line`
  (the NOP-marked copy included);
* pinned lines, barriers and `has_implicit_reg_usage` lines abort before any
  rewrite, so rollback never needs to restore pin state;
* three-operand and SIB operands fail closed (`last_top_level_comma` +
  `plain_gp_operand` rejection of internal-comma/memory text);
* the post-transaction deadness proof is rebuilt from a fresh
  `FileLiveness::new` and anchored at the consumer (the NOP-marked copy
  would answer `None`).

## Validation state

* `cargo test --lib`: 2535 / 0 failed / 6 ignored (was 2521 at S15 close).
* Regression suite: 746 passed / 0 failed / 9 skipped-compare / 0 skipped-run.
* x64 differential corpus (`tests/`, -O2): 0 exit-status diffs, 0 one-sided
  failures; 66 expected (censused) asm diffs.
* i686 differential corpus: **byte-identical** — 792 TUs both compile,
  0 one-sided failures, 0 asm diffs.
* Full delta census (807 TUs × -O0/-O1/-O2/-O3/-Os): sums
  -O0 −164 insns/−169 stk, -O1 −954/−1035, -O2 −259/−257, -O3 −259/−257,
  -Os −249/−249; no positive insn/stk deltas except `fib.c` +1 static at
  -O2/-O3/-Os — accepted (decision below) — and the ra09 correctness case.
* Synthetic differential fuzz (vs GCC+Clang): 400/0/0; the four
  20260912/39/60/90 join-barrier repros still match GCC -O1 md5;
  `check_store_alu_cross_join.sh` PASS.

## Finding 5 — Full red-team review of #503's cross-width fold machinery

In addition to the table gaps above, the three novel algorithms of #503 were
reviewed line by line and found sound:

* **`upper32_zero_at` / `s_write_proves_upper_zero`** (zero-upper proof):
  nearest-writer backward scan; every dword-destination GPR write proves
  zero extension, conditional dword writers are enumerated and refused
  (`cmov*`, `cfcmov*`, `cmpxchg*`, `bsf/bsr`, `rdrand/rdseed`), explicit
  size/address prefixes refused, `mulx`/`xchg` twins fail through the
  non-matching-destination path, partial writes (`movb`/`movw`, `smsw`)
  fail, `movq $imm` accepts only non-negative imm32 with both hex and
  decimal encodings checked (`0xFFFFFFFF` and `$-1` both refuse),
  odd-case text, pins, barriers, labels, calls and implicit-register lines
  all veto. Parameter values correctly refused on open scan (garbage upper
  halves per SysV).
* **`shift_cf_safe_after`** (P2.5 `movl; shlq $32` CF audit): first-event
  scan; reader/clobber/skip tables checked before prefix rules; every
  branch label falls through to default-deny; guarded shifts with
  unprovable (`%cl`) counts are skipped both ways (safe: if they do
  clobber, the later event still governs; if count masks to zero,
  skipping is exact); immediate count masks (`& 63`/`& 31`) handle
  `shlq $64` as shift-by-zero; `adc/sbb/rcl/rcr/cmc/lahf/pushf*/int*/
  syscall/sysenter/salc/BCD` all in the reader set; `inc/dec` correctly
  ride the mov/in prefix skip (CF preserved by definition).
* **apply-prove-rollback transaction**: reviewed in Finding 4 of S15 and
  rechecked; deadness is rebuilt via fresh `FileLiveness` after rewrite.
* **extension-copy folds** (`movzbl/movsbl/movzwl/movswl` + `andl` mask):
  soundness reduces to the mask clearing every bit above the extension
  width; unparseable/wider masks and any non-`andl` consumer refuse.

Residual oracle gap (recorded for the P1 allocator phase, not peephole):
`evidence/oracle_s16/spillguard/` — a high-pressure accumulator chain
with a fall-through guard, -O1: LCCC 85 / GCC 56 / Clang·ICX 41 / ICC 42
instructions. The guard fold itself now matches the oracles (load stays in
`%r8`, register compare, no spill); the gap is the stack-resident
8-accumulator recurrence, which is a global-allocation/scheduling choice.

## Finding 6 — `label_is_fallthrough_only` missed jump-table edges (latent miscompile sealed)

The S16 untargeted-label relaxation crosses a label that "no branch
targets". The predecessor predicate originally scanned only `Jmp`/`CondJmp`
mnemonics' operands. LCCC switch lowering reaches case blocks through an
indirect `jmpq *%rdx`, and those targets are spelled only in jump-table
data:

```asm
    jmpq *%rdx
.section .rodata
.Ljt_0:
    .long .LBB4 - .Ljt_0      # .quad .LBBn on absolute-table paths
    ...
.section .text
.LBB4:                         # no DIRECT branch names this label …
```

Every case label would therefore have been misclassified as
fallthrough-only, allowing a store→register fold/cascade to cross into a
block that is actually entered from the dispatch with different register
state (the fold's precondition `%reg == slot` fails on the indirect edge).
No corpus TU currently crossed such a label (full census identical after
the fix — a latent gap, not a live miscompile), but the predicate is now
exact rather than mnemonic-based: it tokenises every other line on the
assembler symbol alphabet and treats any textual naming of the label as an
incoming edge (token comparison so `.L1` ≠ `.L10`; over-matching only fails
closed). `identical_blocks.rs` already had explicit jump-table awareness
(lines ~554-565); this was the one pass that did not.

Tests (`dead_writes::tests`): `fallthrough_label_without_predecessors_is_untargeted`,
`jump_table_data_references_make_label_targeted` (PC-relative `.long`,
absolute `.quad`, and a `.Lc4` vs `.Lc40` token-boundary near-miss).

## Finding 7 — `is_dst_establishing_move` mnemonic typo (`movlzw` ≠ `movzwl`)

The new destination-establishing-copy step-over
(`resolve_alu_memfold_target`) accepted `"movlzw"`, which is not a GAS
mnemonic (GAS spells word-zero-extend `movzwl`); every -O0 stack-load→ALU
fold behind such a copy silently refused. The allow-list now contains every
reg-reg GAS move/extension spelling (`movq/l/w/b`, `movz{bl,bw,bq}`,
`movz{wl,wq}`, `movs{bl,bq,wl,wq,slq}`). Soundness note: the establisher
copy is only stepped over, never rewritten or deleted, so even partial
writes (`movb/movw`) and extension widths are irrelevant — the fold
substitutes the independent scratch source, and the existing width,
RSP-shift, same-slot-store and deadness guards all still apply.

Tests (`memfold_dst_move_tests`): `folds_through_movzwl_establisher`
(asserts the establishing copy is retained), `folds_through_movb_establisher`.

## Finding 8 — A/B sweep: apparent sub-percent/percent "regressions" are placement noise

Full 39-benchmark paired sweeps (15 reps, seed 4) vs a0e0314 reference build:

| level | geomean LCCC/REF | strict-CI<1 wins | strict-CI>1 "regressions" |
| --- | ---: | --- | --- |
| -O1 | **0.9775** | sha256 0.424, lz4 0.825 | sqlite 1.025, find_bit 1.008, gzip 1.006, expat 1.014 |
| -O2 | 1.0023 | arith_loop 0.968, double_reduction 0.989, ackermann 0.985 | mandelbrot 1.011, strlen 1.024 |
| -O3 | 1.0000 | (none) | expat 1.003, strlen 1.014 |
| -Os | **0.9945** | sha256 0.876, binary_trees 0.959, ring_fifo 0.969, fib 0.968, adler 0.990 | expat 1.025, ackermann 1.024 |

Two facts make the "regressions" non-actionable instruction-level results:

1. **The byte-identical control set moves too.** At -O2, 32 benchmarks
   compile to byte-identical assembly yet the runner reports one of them
   (mandelbrot) CI-strictly *slower* at 1.011 and three CI-strictly
   *faster* (arith_loop 0.968, …) — a ≈12 % false-significance rate at
   15 reps on this VM. `ackermann -Os` (1.024 in-sweep) and
   `find_bit/gzip -O1` (1.006–1.008) are byte-identical to the reference;
   the geomean over all -O2 identical benchmarks is **0.9999**.
2. **The only asm changes among flagged benchmarks are cold write-only
   stack-store deletions** that GCC 16 -O2/-O3 also performs
   (`expat_xml_scan` `movq %rax,24(%rsp)` at a scan-exit merge;
   `strlen_bench` `movq %rax,8(%rsp)` in the driver prologue;
   `sqlite_varint -O1` `movl %eax,20(%rsp)` inside the encode loop).
   Each slot is written once and never read.

A controlled three-way isolation experiment (REF / MINE / PAD = mine with
a length-matched NOP at the deletion site, restoring every downstream byte
offset) over 31 interleaved standalone invocations per case:

| case | mine/ref | pad/ref |
| --- | ---: | ---: |
| expat -O2 | 0.993 | 1.001 |
| expat -O3 | 0.994 | 1.004 |
| expat -Os | 1.021 | 1.030 |
| strlen -O2 | 1.003 | 1.006 |
| strlen -O3 | 0.983 | 0.981 |
| sqlite -O1 | 1.077 | **1.002** |

The store itself is performance-neutral (PAD executes no store yet
reproduces REF placement); the deltas are the known pre-existing
**code-placement phase sensitivity** (`.p2align 4` fixes loop-header mod-16
but not mod-32/64 DSB/cache-line phase of downstream functions). This is
the P1 code-layout workstream (hot-loop/function alignment policy,
A/B-gated), recorded as the first follow-up from this audit. Keeping a dead
store to cajole placement would violate the root-cause policy and diverge
from every oracle; the deletions stand. High-rep multi-seed paired
remeasurement of these six cases is captured by
`scripts/ab_focused_remeasure.sh` (median-of-seeds) and
`scripts/ab_pair_sweep.sh` (whole-corpus, x64 or i686).

## Finding 9 — alignment policy is the root-cause lever (study, not a default flip)

The dead-store placement experiment in Finding 8 points at function-entry
mod-32/mod-64 phase as the noise source. LCCC already implements GCC's full
alignment controls (`-falign-functions/-falign-loops/-falign-jumps=N[:M]`,
default 16-byte function alignment at -O1..-O3, scalar cascade
`.p2align 4,,10` + `.p2align 3`, 32-byte vector-loop cascade, nothing at
-Os — see `src/passes/loop_align.rs`). A pure-flags study
(`scripts/align_policy_ab.py`, REF binary so codegen is held constant,
23 hot benchmarks interleaved over six permutation slots) at -O2:

| variant | geomean (15 r) | wins <.99 | losses >1.01 |
| --- | ---: | ---: | ---: |
| `-falign-functions=32` | 0.9916 | 7 | 3 |
| `-falign-functions=64` | **1.0077 (slower)** | 6 | 8 — rejected |
| func32 **+ `-falign-loops=32`** | **0.9770** | 7 | 5 |

The combined 32/32 policy wins big exactly where Finding 8 showed phase
sensitivity (expat 0.83, sqlite 0.81, switch_dispatch 0.94, arith_loop
0.97), but it also loses >1% on five benchmarks (ring_fifo, histogram,
loop_patterns, chacha20, rbtree) and a single-seed 15-run study on a noisy
2-vCPU VM is not the multi-seed, full-39-corpus (incl. nbody/mandelbrot/
fannkuch/hash_table/strstr), code-size-censused evidence the standing gate
requires for a default change. The default therefore stays GCC-compatible
(16); the full-corpus multi-seed confirmation with a code-size census is
recorded as the first P1 layout follow-up, and the study script ships here
so the experiment is reproducible. A confirmed net win should be taken as
a general policy (predicated on opt level / loop class as the existing
cascade already is), never per-benchmark.

**Confirmation result (31 reps, same hot subset):** the apparent aggregate
win does NOT reproduce — func32_loop32 geomean 0.977 → **0.9999**, with
expat moving 0.828 → 1.014; func32 0.9916 → 0.9974; func64 stays worse
(1.0149). Only two per-benchmark signals are stable across both runs:
sqlite_varint wins from 32-byte function alignment (0.82 / 0.91 — its
encode loop crosses a fetch boundary under 16-byte alignment) and
lz4_compress loses from 32-byte *loop* alignment (1.00 / 1.11). A blanket
default change is therefore rejected; the root-cause direction is
**hotness-driven** alignment (the PGO path in `loop_align.rs` already
gates loop padding on profile-hot blocks and exists for join points) plus
possibly an inner-loop/function-size model, to be validated on a quieter
host with a code-size census. This matches GCC, which also keeps 16-byte
function alignment by default at -O2. The null result is recorded to stop
the blanket-32 idea being retried.

## Final validation matrix (hardened tree, post Findings 6–7)

* `cargo test --lib` (fastbuild): **2541 passed, 0 failed, 6 ignored**.
* Regression corpus: **746 passed, 0 failed, 9 skipped-compare, 0
  skipped-run**.
* x64 differential corpus vs a0e0314 REF: 805 compiled by both, 2 fail
  both, **0 exit-status diffs, 0 one-sided failures, 66 asm-diff TUs — the
  diff set is byte-identical to the pre-hardening censused set** (Findings
  6–7 seal latent/precision gaps without perturbing codegen).
* i686 differential corpus: 792 both-ok, 15 fail-both, **0 / 0 / 0 — fully
  byte-neutral**.
* Fuzz vs GCC+Clang, final binary: synthetic 300/0, differential 150/0,
  phi_cfg 150/0, intcmp_thread 60/0; m32 wide (300 seeds × O0/O1/O2/O3/Os)
  **1500/0 mismatches, 0 compile failures**; aggregate **2160/0**.
* Full delta census vs REF (807 TUs × -O0/-O1/-O2/-O3/-Os): unchanged from
  pre-hardening — -O0 −164/−169, -O1 −954/−1035, -O2 −259/−257,
  -O3 −259/−257, -Os −249/−249; totals **−1885 insns / −1745 stack refs**;
  only accepted residuals fib.c static +1 (hot-loop rotation DR) and
  ra09 +1 stack ref at -O1 (the Finding-2 correctness case).
* A/B paired sweeps (39 benchmarks, 15 reps seed 4) vs REF: geomean
  LCCC/REF -O1 **0.9775**, -O2 1.0023, -O3 1.0000, -Os **0.9945**; strict-CI
  "regressions" all explained in Finding 8 (byte-identical controls and
  byte-exact padding prove placement-phase noise; no instruction-level
  regression). Large wins: sha256 -O1 0.424, lz4 -O1 0.825, sha256 -Os
  0.876, binary_trees -Os 0.959, ring_fifo -Os 0.969.
* `scripts/ci_local.sh --fast`: 27 gates green (pre-hardening run; the
  final run is re-executed after Findings 6–7 and recorded in the delivery
  note). rustfmt clean; clippy `-D warnings` clean.

## Decision record: fib.c static +1 / dynamic −1

Upstream rotates the iterative Fibonacci loop with a compare in a rotated
header (7 executed instructions/iteration); the candidate keeps 6 in the
loop body with the compare at the back edge (one extra static instruction in
the entry/exit plumbing). Output verified against GCC and upstream. Per the
standing gate ("HOT code decides; static instruction count is the
tiebreak"), this is a win and is accepted as-is.
