# S25: rebase onto 3275b8b, rustfmt-clean, quest backlog triaged (2026-09-24)

Oracle: GAS 2.47 (self-built) + `scripts/{insndiff,textdiff,encdiff}.py`.
Deliverable: `/home/user/ms178-1.patch` (re-generated vs `3275b8b`),
snapshot S25 via `lccc-snapshot.sh`.

## What happened

- Environment reset wiped the toolchain, `.git`, and all exec bits; the old
  tree (`lccc-s19-tree`, kept for reference) is a stale mix (S24 scripts,
  ≤S19 encoder) — the S24 patch is the source of truth (verified COMPLETE:
  all S20–S24 markers present).
- Rebased onto latest main `3275b8b` (moves only PR #612, IR provenance,
  zero file overlap): `git apply --check` clean, committed as `bddc799`.
- Toolchain reinstalled (rustc/cargo 1.98.1, rustfmt 1.9.0, clippy 0.1.98);
  oracle binutils (`as`/`objcopy`/`objdump`) re-chmodded.
- PR #613 CI rustfmt RED root-caused: pristine main is fmt-clean, all 9
  diffs were in my hunks (`avx.rs`, `mod.rs`, `registers.rs`).
  `cargo fmt --all` → `FMT_CLEAN`, committed as `3ff6b32`.
- Battery on rebased tree: insndiff 19/19 + textdiff 168/168, zero findings.
- `scripts/ci_local.sh --fast` run: see §CI below.

## CI

- `cargo fmt --all -- --check`: CLEAN (reproduced locally).
- `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings`:
  covered by `ci_local.sh --fast` (gate `clippy-fastbuild`).
- `scripts/ci_local.sh --fast`: **ALL GATES GREEN** (66 passed, 0 failed,
  3 skipped). Two latent S24 bugs surfaced by the test/clippy gates and fixed:
  (1) `Register.broadcast` field added without updating the two i686
  `#[cfg(test)]` helpers (E0063); (2) convert-ER over-generalization, see §Q.

## Quest backlog (data-driven, severity-ordered)

Stale-triage warning: `triage-fresh.json` still lists the S23-fixed
`vpextrw` rows — every triage row must be re-verified on the live binary
before fixing. Fresh corpora: `triage-wrong` (84) + `triage-longer` (42)
rows converted to probe corpora, re-run via insndiff/encdiff.

- P0 WRONG-BYTES (miscompiles, verify fresh first): vcmp `aaa`-mask drop
  (`%k5{%k7}` → aaa=000), VSIB gather SIB/disp (`vgather*`
  `0x298(,%xmm4,1)`), `vinserti32x8/64x4` LL=01-instead-of-10 under `{z}`.
- P1 LONGER (perf): EVEX scalar disp8 (`vaddsd 1016(%rdx)` etc., Tuple1 —
  the scalar arms bypass the S21 compression path); 2-byte-VEX
  distillation (clang-optimal, NOT GAS-parity): commutative-operand swap
  in fixed-assignment 3-op encoders (`encode_avx_3op_0f_imm8` puts
  `ops[1]` in RM unconditionally — `vcmpeqps %xmm14,%xmm6,%xmm2` forces
  C4 where clang reaches C5 via swap), move direction-swap for reg-reg
  `vmovq/vmovsd/vmovss` (F3-0F-7E vs 66-0F-D6), symmetric-cmp swap /
  ordered-cmp predicate-flip (oracle support TBD by measurement).
- Swap-safety rules (binding): integer-commutative free; FP add/mul/FMA
  NEVER (NaN payload, measured Skylake-SP, already `DECLINED-FP` in
  encdiff); `vptest/vblendv/vshuf/vperm/vpermil/vunpck` never;
  `vpclmulqdq` swap needs imm-bit-swap (oracle support TBD).
- P2: i686 mirrors of every optimal form (`i686/.../vex.rs` has its own
  `emit_vex`; `i686/avx.casefile` is only 37 lines — grow it).
- P3: IR/optimizer/codegen — vectorizer + isel emit the optimal forms;
  measure on the benchmark corpus (`docs/benchmarks.md`, `bench/`).

## Notes

- `emit_vex` already auto-selects 2-byte VEX (`mm==1 && w==0 && !x && !b`);
  the LONGERs come from operand *assignment*, not prefix emission.
