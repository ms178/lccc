# Follow-up: PR #364 red-team, direct store immediates, Clz/Ctz zero-guard fold (session 2026-09-03b)

Branch `wip/audit358-359` rebased on upstream main `77eb34e1`; three commits ahead, working tree clean
at the time of writing. Full audit narrative: `AUDIT-2026-09-03-PR363-364-direct-stores-and-zero-guards.md`.

## What this session shipped

| commit | content |
|---|---|
| `cd049d7a` | x86 **direct wide-unsigned immediate stores** in the mature/default emitters (`memory.rs` `direct_store_imm` + const-offset GEP arm + `try_emit_const_store(imm:i64)`; `globals.rs` direct `movX $imm, sym(%rip)` arm). Kills the `%rax` relay for every constant store to memory/global in both text paths. New: `wide_const_imm_stores.c` (GCC-differential), `check_wide_const_imm_stores.sh`. |
| `3f534edf` | IR pre-codegen pass **`bitop_zero_guard_fold`** (`src/passes/mod.rs`, after if-conversion `bit_idioms`, `CCC_DISABLE_PASSES=simplify`): folds `Select(zero-test, Clz/Ctz(v), width)` → copy of the intrinsic arm, and truncating round-trip `Cast` chains over Clz/Ctz roots. Benefits all backends. New: `guarded_clz_ctz_ternary.c`, `check_clz_zero_guard_fold.sh`, 2 unit tests. |
| `c269eabc` | Repaired two **logically impossible regression pins** (aborted under GCC 14.2 at every `-O` level): `sib_index_const_copy_sext.c` (out-of-bounds `listSmall[2]` write + unsatisfiable assertion → in-bounds window, true-result assertion, same fold hazards) and `const_terminator_dead_region.c` (non-dispatchable label in `if(0)` → real nested `case 2:` medce-1 interlock, `foo(2)`). Both pass GCC+LCCC at `O0..O3/Os` with zero `link_error_*`. |

Suite health: 634 passed/10 failed → **640 passed, 2 skipped (env i386 loader), 8 failed** on `-O2`;
all 10 originals reproduced identically on `da964ed4`/`e2d5ef66`/`77eb34e1` (pure upstream), i.e.
pre-existing, ungated upstream (CI only runs `cargo test --lib` + clippy), and none from PR #363/#364.

## Key evidence (kept for the next agent)

* A/B probes `/tmp/ps.c` (stores), `/tmp/lz2.c` (guarded clz), `/tmp/psrun.c` (runtime oracle), corpus
  `k09_clz.c`. Numbers are in the audit doc. lz at `-O3`: upstream 13 insns → this branch 8 → GCC 6
  (branch-shape gap only). `k09_clz` whole body: 13 → 8 instruction lines.
* Upstream binaries for re-measurement can be rebuilt from `77eb34e1` (scratch worktree + shared
  `CARGO_TARGET_DIR`), as done this session (`/var/tmp/tt-up` was deleted after use; budget ~2 min + 950 MB).

## Remaining open items (next agent, by priority)

1. **RISC-V `sext.w` in `check_bitop_nonneg_zext`** (the one bitop-domain red). Analysis done: the
   *result-widen transfer works* (no `sext.w` after the count). The gate trips on a redundant
   `sext.w` during *input canonicalisation*: the clz/ctz lowering zero-extends the U32 operand
   (`slli a0,a0,32; srli a0,a0,32`) then a same-size `UnsignedToSigned` cast re-sign-extends it
   (`sext.w t0,a0`) — dead on an already zero-extended value. Root-cause fix belongs in
   `src/backend/riscv/codegen/cast_ops.rs` (the `UnsignedToSignedSameSize` arm ~line 13-21: skip the
   `sext.w` when the operand is provably non-negative / already zero-extended, mirroring the
   `bitop_nonneg_values` idea already in `prologue.rs`), or by folding the redundant cast pair in the
   shared pipeline before riscv sees it. Not in PR #363/#364 scope; do not paper over with gate edits.
   Sanity reproducers kept at `/tmp/rv1.c` (`w` vs `w2`).
2. **Remaining 7 non-bitop suite reds** (see audit §4 table): `glibc_gottpoff` (TLS), 
   `i686_over_aligned_struct_arg` (i686 ABI), `check_arm_csinc_select` (needs `aarch64-linux-gnu-gcc`),
   `check_affine_map_vectorization_codegen` (`vmulpd`), `check_fma_dest_coalesce_codegen`,
   `check_global_addr_remat` (PIE/GOT), `check_machinst_fallback_replay`. All pre-existing; triage
   each if the session has room — do not treat as regressions from this branch.
3. **Clang 23.1 / ICX oracle runs** for the store-immediate and clz patterns: local sandbox has GCC
   14.2 only; the audit reasons parity from the ISA (raw imm32 encoding is unique), but a Godbolt /
   Compiler-Explorer API capture would make that bulletproof.
4. **Housekeeping**: the repo `.base_ref` should be bumped to `77eb34e1` (per-PR convention: each PR
   sets it to the arena main it is based on), `/home/user/artifacts/.base_ref` refreshed to `77eb34e1`
   so the snapshot diff range covers exactly the three commits, then `scripts/lccc-snapshot.sh`
   regenerates `/home/user/ms178-1.patch`, the per-commit `series/`, the source tarball, the bundle and
   the ledger entry. Verify the ledger entry after the next commit and re-run the snapshot.
