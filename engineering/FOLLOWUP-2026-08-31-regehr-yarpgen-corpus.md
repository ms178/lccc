# Follow-up — 2026-08-31: CCC fork survey + Regehr yarpgen corpus adoption

**Base.** `ms178/lccc` main `b49414c` (PR #316 merge). Environment was a fresh,
wiped sandbox: 2 vCPU / 1.9 GB RAM / no swap / no Rust. Restored rustup 1.98.0
into persisted `/home/user/.cargo` + `/home/user/.rustup`, created the 4 GB
`/swapfile`, and built LCCC with the `fastbuild` profile (per project policy:
`-O1`, no LTO, incremental, `-j2`). All validation below ran on this box; treat
absolute timings as codegen-quality evidence only (no PMU — a microarch gate on
the 14700KF target is still required, per README policy).

## Task: survey ALL CCC forks and adopt the best

Used the GitHub API to enumerate every fork of `anthropics/claudes-c-compiler`
(243 forks, 241 fetched in 3 pages) plus the forks of `levkropp/lccc` and
`ms178/lccc`, then deep-cloned the ~27 with any unique commits and inspected
their diffs.

### Valuable forks (adopted / adopted in part)

1. **`regehr/claudes-c-compiler` `yarpgen` branch** — *the prize.* John Regehr
   (YARPGen/Csmith author, University of Utah) ran differential testing against
   the original CCC and committed **77 unique commits**: ~60 reduced
   miscompilation reproducers, compiler fixes across ~30 source files, and two
   infinite differential-testing harnesses (`yarpgen_diff.py`, `csmith_diff.py`).
   Adopted:
   - 28 reproducers → `tests/regression/regress_*.c` (+`.flags`), GCC-oracle and
     A/B small-slot validated by the existing runner.
   - `scripts/yarpgen_diff.py` + `scripts/csmith_diff.py` (CC0), documented in
     `scripts/DIFFERENTIAL_TESTING.md`.
   - Reference fixes, adapted to LCCC's diverged lowering (8 landed — see below).
2. **`thanhtoantnt/claudes-c-compiler`** (+3 commits): 3000+ lines of
   property-based tests (`tests/*_properties.rs`, 9 suites covering binop/cmp/
   constant-fold/div-by-const/ir-constants/long-double/types). **Deferred**: they
   add a `proptest` dev-dependency and target the original IR; valuable as a
   PBT baseline but not adopted this session (see To-do).
3. **`CrazyTodd-one/claudes-c-compiler`** (+21 commits): real optimizer work —
   `sccp.rs` (1161 lines), `use_def.rs` (598 lines), dead-arg-elimination, loop
   trampolines, advanced peephole folds, 64→32 narrowing. LCCC already has
   `constant_fold`+`gvn` but **no dedicated SCCP or use-def chains**. Deferred
   (see To-do) — rebasing onto LCCC's diverged IR is a dedicated project.

### Rejected forks

- **`wadsaek/claudes-c-compiler`** — **malicious sabotage.** Its single diff
  injects a 4 KB "penguins/cups" QUOTE string into string literals with
  10/256 probability via `/dev/random` (`get_random_value() <= 10`), silently
  corrupting every program that contains a string literal. **Do not adopt, ever.**
- `rosubra` — "Fix all issues" deleted **217 081 lines / 447 files**, gutting
  every optimization pass. Negative value.
- `Matr1x-101` — dumped a 157k-file zlib tree (24 MB) with no compiler changes.
- `yishangzhang` — logging feature but committed a `.orig` file and a binary;
  messy.
- `kaby76` — GRAMMAR.md (C-dialect reference, mildly useful doc only).
- `adwait`/`fabiorafaelcoutada`/`dj707chen` — docs/experiments only.
- `blitzy-public-samples`, `ChaseWNorton`, and ~230 others — zero code changes
  (default-branch HEAD == upstream `6f1b99ac`).

## Adopted + fixed (8 real miscompiles in LCCC)

The 28 ported reproducers **exposed 17 genuine LCCC bugs** (the corpus is the
finding). 8 are now fixed; full lccc regression suite stays green
(**537 pass, 0 A/B diffs** — no regressions from any fix).

| Test (regress_) | Root cause | Fix location |
|---|---|---|
| `const_bool_cast_normalization` | `(_Bool)N` nonzero folded to N, not 1 (C11 6.3.1.2) | `sema/const_eval.rs` (`bits_to_irconst`, `cast_i128_to_ctype`) |
| `logical_and_const_bitnot_short_circuit` | `~u` on 32-bit unsigned const-local (stored I64) 64-bit-complemented → `0xFFFFFFFF00000000` | `ir/lowering/const_eval.rs` (BitNot width truncation) |
| `logical_or_rhs_const_keeps_lhs_eval` | `X || <truthy>` / `X && <falsy>` folded to a constant, **dropping X's side effects** | `ir/lowering/const_eval.rs` + `sema/const_eval.rs` (sound short-circuit folding) |
| `const_local_char_init_coercion` | `const char c=220` cached +220 (not -36) | `ir/lowering/stmt.rs` (coerce const-local to declared type) |
| `struct_array_excess_scalar_init` | excess trailing scalar overwrote last element | `global_init_bytes.rs` (`current_idx = fi`) |
| `struct_array_singleton_inner_dim_global_init` | `[][1]` inner-dim brace peeling | `global_init_bytes.rs` (recurse while strides remain) |
| `local_*_singleton_inner_dim_init` (3 tests) | singleton inner-dim local init / redundant list wrappers | `stmt_init.rs` (`is_subarray = !remaining.is_empty()` + new `normalize_leaf_struct_element_items`) |

## Open bugs (8 remaining) — root-cause analysis for the next agent

Each is reduced, deterministic, and oracle-verified. **Priority order:**

1. **`cond_cast_ternary_truncation`** — `(char)(g ? 512 : 512)` in a nested
   `if`. The value prints `0` (correct) but the inner branch goes *true*.
   **Nested-`if` control-flow / block-layout bug**, not const-eval: two
   dependent `if`s; the inner CondBranch tests a stale/incorrect condition.
   Investigate `cfg_simplify.rs`/`block_layout.rs` and the CondBranch lowering
   for a nested if whose outer condition is a constant.
2. **`unnamed_nonaggregate_struct_member`** — `struct { int; char f4; }`
   (unnamed scalar member, a GCC-accepted constraint violation). GCC ignores the
   unnamed `int` (no layout space, no initializer slot); LCCC treats it as a real
   member (4 bytes, consumes the first `{3}`), so `f4` stays 0. Fix in the struct
   field lowering: skip unnamed non-struct/union members for layout *and* init.
3. **`global_union_array_scalar_braces_ptr_field_init`** — `union U g[] = {{{{6}}}}`
   (braces around a scalar). LCCC silently drops `{{6}}`. Regehr fixed in
   `global_init_compound_ptrs.rs`: handle `Initializer::List` around a scalar via
   `unwrap_nested_init_expr`. Port to LCCC's compound-ptrs path (line ~941/1022).
4. **`struct_array_double_singleton_inner_dim_ptr_global_init`** — same
   compound-ptrs/singleton path but with pointer fields; likely shares #3's
   `!remaining_strides.is_empty()` recursion fix (Regehr fixed in
   `global_init_compound_ptrs.rs`).
5. **`small_struct_packed_assign_clobber`** — packed small-struct (`<4/8` odd
   sizes) assignment clobbers adjacent bytes. Regehr's `structs.rs` rewrite adds
   `packed_spill_alloc_size`/`store_packed_data_exact`/`load_packed_struct_i64`
   (spill via temp+memcpy for odd sizes). **Deep backend change — port carefully
   with the regression tests + A/B harness.**
6. **`stmt_expr_typeof_shadowing` / `stmt_expr_typeof_label_tail` /
   `stmt_expr_gnu_conditional_shadowing`** — GNU statement-expression +
   `__typeof__` scoping. LCCC's GNU-extension support is partial; Regehr fixed in
   `expr_types.rs` (682 lines) and `expr_ops.rs`. These exercise name/type
   shadowing across `({...})` and label-tail `__typeof__`. Verify LCCC even
   supports the dialect before deep fixing; if it's a hard scope-resolution gap,
   that's a frontend `typeof`/stmt-expr project.

## To-do (future sessions)

- [ ] Fix the 8 open bugs above (Regehr's branch has reference fixes for all).
- [ ] Port more of Regehr's ~30 remaining reproducers (28 adopted of ~60).
- [ ] Add Regehr's csmith/yarpgen harnesses to the CI/repro pipeline; reduce any
      new mismatch into `tests/regression/` before fixing (the project's
      correctness-first rule).
- [ ] Evaluate **CrazyTodd SCCP + use_def** as LCCC passes (no SCCP today).
- [ ] Evaluate **thanhtoantnt property tests** (needs a `proptest` dev-dep;
      weigh against LCCC's zero-dependency philosophy — dev-deps don't affect the
      shipped binary).
- [ ] Re-run the **performance** benchmarks (gzip/zlib-ng/expat) after the
      initializer changes to confirm no codegen regression; the corpus is
      correctness, not perf.

## Do not re-open / reject

- `wadsaek` penguin-quote sabotage — never.
- `rosubra` deletion, `Matr1x-101` zlib dump, `yishangzhang` `.orig`+binary.
- The stale `.base_ref` (`4630de0`) in the repo root from an earlier session is
  **not** this session's base; the snapshot base is `b49414c` (see
  `artifacts/.base_ref`).
