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
finding). **All 17 are now fixed (all 28 corpus cases pass)**; full lccc
regression suite stays green (**545 pass, 0 A/B diffs** — no regressions from
any fix). The remaining 8 open bugs from the previous session were closed in the
v2 session:

| Test (regress_) | Root cause | Fix location |
|---|---|---|
| `const_bool_cast_normalization` | `(_Bool)N` nonzero folded to N, not 1 (C11 6.3.1.2) | `sema/const_eval.rs` (`bits_to_irconst`, `cast_i128_to_ctype`) |
| `logical_and_const_bitnot_short_circuit` | `~u` on 32-bit unsigned const-local (stored I64) 64-bit-complemented → `0xFFFFFFFF00000000` | `ir/lowering/const_eval.rs` (BitNot width truncation) |
| `logical_or_rhs_const_keeps_lhs_eval` | `X || <truthy>` / `X && <falsy>` folded to a constant, **dropping X's side effects** | `ir/lowering/const_eval.rs` + `sema/const_eval.rs` (sound short-circuit folding) |
| `const_local_char_init_coercion` | `const char c=220` cached +220 (not -36) | `ir/lowering/stmt.rs` (coerce const-local to declared type) |
| `struct_array_excess_scalar_init` | excess trailing scalar overwrote last element | `global_init_bytes.rs` (`current_idx = fi`) |
| `struct_array_singleton_inner_dim_global_init` | `[][1]` inner-dim brace peeling | `global_init_bytes.rs` (recurse while strides remain) |
| `local_*_singleton_inner_dim_init` (3 tests) | singleton inner-dim local init / redundant list wrappers | `stmt_init.rs` (`is_subarray = !remaining.is_empty()` + new `normalize_leaf_struct_element_items`) |

## v2 session — the 8 previously-open bugs, all now fixed

Each was reduced, deterministic, and oracle-verified. The fix and location for
each:

1. **`unnamed_nonaggregate_struct_member`** — `struct { int; char f4; }`
   (unnamed scalar member, a GCC-accepted constraint violation). GCC ignores the
   unnamed `int` (no layout space, no initializer slot). **Fixed** by skipping
   unnamed non-struct/union members for layout across all 5 struct-field builders
   (C11 6.7.2.1p13): `types_ctype.rs`, `types.rs`, `type_checker.rs`,
   `analysis.rs`, `const_eval.rs` (`.filter_map` dropping unnamed non-aggregate).
2. **`struct_array_double_singleton_inner_dim_ptr_global_init`** — compound-ptrs
   singleton inner dims `[][1][1]` must still peel a brace level. **Fixed** in
   `global_init_compound_ptrs.rs` by changing the leaf-recursion guard to
   `!remaining_strides.is_empty()`.
3. **`global_union_array_scalar_braces_ptr_field_init`** — `union U g[] = {{{{6}}}}`
   (braces around a scalar) was silently dropped on the compound-ptrs path.
   **Fixed** in `global_init_compound_ptrs.rs`: `fill_scalar_array_with_ptrs`
   unwraps a braced scalar via `unwrap_braced_scalar_expr`.
4. **`cond_cast_ternary_truncation`** — the nested `if` folded to *true* even
   though `(char)(g ? 512 : 512)` is 0. **Fixed** in `cfg_simplify.rs`:
   `resolve_value_globally()` ignored cast truncation, returning the raw source
   constant for a `Cast` (so `(char)512` resolved to 512). Now applies
   `to_ty.truncate_i64(from_ty.truncate_i64(v))`.
5. **`stmt_expr_typeof_shadowing` / `stmt_expr_typeof_label_tail` /
   `stmt_expr_gnu_conditional_shadowing`** — GNU stmt-expr/`__typeof__` scoping:
   `get_stmt_expr_ctype()` resolved the tail expression against the sema scope
   first, so a local declaration inside `({...})` was shadowed by an outer
   same-named binding (`long x = ({ unsigned x = -8; x ?: x; }) * 480998226;`
   computed the multiply in 64-bit instead of wrapping at 32-bit). **Fixed** in
   `expr_types.rs`: build the compound-local scope up front and resolve against
   it first (`stmt_expr_result_expr`/`unwrap_stmt_to_expr` unwrap label tails),
   plus a `GnuConditional` arm and an `int`-for-comparison fix in
   `get_expr_ctype_with_scope`.
6. **`small_struct_packed_assign_clobber`** — packed small-struct (`<4/8` odd
   sizes) assignment clobbered adjacent bytes (a 5-byte packed struct stored as
   I64 overwrote the following `marker`). **Fixed** by porting Regehr's
   `structs.rs` helpers (`packed_spill_alloc_size`, `spill_packed_data_to_alloca`,
   `store_packed_data_exact`, `load_packed_struct_i64`) and routing the
   packed-struct assignment through `store_packed_data_exact`.

**Result:** all 28 Regehr corpus cases pass; full suite 545 pass / 0 fail /
0 A/B diffs.

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
