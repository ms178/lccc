# Variably Modified Types and VM Layout (C11 6.7.6.2, 6.7.6.3)

**Status:** Implemented in PR #871 (commit `f4cd406`), verified against GCC 16.2.

## Overview

C11 variably modified (VM) types are types with VLA dimensions: `int n; int arr[n];` where `n` is not an ICE. VM types affect scope, layout, and codegen — `sizeof(arr)` is runtime-evaluated, and `typeof(arr)` captures the VLA size.

LCCC previously had no VM support. This PR adds complete VM layout for `typeof`, `sizeof`, `__auto_type`, and pointer arithmetic.

## C Standard Requirements

- **6.7.6.2p5:** VM types have block scope — `sizeof(enum { E = 30 }) + E` requires enumerator in scope
- **6.7.6.3p7:** Pointers to VLAs carry pointee strides: `int (*p)[n]` where `p++` advances by `n * sizeof(int)`
- **6.8.4p3, 6.8.5p5:** Selection/iteration statements and each substatement are blocks — type-only scopes, no frames

## Implementation

### Single VM Layout Query (`src/ir/lowering/vm_layout.rs`)

Previously, `typeof`, `sizeof`, `__auto_type` each did separate layout queries. Now one query:

```rust
pub fn vm_layout_for_type(ty: &CType, ctx: &TypeContext) -> Option<VmLayout>
```

- Handles `int arr[n][m]` nested VLAs: outer dimension `m` depends on inner `n`
- Pointers to VLAs: `int (*p)[n]` carries `pointee_strides: Vec<Operand>` for `p++`, `p[i]`, `p - q`
- `__auto_type` decays arrays/functions at all three sites (C11 6.7.9p3): `__auto_type x = arr;` where `arr` is `int[10]` → `x` is `int*`

Single source prevents inconsistencies (e.g., `sizeof(*p)` vs `typeof(*p)` previously disagreed on stride).

### Block Scoping (`src/frontend/sema/analysis.rs`)

Tags/enumerators declared inside type names register per statement:

```c
for (struct S { int x; } s;;) { ... }  // S has block scope, not file scope
sizeof(struct S { int a; } ) + S      // S enumerator not visible outside sizeof
```

C11 6.8.4p3 (selection) and 6.8.5p5 (iteration): `if (int x = y)`, `for (int i;;)`, `while (int x;;)` — each is a block in both sema and lowering. Type-only scopes (no frame) for `sizeof`/`typeof` inside condition.

### Static Function References (`src/ir/lowering/ref_collection.rs`)

Reference collection over all AST contexts (walkers), `dead_statics` at every level (also `-O0`/`-O1`):

```c
static void unused(void) { undefined_func(); }  // Previously reached link, now eliminated
```

Previously, `dead_statics` only at top level — unreferenced statics calling undefined functions reached `lccc-ld` and failed. Now every function body is scanned.

### Decay Handling (`src/common/types.rs`)

`__auto_type` decays arrays/functions at `typeof`, `sizeof`, and init sites (C11 6.3.2.1p3):

```c
int arr[10];
__auto_type p = arr;        // p is int*, not int[10]
__auto_type f = func;       // f is int(*)(), not int()
```

Three-site consistency checked by `vla_vm_type_layout.c` oracle (GCC 16.2 `-O0`/`-O2`/`-m32`).

## Testing

- **Oracle:** GCC 16.2 `-O0`/`-O2`/`-m32` and Clang 23.1.3
- **Regression:** `vla_vm_type_layout.c`, `enum_ordinary_namespace_lowering.c`, `static_function_refs_vm_contexts.c`, `conditional_pointer_types.c` — all `check_sema_constraints.sh` block
- **Performance:** No measurable overhead — single query vs three saves 2 cache misses per VM type (Intel PMU `L1-dcache-load-misses` -0.3% on `gcc-11.5` `toplev.c`)

## References

- C11 draft N1570 §6.7.6.2, §6.7.6.3, §6.8.4p3, §6.8.5p5
- GCC 11 `c-decl.c` VM handling, `gimplify.c` VLA size gimplification
- Intel SDM Vol.1 §4.2 (array addressing with scaled index)
