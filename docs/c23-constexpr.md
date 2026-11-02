# C23 Constexpr Objects (C23 6.7.1p5-6, 6.6p7)

**Status:** Implemented in PR #871 (commits `815e800`, `f4cd406`), verified against GCC 16.2 and Clang 23.1.3.

## Overview

C23 introduces `constexpr` for objects: a `constexpr` object must be initialized with a constant expression, and its value can then be used in other constant expressions (integer constant expressions, static initializers, `_Static_assert`). This is distinct from `const` — `const` means read-only, `constexpr` means compile-time constant.

```c
constexpr int N = 42;          // N is a named constant, usable in ICE
constexpr double PI = 3.14159;  // exact representability required
_Static_assert(N == 42, "");     // OK
int arr[N];                      // OK: VLA size must be ICE, N qualifies
```

## Implementation

### Parser (`src/frontend/parser/declarations.rs`)

- Records object-level `restrict`/`_Atomic` per declarator. Critical distinction:
  ```c
  _Atomic int *p;   // _Atomic is pointee: pointer to atomic int
  int *_Atomic p;   // _Atomic is pointer: atomic pointer to int
  ```
  Previously, `_Atomic` was incorrectly treated as declaration-level. Now per-declarator with `TypeBuilder`.

- Enforces `constexpr` ordering vs `_Thread_local`/`auto` (C23 6.7.1p5): `constexpr` must appear before storage class, and rejects `constexpr auto` etc.
- Rejects storage classes on parameters: `"storage class specified for parameter 'x'"` — previously leaked onto enclosing declaration (e.g., `void f(auto int x) { int y; }` would make `y` auto).

### Semantic Analysis (`src/frontend/sema/analysis/constexpr.rs`)

**Declaration rules** (C23 6.7.1p6):
- Cannot be `typedef`, `extern`, `_Thread_local`, `auto`
- Cannot have underspecified type (`auto` type, incomplete enum without fixed underlying type)
- Single declarator only (`constexpr int a, b;` invalid — each needs initializer)
- Must be initialized with `constexpr`-compatible expression
- Cannot have invalid qualifiers (`volatile`, `restrict` on non-pointer)

**Initializer rules** (shared C11 6.7.9 walk via `src/frontend/sema/init_cursor.rs`):
- Integer constant expression (ICE) for integers, exact representability for floats
- `long double → double` via `binary128` intermediate for correct rounding (Intel SDM Vol.1 §4.8.2)
- Null pointers: `((void*)0)` or integer `0` cast to pointer
- Members: implicit zeros, union active member tracking — members become named constants
- Bit-fields: exact representability including truncation (e.g., `constexpr int x : 3 = 4;` → `4` must fit in 3 bits)

Unit tests: `tests/regression/constexpr_named_constants.c` (GCC 16.2 oracle 87/89 MATCH, auto-order case fixed post-measurement; remaining mismatch `&g != 0` is GCC extension).

### Constant Folding (`src/frontend/sema/const_eval.rs`)

Pre-existing miscompile fixed:
```c
const double d = -1u;  // Previously folded to -1.0 (wrong sign!)
```
Unsigned sources were read as signed. Now `fold_to_type` with source-type reads, single rounding, and `FoldedValue::LongDouble` exactness.

Enum width fix: lowering's packed-enum rule truncated >32-bit enums to 4 bytes while sema used 8. Both now use `EnumType` (single source `type_builder::enum_type_of`).

### Lowering (`src/ir/lowering/`)

`constexpr` objects are lowered as named constants in `SemaResult::const_values` (O(1) fast path for `eval_const_expr`). The lowerer consults this before its own `eval_const_expr` to avoid re-evaluation.

### Diagnostics (GCC 16.2 wording, exact)

- `"not an integer"` — enumerator/constexpr used where ICE required but not integer
- `"not constant"` — initializer not ICE or not exactly representable
- `"failed"` — `_Static_assert` condition false

Static assertions are decided by sema only (not lowering), with enumerators and `constexpr` objects as the only named values (C11 6.6p6).

## Testing

- **Oracle:** GCC 16.2 `-std=c23` (locations stripped), 16/16 fixed-enum diagnostics match
- **Runtime:** `s80_cx.c`, `s80_enum_fixed.c` identical to GCC 16.2 and Clang 23.1.3 at `-O0`/`-O2`
- **Regression:** `constexpr_named_constants.c` with `.flags` `-std=c23`

## Future Work

- `constexpr` functions (C23 6.7.1p7) not yet implemented — tracked in `backlog.md`
- `auto` type deduction for `constexpr` init remains underspecified tag handling

## References

- C23 draft N3054 §6.7.1, §6.6
- GCC 16.2 `c/c-typeck.c` constexpr handling
- Clang 23.1.3 `SemaDecl.cpp` `CheckConstexprVarDecl`
