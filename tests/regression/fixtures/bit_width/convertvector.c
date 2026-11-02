/* Opaque __builtin_convertvector kernels for convertvector_check.c: one per
 * lane-type pair and vector width.  Each loads its operand and stores its
 * result through may_alias vector pointers, so the lane conversion is the
 * only code under test (no vector calling convention is involved).
 *
 * x86-64 converts the signed 4 x i32 <-> 4 x f32 pair with one packed
 * instruction; every other pair, and every pair on i686, AArch64 and
 * RISC-V, converts lane by lane.  The unsigned, 64-bit-lane and
 * integer-to-integer pairs below are the shapes the previous lowering
 * either miscompiled or stopped on. */
#define V(name, T, bytes) typedef T name __attribute__((vector_size(bytes), may_alias));
V(v4si, int, 16)
V(v4su, unsigned, 16)
V(v4sf, float, 16)
V(v2df, double, 16)
V(v2di, long long, 16)
V(v2du, unsigned long long, 16)
V(v8hi, short, 16)
V(v8hu, unsigned short, 16)
V(v16qi, signed char, 16)
V(v16qu, unsigned char, 16)
V(v4hi, short, 8)
V(v2si, int, 8)
V(v2sf, float, 8)
V(v8qi, signed char, 8)
V(v8qu, unsigned char, 8)
V(v4df, double, 32)
V(v8si, int, 32)
V(v4di, long long, 32)
V(v8sf, float, 32)

#define K(name, From, To)                                                     \
  void cv_##name(const void *in, void *out) {                                 \
    *(To *)out = __builtin_convertvector(*(const From *)in, To);              \
  }

K(i32_f32, v4si, v4sf)
K(f32_i32, v4sf, v4si)
K(u32_f32, v4su, v4sf)
K(f32_u32, v4sf, v4su)
K(f64_i64, v2df, v2di)
K(i64_f64, v2di, v2df)
K(u64_f64, v2du, v2df)
K(f64_u64, v2df, v2du)
K(s16_u16, v8hi, v8hu)
K(s8_u8, v16qi, v16qu)
K(f64_f32_2, v2df, v2sf)
K(f32_f64_2, v2sf, v2df)
K(i32_i16, v4si, v4hi)
K(i16_i32, v4hi, v4si)
K(u8_i32, v8qu, v8si)
K(s8_f32, v8qi, v8sf)
K(f32_f64, v4sf, v4df)
K(f64_f32, v4df, v4sf)
K(i32_f64, v4si, v4df)
K(f64_i32, v4df, v4si)
K(u32_i64, v4su, v4di)
K(i64_i32, v4di, v4si)
K(i32_i32, v4si, v4si)
K(i32_u32, v4si, v4su)
K(i32_f32_2, v2si, v2sf)
K(f32_i32_2, v2sf, v2si)
