# FOLLOWUP 2026-10-24: latch-copy phi coalescing, IPA-RA clobber sets, latch-loop peephole folds

Base: `ms178/lccc` main `4ba567022` (merge of #941).
Measured with: the fastbuild preset; Valgrind/Callgrind Ir (the VM has no PMU);
GCC 16.2, Clang 23.1.3 and ICX 2026.2 as oracles.
Every number below was produced by the build this patch delivers.

## Headline

| Metric (70 programs in `tests/benchmark/programs`, `-O2`) | Base `4ba567022` | This patch |
|---|---|---|
| Geomean Callgrind Ir, new/base (69 programs gated on identical output) | 1.0000 | **0.9940** |
| Geomean Ir vs GCC 16.2 | 0.7587 | **0.7542** |
| Static x86-64 instructions (all functions) | 10436 | **10388** (15 functions better, **0 worse**) |
| Static AArch64 / RISC-V / i686 instructions | 17086 / 35747 / 22561 | **17048 / 35710 / 22543** (0 worse on any target) |
| `hot_web_loop` Ir | 1,201,087,784 | **841,213,219 (-30.0%)**; GCC 16.2 841.1M, Clang 23.1.3 841.2M, ICX 2026.2 841.2M |

Programs whose dynamic Ir moved by more than noise:

- hot_web_loop: -29.96%
- zstd_double_fast: -2.18%
- gzip_longest_match: -1.90%
- expat_siphash24: -0.60%
- i686_alu_chains: -0.58%
- linux_find_bit: -0.29%
- matmul: -0.10%
- sqlite_varint: -0.06%

Ten programs showed +0.00001% to +0.009% (at most +14 Ir). All ten were checked and have **byte-identical
assembly** for base and new. The deltas are loader/environment noise, not codegen.

`vecreg_new_ops` failed to compile with the base binary only because that binary was run from outside the
repo tree, so it picked up GCC 14's `immintrin.h` instead of lccc's. Re-measured from inside the tree,
base and new are equal (166,674 Ir).

## 1. Secondary latch-copy phi coalescing (`regalloc.rs`)

**Problem.** An if/else latch has one copy per arm into the same phi destination:
`x = f(old); if (rare) x' = g(x); old = phi(x, x')`. Only the first source was coalesced, because the
`seen_dest` dedupe kept one candidate per destination. Every other arm kept its copy on the hot path.

**Fix.** The apply phase now gets *primaries + extras*:

- Primaries are the existing home-less sources.
- Extras keep the home Phase 2 gave them. They move onto the destination register only when the
  per-candidate proof holds and `phi_home_blockers` finds no foreign holder whose live segments overlap.

The sieve hazard (every source home-less and competing for one register, which cost +52% when tried)
cannot recur, because extras are never made home-less. The dedupe stays.

**Proof obligations, each red-tested:**

- `src_use_path_clear_of_dest_redefs`: a use block that redefines the destination is accepted only if no
  source use occurs at or after the first destination definition and the terminator does not read the
  source.
  Test: `use_path_respects_instruction_order_in_a_redefining_use_block`.
- `source_home_survives_dest_redefs_in`: redefinitions of the destination (other than the selected copy)
  mark the shared home dirty, and so do definitions of **phi-pair-web** members. Reading the source while
  dirty fails.
  Test: `source_home_rejects_a_read_after_a_sibling_source_definition`.
- The dirty set is `phi_pair_web_parent` (Copy edges only), **not** `coalesce_web_parent` (which also
  follows int-Cast edges).
  - Using the cast-inclusive web vetoed `gzip_crc32`'s `v40 <- v17`, because `v19 = Cast(v17)` and
    `v20 = Cast(v19)` are web members. That cost +5 instructions, an extra callee-save and a copy.
  - Cast co-residents are still guarded: `phi_home_blockers` segment-checks every holder of the register
    except the same-home class, and the pair edges mirror the candidate filter.
  - Test: `source_home_ignores_a_cast_of_the_source`.
- The primaries/extras merge is set-based. A `Vec::contains` filter would be quadratic in the pair count.

## 2. IPA-RA callee clobber sets (`ipa_ra.rs` + wiring)

Leaf callees publish the caller-saved GPRs they actually clobber. Callers may keep call-spanning values in
the remaining caller-saved registers, which avoids callee-saved push/pop pairs.

- **Fill.** A post-pass relocation of call-spanning, slotted, unassigned GPRs, by cost, descending. It
  takes the first free register, else calls `ipa_make_room`. `IPA_MIN_FREE_SCRATCH = 1`.
- **Per-call `# LCCC_IPA_KEEP` masks** tell the peephole which registers must survive each call. A
  function-wide mask was rejected because it blocks copy forwarding.
- **Verification.** `ipa_ra::verify` re-checks every published set against the final asm. On a mismatch it
  recompiles conservatively, up to `IPA_RA_MAX_ROUNDS = 2`.
- **Kill switch.** `CCC_DISABLE_PASSES=ipa_ra`. Traces go to `CCC_DEBUG_RA=1` (`[RA-IPA]` lines). No new
  knob literal was added.

**Honest attribution (red-team).** Across all 70 programs, IPA-RA changes exactly two functions:

- `llvm_scalar_facts::sf_inverse`: 30 to 26 instructions. `push/pop %r13, %r14` disappear because values
  live across the call now sit in `%r8`/`%r9`, which the callee provably does not clobber.
- `hot_web_loop::exercise`: -2 instructions, on the 1-in-256 rare path.

Its dynamic effect on this corpus is **nil** (on/off within +/-45 Ir on the eight most-improved programs).
**The 2026-10-22 note that called IPA-RA the largest remaining `hot_web_loop` gap had the wrong premise.**
The gap was the secondary latch copy (section 1): with IPA-RA disabled, `hot_web_loop` still reaches
841.2M Ir.

IPA-RA is kept because:

- it is verified (the verify loop, corpus with all verifiers, 9 `ipa_fill_tests`, 54 lines of
  machinst tests);
- it can be switched off;
- it is strictly better statically in the leaf-call shape it targets.

Two earlier variants were rejected on data:

- round-1 reservation before Phase 2: +3.3% Ir;
- splicing the IPA registers into `available_regs`: positional divergence.

## 3. Peephole folds (latch-loop shapes)

| Pass | Change | Witness |
|---|---|---|
| `fold_copy_into_dying_operand` (`load_op_fuse.rs`) | Reader may fully redefine C (`kills_family_only`). Extension readers (`movz*`/`movs*`) are refused, because renaming them leaves a self-extension (`movzbl %sil,%esi`) that no pass may delete, while the cross-register form is what the relay folds retire. | crc index shape; `affine_countdown` (+1 found by the static census, fixed, pinned by `copy_into_dying_operand_extension_reader_pipeline`) |
| `rename_chain_into_copy` (`local_patterns.rs`) | Drop the 64-bit self-move the rename produced. | 3 tests |
| jcc/jmp inversion | Inverts across emptied fallthrough-only labels. Uses a fresh label scan: the `LabelRefs` cache is unsafe in passes that add label references. | `branch_inversion_tests` (3) |
| `fold_copy_and_mask_into_test` (`flag_peepholes.rs`) | Window 12; all-ones `testb`/`testl` forms; re-run in the Phase-3 fixpoint (skip key `copy_mask_test`), because Phase-2 global copy propagation creates the single-reader shape after Phase 1 has run. | `andq $255` to `testb` in `hot_web_loop` |
| `fold_scaled_index_temp_into_mem` (`relay_and_lea.rs`) | No-displacement two-component LEA composition plus the `(%T,%B,1)` swap. Three-component (disp/sym) LEAs are refused: they are the slow LEA on Skylake-derived cores, including the 14700KF P-cores. | `scaled_index_temp_tests` (3 groups) |

**Pre-existing bugs found by red tests and fixed:**

1. `compute_gpr_live_out` numbering mismatch.
2. `fold_load_copy_relay` ignored argument reads at calls.
3. The chain marker window was too short.
4. `rename_chain_into_copy` self-move.

## Validation (all on the delivered build)

- `cargo test --lib`: **6170 passed, 0 failed, 10 ignored**.
- Regression corpus with `CCC_VERIFY_LIVENESS_SPAN=1 CCC_VERIFY_REGALLOC=1 CCC_VERIFY_IR=abort
  CCC_VALIDATE_SSA=1`: **1133 passed, 0 failed, 1 skipped-compare**.
- `cargo fmt --check`: clean. `scripts/check_clippy_policy.py`: PASS.
- `scripts/audit_census.py --check`: PASS (panic_sites 835 to 834, knob_literals 526 unchanged).
- `scripts/ci_local.sh --fast`: see the delivery note.

## Still open (from the LLVM batch-2 list, re-measured on this build)

| Item | Status |
|---|---|
| `expat_xml_scan` bounds compare | Open; Ir unchanged by this patch. |
| `linux_find_bit` load fold | -0.29% from this patch; the residual load fold is still open. |
| `spectral_norm` loop-PRE | Open. |
| Constant switch to table | Open. `switch_dispatch` `main` is 14/14/11 instructions smaller on AArch64/RISC-V/i686 from the shared register-allocator changes, but not table-lowered. |
| `zstd_count` | +21% vs GCC 16.2; open. |

## Reproduce

```sh
CCC_DISABLE_PASSES=ipa_ra lccc -O2 ...   # IPA-RA off
CCC_DEBUG_RA=1 lccc -O2 -S ...           # [RA-IPA] trace
CCC_DEBUG_PHI_COALESCE=1 lccc -O2 -S ... # phi proofs / BLOCKED reasons
CCC_PEEPHOLE_SKIP=copy_dying_operand,copy_mask_test lccc -O2 -S ...
```

Corpus Ir A/B: `scripts/callgrind_corpus.py` / `scripts/icount_ab.py`. Static deltas: `scripts/census_ab.py`.
