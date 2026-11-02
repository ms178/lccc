/* Cross-compiler by-value ABI provider for Clang ext_vector_type.
 *
 * One function per vector shape, taking and returning the vector BY VALUE, so
 * a single call exercises both the argument classification and the return
 * classification of the psABI (or the i386 rule set). The companion caller
 * links against this file compiled by a *different* compiler, which is the
 * whole point: a per-lane result check catches a carrier mismatch that a
 * same-compiler build would never see.
 */
typedef _Bool bool4 __attribute__((ext_vector_type(4)));
typedef unsigned char u8x3 __attribute__((ext_vector_type(3)));
typedef unsigned int u32x2 __attribute__((ext_vector_type(2)));
typedef unsigned int u32x3 __attribute__((ext_vector_type(3)));
typedef unsigned int u32x4 __attribute__((ext_vector_type(4)));
typedef unsigned long long u64x1 __attribute__((ext_vector_type(1)));
typedef unsigned long long u64x2 __attribute__((ext_vector_type(2)));
typedef float f32x2 __attribute__((ext_vector_type(2)));
typedef float f32x3 __attribute__((ext_vector_type(3)));
typedef float f32x4 __attribute__((ext_vector_type(4)));
typedef double f64x1 __attribute__((ext_vector_type(1)));
typedef double f64x2 __attribute__((ext_vector_type(2)));

/* Each function adds a lane-dependent delta so a swapped, truncated or
 * zero-extended carrier is visible in the result. */
bool4 abi_bool4(bool4 v) { v[0] = !v[0]; v[3] = !v[3]; return v; }
u8x3 abi_u8x3(u8x3 v) { v[0] += 1; v[1] += 2; v[2] += 3; return v; }
u32x2 abi_u32x2(u32x2 v) { v[0] += 3; v[1] += 5; return v; }
u32x3 abi_u32x3(u32x3 v) { v[0] += 10; v[1] += 20; v[2] += 30; return v; }
u32x4 abi_u32x4(u32x4 v) { v[0] += 1; v[1] += 2; v[2] += 3; v[3] += 4; return v; }
u64x1 abi_u64x1(u64x1 v) { v[0] += 0x100000002ULL; return v; }
u64x2 abi_u64x2(u64x2 v) { v[0] += 1; v[1] += 2; return v; }
f32x2 abi_f32x2(f32x2 v) { v[0] += 1.5f; v[1] += 2.5f; return v; }
f32x3 abi_f32x3(f32x3 v) { v[0] += 1.0f; v[1] += 2.0f; v[2] += 3.0f; return v; }
f32x4 abi_f32x4(f32x4 v) { v[0] += 1.0f; v[1] += 2.0f; v[2] += 3.0f; v[3] += 4.0f; return v; }
f64x1 abi_f64x1(f64x1 v) { v[0] += 0.25; return v; }
f64x2 abi_f64x2(f64x2 v) { v[0] += 0.5; v[1] += 1.25; return v; }
