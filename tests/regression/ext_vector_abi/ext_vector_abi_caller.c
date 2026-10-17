/* Cross-compiler by-value ABI caller: every vector shape, both directions.
 *
 * The skip set is compile-time so one source serves m64 and i386: shapes whose
 * carrier the two compilers cannot agree on for a documented reason are
 * excluded by -DABI_SKIP_<SHAPE> (see the script for the evidence).
 */
#include <stdio.h>

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

extern bool4 abi_bool4(bool4);
extern u8x3 abi_u8x3(u8x3);
extern u32x2 abi_u32x2(u32x2);
extern u32x3 abi_u32x3(u32x3);
extern u32x4 abi_u32x4(u32x4);
extern u64x1 abi_u64x1(u64x1);
extern u64x2 abi_u64x2(u64x2);
extern f32x2 abi_f32x2(f32x2);
extern f32x3 abi_f32x3(f32x3);
extern f32x4 abi_f32x4(f32x4);
extern f64x1 abi_f64x1(f64x1);
extern f64x2 abi_f64x2(f64x2);

static int failures = 0;
static void bad(const char *what) { printf("FAIL %s\n", what); failures++; }

int main(void) {
#ifndef ABI_SKIP_BOOL4
    /* {1,0,1,0} with lanes 0 and 3 flipped is {0,0,1,1}. */
    { bool4 v = {1, 0, 1, 0}; v = abi_bool4(v);
      if (v[0] || v[1] || !v[2] || !v[3]) bad("bool4"); }
#endif
#ifndef ABI_SKIP_U8X3
    { u8x3 v = {10, 20, 30}; v = abi_u8x3(v);
      if (v[0] != 11 || v[1] != 22 || v[2] != 33) bad("u8x3"); }
#endif
#ifndef ABI_SKIP_U32X2
    { u32x2 v = {7, 9}; v = abi_u32x2(v);
      if (v[0] != 10 || v[1] != 14) bad("u32x2"); }
#endif
#ifndef ABI_SKIP_U32X3
    { u32x3 v = {1, 2, 3}; v = abi_u32x3(v);
      if (v[0] != 11 || v[1] != 22 || v[2] != 33) bad("u32x3"); }
#endif
#ifndef ABI_SKIP_U32X4
    { u32x4 v = {10, 20, 30, 40}; v = abi_u32x4(v);
      if (v[0] != 11 || v[1] != 22 || v[2] != 33 || v[3] != 44) bad("u32x4"); }
#endif
#ifndef ABI_SKIP_U64X1
    { u64x1 v = {0x100000003ULL}; v = abi_u64x1(v);
      if (v[0] != 0x200000005ULL) bad("u64x1"); }
#endif
#ifndef ABI_SKIP_U64X2
    { u64x2 v = {5, 7}; v = abi_u64x2(v);
      if (v[0] != 6 || v[1] != 9) bad("u64x2"); }
#endif
#ifndef ABI_SKIP_F32X2
    { f32x2 v = {1.25f, 2.5f}; v = abi_f32x2(v);
      if (v[0] != 2.75f || v[1] != 5.0f) bad("f32x2"); }
#endif
#ifndef ABI_SKIP_F32X3
    { f32x3 v = {1.0f, 2.0f, 3.0f}; v = abi_f32x3(v);
      if (v[0] != 2.0f || v[1] != 4.0f || v[2] != 6.0f) bad("f32x3"); }
#endif
#ifndef ABI_SKIP_F32X4
    { f32x4 v = {1.0f, 2.0f, 3.0f, 4.0f}; v = abi_f32x4(v);
      if (v[0] != 2.0f || v[1] != 4.0f || v[2] != 6.0f || v[3] != 8.0f) bad("f32x4"); }
#endif
#ifndef ABI_SKIP_F64X1
    /* Carried in %xmm0 by lccc; clang-19 and gcc's one-lane double vector use
     * the memory class, so mixed-compiler legs skip this shape (see the
     * script). Same-compiler legs prove the value round-trips. */
    { f64x1 v = {1.5}; v = abi_f64x1(v);
      if (v[0] != 1.75) bad("f64x1"); }
#endif
#ifndef ABI_SKIP_F64X2
    { f64x2 v = {1.5, 2.75}; v = abi_f64x2(v);
      if (v[0] != 2.0 || v[1] != 4.0) bad("f64x2"); }
#endif
    if (failures != 0) { printf("ext-vector cross-ABI: %d failure(s)\n", failures); return 1; }
    printf("ext-vector cross-ABI: ok\n");
    return 0;
}
