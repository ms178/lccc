# Follow-up: S13 codegen-gate fix (re-remove bogus x86-64 indexed-fold staging rule)

**Base:** upstream `563fb0bc` (PR #653, "adaptive unroll for two-lane
I64/U64 map loops"). The 10-commit stack (S05–S13) rebased 10/10 clean —
zero file overlap with #653. Patch: `ms178-1.patch` (snapshot entries
`S13-…`, base `563fb0bc`), 49 files, +5462/−378.

**One-line:** CI's codegen gate went red on PR #652 (expat moves 31→33,
sqlite stackmem 11→12). Root cause was S08's accidental wholesale revert
of F15 (bad rebase, ungated session): the bogus `{%rdx,%r11}`
store-staging refusal in x86-64 `indexed_fold_ok` came back *together
with* its flipped unit test, so unit tests stayed green while codegen
silently degraded. S13 independently re-derived F15's proof, re-removed
the rule, re-flipped the test, and triple-pinned the fold (unit test +
new `check_indexed_fold_scratch_index.sh` + golden gate). Post-fix output
is byte-identical to the pre-regression base on both workloads, and the
full CI surface (test job + bench job + `ci_local.sh --fast/--slow`) is
green on the rebased tree.

## S13.1: Root cause (proven, not guessed)

`X86Codegen::indexed_fold_ok` (the *deciding* half of the indexed-GEP
fold contract) refused store-fed folds whose SIB base/index was homed in
`%rdx`/`%r11`, copying the const-offset fold's scratch set. That set
never transfers: the indexed path never writes either register. The
failing shape (sqlite `put`, 9-byte arm):

```asm
# pre-fix (refused fold → LEA rematerialisation):
    leaq 8(%rsp), %rcx
    movb %r9b, (%rdx, %rcx)
# post-fix (folded, byte-identical to base 19cf2b3c):
    movb %r9b, 8(%rsp, %rdx)
```

+1 insn, +1 stackmem — exactly the gate delta. Expat's +2 moves shared
the root cause (refusal cascaded through RA: same 22 LEAs, worse homes).

## S13.2: The fix + why it is sound (not just green)

`src/backend/x86/codegen/emit.rs`: deleted the `stage_safe` closure;
kept the type+shift mirror. Soundness rests on three independently
verified legs (see the decider's doc comment for the full write
inventory):

1. **Emitter-exact mirror.** The 13-type allowlist + `shift <= 3` matches
   `emit_store_indexed_common` and `emit_load_indexed_common` arm-for-arm
   (read both; identical lists). The override accepts ⟺ the emitter
   accepts — the trait contract's exact demand.
2. **Write confinement.** A folded access writes only the index/value/dest
   homes plus `%rax`/`%rcx`/`xmm0` (SIB formation in-place/pure/`%rcx`;
   stores imm-direct or via `%rax`/`xmm0`; loads to home/acc; the indexed
   path never calls `emit_save_acc` — that call is what makes the
   const-offset sibling exclude `%rdx`/`%r11`, and it is unreachable
   here). `%rax`/`%rcx` are not allocatable homes (5 in-tree citations,
   incl. `MACHINST_ALLOCATABLE_GPRS`); XMM SIB is refused by the emitter;
   staging reads never clobber. So no SIB operand can be disturbed.
3. **i686 untouched.** Its override keeps its home check: i686's
   accumulator/x87 staging genuinely differs (homes 0..=3 rule).

## S13.3: Archaeology — how the bug came back (read before touching this code)

- `bc9e9586` (S05 rebase) introduced `stage_safe`.
- `6e74ff6d` (F15) removed it ("fixes 2 red gates") with a staging-chain
  audit, and flipped `store_fed_folds_…` to
  `store_fed_folds_accept_any_gpr_homed_address_registers`.
- `aa93d172` (S08, "UNGATED: full CI pending") reverted F15 wholesale —
  rule **and** pin — with no mention in its message. The test suite then
  asserted the bug, and only the bench-workflow golden gate (which runs
  on different triggers) noticed.