- `vcmpeqps` has no x86-assembler arm (only riscv intrinsics reference it);
  triage rows used it — check `encdiff._canon_insn` vs live-binary
  behavior when rebuilding corpora.
- Host is 2 cores / 2 GB RAM (gcc+bfd link, 8 GB swap active): keep `-j2`,
  single-harness builds; the oracle `bin/` needs `chmod +x` after every
  harness restore (snapshot strips exec bits outside git).

## Q-CMP: VEX packed-compare symmetric swap (distilled from clang/icx)

- Measured (encdiff, all 5 oracles): clang/icx emit 5B (C5) for
  `vcmp* %xmm14,%xmm6,%xmm2` iff `(imm&31)&7 ∈ {0,3,4,7}` (symmetric relations
  in every _q/_s flavor + true/false); ordered predicates never swap (no
  oracle does); scalar (ss/sd) never swaps (merge lane — all oracles agree).
- Implemented in `encode_avx_3op_0f_imm8` (opcode-0xC2-gated; vshuf shares the
  helper and never swaps) + `try_encode_avx_cmp_pseudo` (packed-only) + i686
  mirror. i686 also gained the missing scalar-cmp encoder and the
  vcmp-pseudo expander (shared x86 predicate table, 6/6 byte-match vs GAS-32).
- Verifier canons (`_COMMUTATIVE_VEX_CMP` 32 pseudos + imm-aware
  `_VEXCMP_IMM` for imm>31, which objdump prints numbered) in all three
  scripts; `vex2_cmp_symmetric betterok` (93 rows) + i686 `cmp_pseudo_scalar`
  (23 rows) casefile groups.
- Result: pred.corpus 48 LONGER → 0 (LCCC ties clang/icx on all 88, beats
  gas/gcc/icc by 48B); q2 (107) = 89 BETTER + 18 ok; 19/19 battery green.

## Q-fix: convert-ER can-round rule (S24 regression)

- The `evex_sae_broadcast_and_cmp_cvt` unit test failed: S24 had removed 0x5B
  from the SAE table ("no convert supports SAE/ER") after probing only bare
  `{sae}`. Oracle truth (GAS 2.47, all 14 packed converts × {rz-sae}):
  embedded rounding is accepted exactly when the conversion can round —
  the 8 rounding converts accept, the 4 truncating (vcvtt*) + 2 exact
  widenings (vcvtps2pd, vcvtdq2pd) reject. (First probe round conflated
  shape-rejects (zmm-src for widening) with SAE-rejects; re-probed with
  correct ymm/zmm shapes.)
- Fix: mnemonic-gated class in `encode_evex_vcvt` (Er/None) + LL/rc + b-bit
  threading + ER+mem rejection (GAS text). Unit test extended to the full
  8-accept/8-reject matrix. `evex_tuple_sae` casefile green.

## Quest measurements banked (implementation queued)

- Q-A2-batch-1 fully specified (`/home/user/quest-qa2-plan.md`): scalar-N rule,
  sqrt/FMA/cmp/cvt/mov masked-scalar arms + texts (all GAS byte-refs captured,
  incl. scalar-FMA opcode table (packed+1, W ⟂ width, 6/6) and vmovss/sd/q
  accept-shape matrix (2-op-load/store-EVEX ✓, 3-op-mem-EVEX ✗, vmovq masked ✗)).
- Q-movdir: store-form swap for vmovq/vmovsd/vmovss reg-reg iff src≥8&&dst<8
  (4-oracle-unanimous; no other moves; tie-break = load-form, measured).
- Q-xfamily: 125 same-shape VEX+EVEX duals measured via `{evex}` matrix
  (incl. surprising real ones: vucomisd, vpextrq, vpinsrq — all 62-verified);
  no oracle does shortest-wins (GAS: VEX-always except VNNI-EVEX-always);
  rule = loads-only, EVEX ⟺ VEX-needs-disp32 && disp%N==0 (N=VL default,
  scalar/elem/convert exceptions measured; stores/reg-reg always VEX; ties
  impossible). Explicit `{evex}` will be honored literally (LCCC currently
  ignores it on VEX insns — benign BETTER today).
- Q-codegen smoke: `-O2` vectorizes daxpy (ymm+FMA+scalar tail); visible gaps
  (self-moves) noted for the bench-corpus campaign.
