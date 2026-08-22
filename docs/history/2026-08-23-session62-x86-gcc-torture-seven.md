# Session 62 — Seven x86 GCC torture execute fixes

Date: 2026-08-23

Upstream base: `e2a197a29082536d8f71b94f349dfe1284dfad93` (latest `ms178/lccc` main).
Compiler builds used `cargo build --profile fastbuild --locked -j 2`.
`swapon` is not permitted in this sandbox; `scripts/ensure_swap.sh` now
warns and continues so the fastbuild wrapper is usable.

## Mandate

Fix at least five native x86 GCC `gcc.c-torture/execute` failures with
proper compiler fixes (no test-skipping, no golden-output hacks).

Delivered **seven** previously-failing tests, all passing at `-O2` and
the relevant ones also at `-O1`/`-O0`:

| Test | Was | Root cause | Fix |
| --- | --- | --- | --- |
| `20040411-1.c` | execute abort | `sizeof(VLA typedef)*3` const-folded via pointer-size fallback (8*3=24) | Skip const-eval of VLA sizeof; use recorded runtime size |
| `20040423-1.c` | execute abort | `typedef struct { int c[i+2]; }` sizeof not computed | Record runtime sizeof of struct-with-VLA at the type definition |
| `20041218-2.c` | execute abort | `struct s { char b[n]; } packed; n++; sizeof(struct s)` used post-increment `n` | Capture VLA bound at the struct definition; lookup by tag |
| `20021127-1.c` | execute abort | User `llabs` body (which `abort()`s) overrode the builtin | GCC `-fbuiltin`: expand `abs`/`labs`/`llabs` before `is_defined` |
| `20031003-1.c` | execute abort | `(int)2147483648.0f` wrapped to INT_MIN (`cvttss2si` indefinite) | Saturate finite out-of-range float-to-int (GCC `-fno-trapping-math`) |
| `20020720-1.c` | link `link_error` | `fabs(x) < 0.0` not folded | Fold `Cmp(Slt, fabs, 0)` to false (NaN-safe) |
| `20030216-1.c` | link `link_error` | Load of `const double one=1.0` not folded | Fold loads of `is_const` scalar globals |

Native x86 re-run of the session-61 first-500 ARM fail list: **13 pass /
30 fail** (was ~8 pass on x86). The seven named tests are in the pass set.

## Fixes in detail

### 1. VLA sizeof is not a compile-time constant

`eval_const_expr` treated every `Sizeof` as `sizeof_type()`, which for a
VLA typedef stored as `CType::Array(_, None)` returns the pointer size.
`sizeof(c)*3` with `typedef int c[i+2]` and `i=20` became 24 instead of
`22*sizeof(int)*3`.

`eval_const_expr` now returns `None` for VLA sizeof (typedef name in
`vla_typedef_sizes`, local with `vla_size`, array with a non-const dim,
or struct/union containing a VLA). Lowering already knew how to emit the
runtime size for array typedefs; the const-eval skip is what unblocks it.

### 2. Struct / packed-struct VLA sizeof

GCC allows a VLA as a struct member in a local type. `sizeof` of that
type is evaluated at the **type definition**; later stores to the bound
expression are ignored (`n++` in 20041218-2).

- `register_struct_type` records a runtime sizeof under the layout key
  (`struct.s` / `struct.__anon_N`) in `vla_typedef_sizes`.
- Local typedefs of such structs copy that Value under the typedef name.
- `lower_sizeof` looks up both typedef names and struct/union tags.
- Layout walks fields, adds VLA field sizes at runtime, and applies
  non-packed tail alignment. Packed single-field `char b[n]` is just `n`.

### 3. `abs` / `labs` / `llabs` are builtins under `-fbuiltin`

`try_lower_builtin_call` used to prefer a user `is_defined` body over
every builtin except SSE intrinsics. That matches GCC for
`__builtin_alloca`, but **not** for `abs`/`labs`/`llabs`: GCC still
expands the call when a later definition exists (the definition is only
used for taking the address, or with `-fno-builtin-llabs`).

Calls now lower to `select(x < 0, -x, x)` **before** the `is_defined`
check. Address-of `llabs` is unchanged (not a call).

### 4. Saturating float-to-int

GCC `-fno-trapping-math` (and LCCC's default, which does not model FP
exceptions) saturates finite out-of-range conversions:

```
(int)2147483648.0f == INT_MAX        /* 2^31, one past INT_MAX */
(int)(float)2147483647 == INT_MAX    /* 2^31-1 is not an f32 */
```

x86 `cvttss2si` produces the indefinite integer `INT_MIN`. Folding must
happen in the compiler.

- `IrConst::saturate_float_to_int` clamps finite values; NaN/Inf stay
  unfolded.
