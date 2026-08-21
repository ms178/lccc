# Frontend semantic analysis

`src/frontend/sema/analysis.rs` builds symbols, function signatures, expression types, and constants before lowering.

Session 33 moves core C constraints into sema: invalid valued/valueless returns, named aggregate assignment mismatch, incompatible pointer assignment, fixed-prototype arity, and indirect function-pointer arity. Empty legacy `f()` and fallback libc declarations stay permissive because they do not carry a real prototype. Anonymous SIMD aggregate keys are not compared as distinct named types until type identity is canonicalized.

Regression: `tests/regression/check_sema_constraints.sh` requires seven semantic diagnostics and compiles/runs a valid variadic, function-pointer, aggregate-copy, and `void *` conversion control.
