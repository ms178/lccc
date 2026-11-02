# Frontend semantic analysis

`src/frontend/sema/analysis.rs` builds symbols, function signatures, expression types, and constants before lowering.

Core C constraints live in sema: invalid valued/valueless returns, named aggregate assignment mismatch, incompatible pointer assignment, fixed-prototype arity, and indirect function-pointer arity. Empty legacy `f()` and fallback libc declarations stay permissive because they do not carry a real prototype. Anonymous SIMD aggregate keys are not compared as distinct named types until type identity is canonicalized. GNU char-pointee pointer-sign compatibility is a warning (`-Wpointer-sign`), never a hard error, even under `-Werror=incompatible-pointer-types` (kernel contract); explicit casts suppress the pointer-types diagnostic (GCC parity).

Address-space qualifiers (`__seg_fs`/`__seg_gs`) thread through declarators, typedefs, locals, and address-taken lvalue stores — the array-subscript lvalue path must derive the same address space as the load path (the LK-26 glibc NULL-page store root cause, fixed in `afa22485`). Parameter-position and multi-level qualifier support remains open (FE-23), as do per-pointer-level const qualifiers (FE-02).

`-fprofile-generate/-fprofile-use` flags are handled at the driver level and surface as PGO instrumentation/use (see `src/pgo/`). Preprocessor items landed here: C2x `__VA_OPT__` (balanced expansion before `#`/`##`, GNU named variadics) and `_Pragma` operator de-stringizing; `#error` still macro-expands its token list (FE-24).

Regression: `tests/regression/check_sema_constraints.sh` requires seven semantic diagnostics and compiles/runs a valid variadic, function-pointer, aggregate-copy, and `void *` conversion control.