- `cast_float_to_target` uses it for integer targets.
- `try_fold_float_cast_mapped` uses it (replacing wrap-via-`as i64`).
- `lower_cast` folds constant casts at lowering time so `-O0` matches.

### 5. `fabs(x) < 0` is always false

IEEE 754: `fabs` is never negative, and ordered `<` with NaN is false.
`simplify_function` tracks `fabs`/`fabsf` calls and `FabsF32`/`FabsF64`
intrinsics, propagates through `Copy`, and folds `Cmp(Slt, fabs, 0)`
(and the swapped `0 > fabs`) to `0`. DCE then drops `link_error()`.

### 6. Const scalar global loads

`const double one = 1.0;` is `IrGlobal { is_const, init: Scalar(F64(1.0)) }`.
`constant_fold::run` now folds non-volatile loads of those addresses
(including through `Copy` of `GlobalAddr`) to the initializer, so
`(int)one != 1` becomes a compile-time false and `link_error` is DCE'd.

## Tests added

Regression C (run by `tests/regression/run_regression.sh`):

- `vla_typedef_sizeof.c`
- `vla_struct_sizeof.c`
- `builtin_llabs_overrides_user.c`
- `float_cast_saturate.c`
- `fabs_lt_zero_dce.c`
- `const_global_load_fold.c`

Unit:

- `test_fold_float_cast_overflow_to_int_saturates` (was "no fold")
- `test_fold_float_cast_two_to_the_31_saturates_to_int_max`
- `test_fabs_lt_zero_folds_false`
- `test_fabs_lt_zero_swapped_gt_folds_false`

Script: `scripts/x86_gcc_torture_slice.sh` — native compile+run of a
named slice of `gcc.c-torture/execute`.

## Follow-up (next session)

Prioritized by (affected tests × confidence) / cost. Nested functions
are the largest remaining cluster in the first 500.

### Nested functions (compile-fail, ~12 in the first 500, ~41 in the full suite)

Parser rejects `int foo() { int bar() { ... } }` with `expected ';'
after declaration before '{'`. Needs trampolines + static chain. High
cost, high GCC-torture ROI. Do not fake it with a source rewrite.

Candidates: `20000822-1`, `20010209-1`, `20010605-1`, `20030501-1`,
`20040520-1`, `20061220-1`, `20090219-1`, `920415-1`, `920428-2`,
`920501-7`, `920612-2`.

### Remaining x86 execute fails from the session-61 first-500 list

| Test | Symptom | Likely cause |
| --- | --- | --- |
| `20080604-1.c` | abort | DSE of store through `&x+1-1`; alias of GEP±1 not proven equal to `&x` |
| `20051215-1.c` | SEGV | `if (z) b = d * *z` with `z==NULL` still dereferences; speculative load / if-convert |
| `20041019-1.c` | abort | (diagnose) |
| `20070212-2.c` | abort | (diagnose) |
| `20060929-1.c` | abort | (diagnose) |
| `20040709-{1,2,3}.c` | abort | complex / nested-fn adjacent |
| `20041214-1.c` | SEGV | (diagnose) |
| `20050121-1.c` | abort | (diagnose) |
| `20070614-1.c` | abort | (diagnose) |
| `20070919-1.c` | abort | (diagnose) |
| `20071210-1.c` | abort | (diagnose) |
| `20071220-1.c` | SEGV | (diagnose; `-2` now passes) |
| `20020412-1.c` | abort | (diagnose; `-11` and `-13` pass) |
| `20230630-{2,4}.c` | abort | (diagnose) |
| `920302-1.c` | SEGV | (diagnose) |
| `20041114-1.c` | link `link_failure` | missed DCE of a builtin-constant / overflow path |

### ARM

This sandbox has no qemu-aarch64 and no cross-binutils. ARM execute
cannot be re-run here. The VLA / abs / saturate / fabs / const-global
fixes are IR-level and should apply to AArch64; confirm with
`scripts/aarch64_execute_suite.py` on a machine with GAS 2.47 + qemu.

The original "25 ARM torture fixes" bar is still open. Nested functions
plus the execute cluster above are the path to that number.

### Do not

- Disable `-fbuiltin` to "fix" 20021127-1 the other way.
- Fold `fabs(x) >= 0` (false for NaN).
- Fold NaN/Inf float-to-int (IEEE invalid; unit tests keep them unfolded).
- Treat pointer-to-VLA as a VLA for sizeof (pointer size is constant).

## Validation

```
cargo test --profile fastbuild --lib fold_float   # 16 passed
cargo test --profile fastbuild --lib fabs         # 4 passed
scripts/x86_gcc_torture_slice.sh 20040411-1.c 20040423-1.c \
    20041218-2.c 20021127-1.c 20031003-1.c 20020720-1.c 20030216-1.c
# 7 pass / 0 fail
# plus 6 new tests/regression/*.c at -O2 and -O1; float_cast_saturate also -O0
```
