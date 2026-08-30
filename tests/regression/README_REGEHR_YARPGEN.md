# Regehr yarpgen/csmith regression corpus

These 28 `regress_*.c` cases were ported verbatim from John Regehr's
`claudes-c-compiler` **yarpgen** branch
(CC0, https://github.com/regehr/claudes-c-compiler). Each is a **reduced
miscompilation reproducer** found by differential testing with YARPGen/Csmith
against GCC. They exercise real, subtle C semantics that stress LCCC's constant
evaluation, aggregate initializers, bit-fields, short-circuit logic, GNU
statement expressions, and narrowing.

## Porting

Each `.c` is the C reproducer exactly as Regehr reduced it. The expected output
is the assertion the original Rust harness checked; LCCC's regression runner
(`scripts/run_regression_suite.sh`) validates it two ways:

1. **GCC-oracle differential** — LCCC's stdout+exit must match GCC's.
2. **A/B small-slot differential** — default vs `CCC_NO_SMALL_SLOTS=1`.

`.flags` sidecars carry the compile mode Regehr used (`-std=c99` / `-std=gnu11`,
and `-O0` where he chose it). `-w` was dropped (the runner ignores compiler
warnings).

## Bug-fix status (as of the adopting session)

Running the corpus against LCCC exposed genuine miscompiles. **20 of 28 cases
now pass** (full lccc regression suite stays green: 537 pass, no A/B diffs).
Fixed so far:

| # | Test | Root cause fixed |
|---|------|------------------|
| 1 | `regress_const_bool_cast_normalization` | `_Bool` cast of nonzero constant folded to N instead of 1 (C11 6.3.1.2) — fixed in `src/frontend/sema/const_eval.rs` (`bits_to_irconst`, `cast_i128_to_ctype`). |
| 2 | `regress_logical_and_const_bitnot_short_circuit` | `~u` on a 32-bit unsigned const-local stored as I64 was 64-bit complemented (`0xFFFFFFFF00000000` instead of 0) — fixed in `src/ir/lowering/const_eval.rs` (BitNot width truncation). |
| 3 | `regress_logical_or_rhs_const_keeps_lhs_eval` | `X || <truthy-const>` (and `X && <falsy-const>`) folded to a constant and **dropped X's side effects** — fixed in `src/ir/lowering/const_eval.rs` + `src/frontend/sema/const_eval.rs` (sound short-circuit folding). |
| 4 | `regress_const_local_char_init_coercion` | `const char c = 220` cached as +220 instead of -36 (signed char) — fixed in `src/ir/lowering/stmt.rs` (const-local value coerced to declared type). |
| 5 | `regress_struct_array_excess_scalar_init` | excess trailing scalar initializer overwrote the last element — fixed in `global_init_bytes.rs` (sync `current_idx` with the flat walk). |
| 6 | `regress_struct_array_singleton_inner_dim_global_init` | singleton inner-dim `[][1]` brace peeling — fixed in `global_init_bytes.rs` (recurse whenever sub-strides remain). |
| 7 | `regress_local_struct_array_singleton_inner_dim_init` | singleton inner-dim local init — fixed in `stmt_init.rs` (`is_subarray = !remaining.is_empty()`). |
| 8 | `regress_local_struct_array_double_singleton_inner_dim_init` | double-singleton local init + redundant list wrappers — fixed in `stmt_init.rs` (new `normalize_leaf_struct_element_items`). |
| 9 | `regress_local_union_array_singleton_inner_dim_init` | local union-array singleton init — fixed in `stmt_init.rs`. |

### Still failing (open bugs — see follow-up doc)

| Test | Symptom (lccc vs gcc) | Likely area |
|------|----------------------|-------------|
| `regress_cond_cast_ternary_truncation` | prints seed=0, gcc silent | nested-`if` branch (value 0 but branches true) |
| `regress_global_union_array_scalar_braces_ptr_field_init` | 0 vs 6 | union-array global init; braces-around-scalar dropped (Regehr fixed in `global_init_compound_ptrs.rs`) |
| `regress_small_struct_packed_assign_clobber` | 0 vs 4 | packed small-struct assign (Regehr fixed in `structs.rs`) |
| `regress_stmt_expr_gnu_conditional_shadowing` | 2065871646256031088 vs 446981488 | GNU stmt-expr shadowing |
| `regress_stmt_expr_typeof_label_tail` | 1 vs 0 | `__typeof__` label-tail scoping |
| `regress_stmt_expr_typeof_shadowing` | 0 vs 1 | `__typeof__` shadowing |
| `regress_struct_array_double_singleton_inner_dim_ptr_global_init` | 0 vs 4 | struct-array global init (ptr field, compound-ptrs path) |
| `regress_unnamed_nonaggregate_struct_member` | 0 vs 3 | unnamed non-aggregate struct member |

These are the next correctness targets. Regehr's branch contains the reference
fixes for most of them; adapt them to LCCC's diverged lowering rather than
copying blindly (see the project rules: understand → implement → measure).