- S13 re-removed the rule and re-flipped the pin (with an S08/S13
  archaeology note in the test). Lesson: a rebase that touches
  `emit.rs`+`memory.rs` contract tests without mentioning them in the
  message is suspect; session commits touching codegen MUST gate before
  snapshot (this session's snapshots #8–#12 were all UNGATED — that ends
  here: snapshot #13 is `ci_local-full-PASS`).

## S13.4: Pins (a third revert fails in three places, not zero)

1. Unit test `store_fed_folds_accept_any_gpr_homed_address_registers`
   (`memory.rs`, `#[cfg(test)]`, runs in `cargo test` both profiles).
2. `tests/regression/check_indexed_fold_scratch_index.sh`: compiles the
   real `sqlite_varint.c` workload, asserts zero `leaq` inside
   `sqlite_put_varint`, asserts the folded `movb …(%rsp, %rdx)` store,
   and differentially executes a shrunk corpus against `gcc -O2`.
   Validated to FAIL on pre-fix `.s` (1 LEA, no folded store) and PASS
   post-fix. Wired into **both** `ci.yml` and `ci_local.sh` (`--fast`
   caught the missing mirror via `ci-gate-parity` — the mechanism works).
3. The bench-workflow golden gate (`sqlite_varint`/`expat_xml_scan`
   metrics).

## S13.5: Validation tally (all on the rebased tree unless noted)

- `ci_local.sh --fast`: **88/88**; `--slow`: **6/6** (dbgassert cargo
  test 3731, corpus-ssa 816, benchout 204/204, peephole-ws, dbgassert
  corpus 816) → `mode=full` stamp.
- Codegen gate: 9/9, metrics **bit-identical** pre/post-rebase
  (expat {181,17,6,26}, sqlite {256,11,7,34}); both `.s`
  byte-identical to base.
- `cargo test`: 3723 (+flipped pin `ok`); debug-assertions: 3731.
- Regression corpus: 816/816 normal and debugassert (shadow-epoch
  validator silent).
- Differential correctness: 58/58; fib rec2iter: ALL PASS.
- asmdiff vs GAS 2.47: 1290/1290 x86-64 + 583/583 i686.
- Linker fuzz: 128 mutants + 64 grammar links, 0 defects.
- Runtime corpus (`ci-bench.py --strict`, lccc vs gcc): **41/41
  correct, EXIT 0**; geomean 0.724, arithmetic 0.947. Evidence:
  `engineering/evidence/benchmarks/2026-09-27-s13-685da054/` (+ raw JSON
  in `artifacts/`).
- `cargo fmt --check`: clean; strict clippy (`-D warnings`): clean;
  rebuild: zero warnings.
- Bench-observed noise caveat: 2-core KVM, no PMU; sub-20 ms medians are
  noise-dominated (runner flags them); ratios near 1.0 on tiny kernels
  are ties.

## S13.6: Runtime verdicts per benchmark (lccc vs gcc, `-O2`)

Big wins: `fib` 72.9×, `ackermann` ~69×, `constant_recursion` ~61×
(rec2iter/const-recursion), `libm_round_family` 4.3×, `bitops` 1.75×,
`double_reduction` 1.35×, `chacha20_block` 1.26×, `struct_copy` 1.24×,
plus 8 more sub-1.0 rows. S13 rows: `sqlite_varint` 0.97 (tie-or-win —
the fold holds at runtime); `expat_xml_scan` 1.25 (static moves improved
33→26 regardless — the runtime gap is code-shape vs GCC, see TODO).

## S13.7: TODO for future agents (ordered)

1. **P1 — `lz4_match_extend` 1.94× slower than GCC.** Largest runtime
   gap in the corpus; `insndiff`/`encdiff` vs GCC 14 and a hot-loop
   read are the first hour. (Not a regression — but the biggest win
   available.)
2. **P1 — `nbody` 1.41×.** FP-loop codegen gap; vectorizer/SLP suspect.
3. **P2 — `expat_xml_scan` 1.25×.** Static metrics now beat the stale
   baseline, yet GCC runs it 25% faster: compare dispatch-loop shape
   (jump tables? bounds-check motion?).
4. **P2 — `hash_table` 1.18×, `linux_find_bit_scaled` 1.16×,
   `zstd_count` 1.16×, `fannkuch` 1.15×.** Second tier; batch after 1–3.
5. **Process — never snapshot codegen UNGATED again.** S08's ungated
   revert cost a full session; `ci_local.sh --fast/--slow` (~40 min on
   2 cores) is the price of a delivery-grade stamp.
6. **Hygiene — `bench-artifacts/` is not gitignored** (CI litters the
   tree; `.gitignore` covers only the three files). One-line fix, no
   gate impact — bundled here as a note, not done to keep S13's diff
   review-tight.
7. **Upstream watch — PR #653's `NO_MAP_VEC` kill switch** (`vectorize.rs`):
   new `set_no_map_vec` entry; if a future map-benchmark misbehaves,
   check env/plumbing first.
