/* Vector returns must reach the caller without a stack round trip.
 *
 * `v8 f(v8 a){ return ext8(a); }` is one instruction in GCC 16.2
 * (`jmp ext8`) and 25 in LCCC, because the IR carries a vector return as
 * packed `I64` data (`src/ir/lowering/lower.rs`: "A small vector travels as
 * packed I64 data") and `lower_function_call` then decomposes the `%xmm0` /
 * `%ymm0` result into GP halves.  This fixture is the execution half of the
 * gate: whatever the codegen does, the VALUES must arrive intact at every
 * width, so a future register-home fix cannot silently corrupt an upper lane.
 *
 * Compiled per width: see check_vector_return_abi.sh.
 */
typedef float v4 __attribute__((vector_size(16)));
typedef float v8 __attribute__((vector_size(32)));

v4  ret4(v4  a) { return a * 2.0f; }
v8  ret8(v8  a) { return a * 2.0f; }

/* A value that came back from a call, then forwarded -- the shape that produced
 * the `movq`-pair decomposition. */
v4  fwd4(v4  a) { return ret4(a); }
v8  fwd8(v8  a) { return ret8(a); }
