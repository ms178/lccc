# 2026-08-20 (session 23) — full critical audit of Agent B's patch (ms178-1-AgentB.patch)

**Base:** `origin/main` @ `6d2ac62` (PR #140 — session-22 lever-3 landed).
**Patch under audit:** Agent B's `2026-08-20-session21-levkropp-audit-gauntlet`
(base `53b4254`, 1852 lines: levkropp adaptations, AArch64 RA fix, new passes,
fuzzers, gauntlet repros).
**Method:** every hunk re-derived from first principles, then re-validated on
the rebased tree with the full battery + targeted repros.

## Verdict table

| Item | Agent B | My verdict | Why |
|---|---|---|---|
| levkropp GVN F32/F64 load CSE | ADAPTED | **DISAGREE — REVERTED** | Breaks x86-64 FP register coalescing: `fp_die_at_birth` miscompiles (chain_div returned chain_neg's value). The created FP Copies are not handled by the die-at-birth FP chain on this tree. Their battery (180 m32 seeds + 21 regressions) did not include this test. Kept the exclusion `ty.is_float()` with the reason documented at the site. |
| redundant_loads pass | ADAPTED | **AGREE with the design, DISAGREE with a critical implementation detail — FIXED** | The store-invalidation predicate was INVERTED: `retain(!forms_disjoint)` keeps may-alias loads across a store (stale forwarding) and removes provably-disjoint ones. `gvn_global_partial_store` proved it (word load forwarded across a byte store to the same address). Flipped to `retain(forms_disjoint)`; alias fuzz 180/180 + battery green after the fix. |
| redundant_loads volatile gate | hard gate | **AGREE** | Volatile loads are observable side effects (C11 5.1.2.3); never merge. Unit test `volatile_loads_are_never_merged` pins it. |
| redundant_loads source_spans sync | kept parallel | **AGREE** | Backend indexes spans by position under -g; levkropp's original desynced them. Correct and necessary. |
| quadratic_sr pass | ADAPTED | **AGREE, with corrected soundness claims + doc fixes** | The transformation (triangular index → two carried counters) is exact for t(t+1) < 2^N. Their claim "bit-identical including wraparound" is FALSE: (P mod 2^N)/2 ≠ (P/2) mod 2^N once P/2 ≥ 2^(N−1) (e.g. t = 2^16: direct gives 32768, recurrence gives 2147516416). The transform is legal ONLY under the signed-overflow-is-UB contract (as GCC/Clang do); under an explicit -fwrapv contract it diverges. Doc rewritten to state this; also fixed the overstated `find_div2_result` comment (it matches ONLY the corrected-division chain, not plain SDiv/AShr forms). Matcher stays cold on i686 (no div_by_const chain) — confirmed. Added `quad_sr_triangular` differential regression. |
| alias.rs (LoopFrames, forms_disjoint) | ADAPTED | **AGREE** | Conservative disjointness contract (different roots ⇒ may-alias, never "disjoint"; marching terms only comparable same-frame; checked arithmetic refuses on overflow). Sound. |
| AArch64 folded-index RA fix (`folded_index_uses`) | ADAPTED | **AGREE with the fix, DISAGREE with the scope — REGATED** | The bug is real: `[base, index, lsl #N]` consumes the index at the Load/Store with no IR-visible use; allocator reuses the register (fa[i] stores landing on fa[seed]). BUT: applying the interval extension globally regressed x86-64 `check_gpr_leaf_param_codegen::pointer_mix` (+2 callee-saves). Root cause: ONLY arm overrides `emit_load_indexed/emit_store_indexed`; x86-64/i686 return false and RE-MATERIALISE the skipped GEP at the load (IR-visible uses intact), so the extension there only adds pressure. Now per-backend: arm passes `collect_folded_index_links(func)`, x86-64/i686/riscv pass empty. Env gate `CCC_NO_FOLDED_INDEX_LIVENESS` kept for A/B. |
| i686 peephole unary-RMW implicit-write fix | ADAPTED | **AGREE, COMPLEMENTED** | Their `line_writes_reg_implicitly` arm (neg/not/inc/dec/bswap) complements the session-20 `classify_line` dest_reg fix (different mechanisms: writes_src vs writes_dst paths). Completed the family: classify_line now also covers bswap + single-operand shifts/rotates (shift-by-1 forms) that BOTH previous lists missed. |
| CCC_NO_I686_PEEPHOLE gate | added | **AGREE** | Standard bisection tool. |
| regparm fuzzer / aarch64 fuzzer / alias fuzzer | new | **AGREE** | All three adopted (regparm covers the boot ABI gap; alias fuzz now guards the fixed retain polarity). |
| gauntlet repros + open defects writeup | docs | **AGREE** | zlib-ng gz_reset + sqlite openDatabase crash fixtures kept; see the defect section below. |
| Battery claims (gzip 30/30, expat diff-clean, zlib-ng build) | claimed | **CREDIBLE, not re-run this session** | Consistent with my earlier gzip/expat runs on this tree; zlib-ng runtime defect matches my independent observation class. |

## The two x86-64 -O0 segfault families (open defects 1)

Agent B's characterisation is accurate and my independent analysis agrees:

1. **zlib-ng gz_reset**: `mov %r15d,(%r15)` — a zero materialisation writes
   THROUGH the register that still holds the live base pointer. Same disease
   family as the i686 unary-RMW alias bug and the AArch64 folded-index bug:
   an emitter path writes a register it assumes dead that the allocator
   still considers live (or vice versa). Survives `CCC_DISABLE_PASSES=all`
   → frontend/lowering/backend-emit class, NOT an IR pass.
2. **sqlite openDatabase**: rdi holds a rodata-looking value at the
   sqlite3SchemaGet call — register corruption preceding the call, after an
   out-param (`&db->aDb[0].pBt`) + member-chain sequence. Isolated
   single-pattern repros pass → needs openDatabase's full context.

Session-23 adds the lever-1 x86-64 data point to this family: the FULL
immediately-consumed home-exclusion also corrupted live pointers
(simd_sse2_arith SIGSEGV) via the single-entry acc cache — the same
"emitter/allocator liveness disagreement" shape. Scoped to Return-consumers
(proven path), battery green.

## Cross-backend porting status (session-23)

| Lever | i686 | x86-64 | arm | riscv |
|---|---|---|---|---|
| L1 acc-flow no-home | ALL consumers (uniform acc codegen) | Return-consumer ONLY (others pending per-path audit — BinOp direct-dest + single-entry acc cache made the full set unsound) | n/a (different arch model) | n/a |
| L4 load-hazard refinement | shipped (session 21) | n/a (16 regs; ecx not a scratch bottleneck) | n/a | n/a |
| folded-index liveness | empty (re-materialises) | empty (re-materialises) | ACTIVE (emits indexed forms) | empty |

## Validation (session-23 final tree)

- unit 959/959 · correctness 50/50 · regression 352 + 6 known gcc-14 set
- m32 fuzz 900/900 · alias fuzz 180/180 · slot-rmw fuzz 750/750
- simd_sse2_arith restored green · gpr_leaf_param restored green
- boot objects text 32 579 (unchanged by the audit fixes — as expected:
  they are correctness/scope corrections, not size levers)

## SIB-indexed addressing for i686 — implementation spec (session-22 roadmap #1)

Investigated this session; spec'd precisely for immediate execution:

**Target shape** (boot corpus, e.g. cpucheck flag arrays):
```
movl $cpu, %edx        ; base materialisation      ┐
shll $2, %ebx          ; index *= 4                ├ 5-7 insns, 2-3 regs
leal (%edx,%ebx), %ebp ; address                  │
movl %ebp, %ecx        ; relay (scratch demand)    │
movl (%ecx), %eax      ; load                     ┘
```
→ GCC: `movl cpu(,%ebx,4), %eax` (1 insn, saves ~8-14 B per site + a
register; 38 materialisations / 40 index chains in the boot corpus,
est. −300..−500 B).

**Why the existing machinery doesn't fire for i686 globals:**
`build_indexed_gep_map` builds the fold map, but `can_indexed_addr_fold`
requires `get_phys_reg_for_value(base).is_some()` — a GlobalAddr base that
is otherwise foldable has NO register (never_materialized), so the map entry
is rejected and the GEP rematerialises.

**Design (two parts):**
1. **generation.rs**: thread the base symbol through the indexed fold.
   `generate_load`/`generate_store` already hold `global_addr_map`
   (value→symbol). Extend the indexed branch: if `info.base` has no
   register but `global_addr_map` maps it to a non-TLS, non-GOT symbol
   (same predicate family as `can_const_addr_fold`), call a new hook
   `emit_load_indexed_sym(dest, sym, index, shift, ty)` (trait default
   false → rematerialise as today).
2. **i686 emitter**: implement the hook:
   `movX sym(,%idxreg,scale), %dest_or_eax` / store dual. Index must be
   register-resident (`get_phys_reg_for_value(info.index)`); scale from
   `info.shift` (1/2/4/8). 16-bit (-m16) code needs the 0x67 address-size
   prefix — GAS emits it automatically for 32-bit SIB under `.code16gcc`;
   verify the assembler path handles the prefix before enabling for boot.
3. **RA link**: the index is consumed at the Load with NO IR-visible use —
   i686 must pass `collect_folded_index_links(func)` (currently empty for
   i686) once the hook is live, exactly like arm (the per-backend gating
   shipped this session is the vehicle).

**Validation gate:** m32 fuzz + a new indexed-address adversarial scenario
(global array + scaled index + interleaved stores) + boot corpus delta.
