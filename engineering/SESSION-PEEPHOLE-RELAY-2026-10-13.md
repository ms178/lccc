# Session: peephole store-forwarding + FP dead moves + memory-operand relays (worst-15 perf)

Date: 2026-10-13. Follow-up to `SESSION-IVSR-VECADDR-2026-10-12.md` (all 5 ci
gates green at S04) and the worst-15 Callgrind list (AUDIT-pr793-followup
§4). Strategy per user: **no re-analysis, find+implement only high-ROI wins,
validate every claim against the Godbolt oracle, snapshot every validated
fix.** This session moved three of the worst-15 benchmarks decisively.

## Accomplished (all validated, snapshotted S05–S07)

### 1. Scalar-FP store→load forwarding survives read-only indirect lines (S05)

`store_forwarding.rs` tracked `movsd %xmmN, slot` → `movsd slot, %xmmM`
mapping already, but `gsf_handle_other` invalidated ALL mappings at any line
with `has_indirect_mem` — including pure memory READS. The nbody d² chain
stores its `dx`/`dy` extracts to spill slots, interleaves plain loads
(`movsd (%r11), %xmm4`, `vsubsd 16(%r10), …`), then reloads the slots: the
blanket kill kept every such round-trip a double memory stall.

Fix: the shared conservative classifier `dead_writes::line_writes_memory`
decides instead. A read cannot change slot contents; only actual/RMW/implicit
writers stay barriers. Also strengthened `line_writes_memory` itself (both
consumers get strictly more conservative): `stos*`/`ins*`/`maskmov*`/
`xchg*`/`cmpxchg*` are writers regardless of operand order (the old
last-operand rule missed `xchg (%rax), %rbx` and `maskmovq`), and
`cmp*`/`test*`/`ucomi*`/`comi*` are readers even with memory in AT&T
destination position. Implicit writers with no memory operand at all (`stosq`)
never set `has_indirect_mem` — a second guard keeps them barriers.

Result on the pair loop: the dx round-trip forwarded to `movsd %xmm15, %xmm6`
and its slot store died.

### 2. Dead XMM reg-move elimination + shuffle def-model fix (S06)

`eliminate_dead_reg_moves` is GP-`movq`-shaped (`reg_refs` is a 16-bit GP
mask), so the extract-staging idiom `movapd %xmm15, %xmm0` immediately
overwritten by `pshufd $0x0E, %xmm15, %xmm0` survived in every pair loop.

* New `dead_code::eliminate_dead_fp_moves`: pure XMM→XMM reg moves (incl.
  store forwarding's 3-operand `vmovsd` merge forms) whose destination is
  never read before its next rewrite die. Uses region-aware `FpLiveness::reg_move_dead_after` after the safety repair.
  Wired after every `eliminate_dead_reg_moves` site in the pipeline.
* `fp_liveness::is_full_xmm_def` misclassified the LEGACY immediate shuffles
  (`pshufd`/`pshufhw`/`pshuflw`) as dest-reading merges — they build the
  destination from source+immediate only. Legacy `shufps`/`shufpd` DO read
  their destination and must stay excluded (the original narrative incorrectly
  grouped them with source-only shuffles). The VEX spellings
  were already full defs. This fix is what lets the liveness answer "dead"
  across a shuffle overwrite; it benefits every FpLiveness consumer.

### 3. Memory-operand move relays + same-block rip-LEA CSE (S07) — biggest win

Unrolled numeric loops (the histogram shape) emit, per element:

```text
    movzbl (%rdi, %rsi), %eax
    movl   %eax, %r11d            <- relay copy into the SIB INDEX
    leaq   bins(%rip), %r9        <- rip-LEA REMAT, 4x per unrolled iteration
    addq   $1, (%r9, %r11, 8)
```

`eliminate_move_relays` only folded copies feeding REGISTER operands, and no
pass deduplicated identical `leaq sym(%rip)` materializations.

* **Memory-operand relays** (`eliminate_move_relays` third consumer case):
  the SIB base/index registers are read before the instruction's own register
  writes, so renaming them to the copy's source is value-preserving even when
  the consumer later overwrites that source (`movzbl (%rdi,%rdx),%edx` — the
  address is computed first) and even when the source appears twice in the
  line (the relay invariant gives %D == %S at the consumer, so `addq $1,
  (%r11,%r9,8)` with %r11 a copy of %r9 legally folds to `(%r9,%r9,8)`).
  Width contract unchanged: a memory operand reads at 64 bits, so 32-bit
  copies need `upper32_zero_at`. One hazard is refused: the rename must not
  introduce a REGISTER WRITE of %S (a 32-bit RMW destination naming %D would
  zero-extend over %S's live high half) — `consumer_writes_dst_as_reg`.
  High-byte veto (established remedy, `copy_coalesce` rule 5): a `%ah/%ch/%dh/
  %bh` spelling of either family anywhere in the function refuses the fold
  (pins `test_rcx_address_copy_high_byte_use_is_not_a_kill`).
* **`cse_symbol_lea`** (new pass in `relay_and_lea.rs`): same straight-line
  run, `leaq sym(%rip), %R` when `%H` already materialized `sym` and is
  unmodified: `%R == %H` deletes the no-op redef; `%R != %H` becomes
  `movq %H, %R` for the relay machinery to fold. Homes die at any write of
  `%H`; barriers clear the run.

## Measured results (Callgrind A/B vs GCC 14.2, `--opt=-O2`, output-checked)

| benchmark | Ir ratio before | Ir ratio after | note |
| --- | --- | --- | --- |
| histogram | 2.05 | **1.036** | hot loop 23→18 insns per unrolled 4 |
| nbody | 1.29 | **0.878** | beats GCC 14.2 on Ir |
| ascii_case_fold | 1.83 | **1.287** | relays helped here too |
| ring_fifo | 2.50 | 2.73 | GCC VRP miracle (see To-do) |

Oracle (pairs kernel, -O2 -march=x86-64-v3): lccc 82→~78 total insns vs
gcc16.2 52, clang 58. Inner loop 41→40 vs gcc 29. nbody native runtime A/B
(50M steps, 7-run interleaved min): 1.09 → **1.078** (outputs identical).

## To-do (priority order for the next session)

1. **`ring_fifo`/`histogram` VRP-class branch folding (the big structural
   gap).** GCC proves `((head-tail)&1023)!=1023` and `head!=tail` constant and
   eliminates the ring array entirely (13 insns total). Clang 23.1 is 57 and
   icx 90 — lccc (56) already beats both; matching GCC needs value-range
   propagation + loop induction range proofs (head-tail ∈ {0,1}). Feature
   work, not a peephole. Until then ring_fifo stays ~2.7x Ir (vs local gcc14).
2. **Dead pure-op elimination with flag liveness** (`shlq $2, %r13` orphans
   after SIB-folding: `movl %r10d,%r13d; shlq $2,%r13` with dest and flags
   both dead until the next `cmpl`). Generalizes the FP dead-move pass to GP
   ALU/LEA/shift forms; needs the flag-reader scan (`flag_peepholes` helpers).
   First evidence: ring_fifo inner loop, and probably every SIB-folded store.
3. **histogram 4th-element anomaly**: the unroll's last increment expands as
   `movq %r9,%rdx; leaq (%rdx,%rax,8),%rdx; movq (%rdx),%r8; leaq 1(%r8),%rax;
   movq %rax,(%rdx)` (5 insns) where the first three use `addq $1,(%r9,%rax,8)`
   (1 insn). Find why the RMW-forming pass stops at 3 of 4 (back-edge live
   range?) and why `fold_copy_into_lea_base` misses `movq %r9,%rdx` +
   `leaq (%rdx,%rax,8),%rdx`. ~4 insns/iter more.
4. **Vectorize the init/PRNG shapes** (ascii_case_fold residual 1.29): GCC
   emits AVX2 `vpmulld` 32-wide for the `i*1103515245>>16` LCG and the
   fold+store loop. lccc keeps both scalar. Also `mod → mul-high` strength
   reduce (`v % 95`). Feature work in the vectorizer/instcombine.
5. **nbody pair loop residual** (40 vs gcc16.2 29): dy slot round-trip (the
   accumulator churn `movsd %xmm6,%xmm0 … movsd %xmm0,%xmm6` clobbers the
   extract's backing register before the reload — a copy-out/copy-back window
   coalesce in `copy_coalesce` would collapse both), mass_i reload+redup per
   j-iteration (needs i<j alias proof — deep), `leaq 40(%r10),%rsi` per
   j-iteration (negative-displacement addressing).
6. **ci-gate parity for the new passes**: if any gate output changed, re-pin
   via `check_ci_gate_parity.py` with an explicit note (the S07 gates were
   re-run clean after the rustfmt race; see session log).

## Pitfalls recorded (do not relearn)

* Never run ci_local concurrently with source edits — the gates compile the
  tree mid-edit and produce phantom failures (5 bogus gate failures at S07
  pre-fmt state; clean rerun is the only valid result).
* Snapshot slugs must not contain `:` (the artifacts copy fails on the
  filename). Pass `lccc-snapshot.sh` a colon-free slug; the script prefixes
  the S-number itself.
* The relay invariant (D == S at the consumer) makes same-line multi-mentions
  of S legal to merge — the earlier "SIB index conflict" premise was wrong and
  is pinned as `mem_relay_merges_same_value_sib_legally`.
* `function_range` returns `Option`; use `unwrap_or((0, store.len()))` for the
  conservative whole-file veto window.
* Pipeline tests that want observable behavior must store results to memory
  (`movq %r9, (%rdi)`) — otherwise dead-code elimination correctly vaporizes
  the whole fixture and the assertion measures nothing.
