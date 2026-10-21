/* ext_vector_type layout and lane-operation matrix (P0.1 receipts).
 *
 * The `sizeof`/alignment contract of `ext_vector_type` is Clang's, and it is
 * *target dependent*: `unsigned long` is 32 bits on i686 and 64 on LP64, the
 * `long double` lane occupies 10 significant bytes of x87 state whatever its
 * `sizeof` is, and three-lane vectors are padded to a power-of-two element
 * count.  Rounding those byte counts, or assuming the LP64 answer on i686, is
 * exactly the class of defect this file exists to catch, so the fixture never
 * names a byte count: it prints what the compiler used and the gate compares
 * that against Clang 23.1.3 for the same target.
 *
 * Three independent properties are exercised per shape:
 *
 *   1. compile-time layout laws, as `_Static_assert`s that must hold for every
 *      accepted shape on both targets (storage is a multiple of alignment,
 *      alignment is a power of two, element alignment is honoured, and the
 *      power-of-two padding never loses a non-bool lane);
 *   2. the runtime `sizeof`/`_Alignof` pair the compiler actually emitted;
 *   3. lane stores and lane loads through the vector, folded into a hash, so a
 *      storage size that is self-consistent for `sizeof` but wrong for lane
 *      addressing (a lane written into the padding, an element read at the
 *      wrong stride) cannot hide behind a matching size.
 *
 * The fixture is freestanding by design: it declares `printf` itself and
 * includes no header, so the i686 leg needs only a 32-bit crt and libc to
 * link.  A fixture that needs the target's glibc development headers cannot
 * run in a minimal sandbox, which is how the i686 half of this contract stayed
 * unverified outside the hosted runners.
 */
int printf(const char *, ...);

/* Every accepted shape: 12 element types x 10 lane counts (1..8, 11, 16).
 * The odd and non-power-of-two counts are the padding cases. */
typedef _Bool v_b1_1 __attribute__((ext_vector_type(1)));
typedef _Bool v_b1_2 __attribute__((ext_vector_type(2)));
typedef _Bool v_b1_3 __attribute__((ext_vector_type(3)));
typedef _Bool v_b1_4 __attribute__((ext_vector_type(4)));
typedef _Bool v_b1_5 __attribute__((ext_vector_type(5)));
typedef _Bool v_b1_6 __attribute__((ext_vector_type(6)));
typedef _Bool v_b1_7 __attribute__((ext_vector_type(7)));
typedef _Bool v_b1_8 __attribute__((ext_vector_type(8)));
typedef _Bool v_b1_11 __attribute__((ext_vector_type(11)));
typedef _Bool v_b1_16 __attribute__((ext_vector_type(16)));

typedef signed char v_i8_1 __attribute__((ext_vector_type(1)));
typedef signed char v_i8_2 __attribute__((ext_vector_type(2)));
typedef signed char v_i8_3 __attribute__((ext_vector_type(3)));
typedef signed char v_i8_4 __attribute__((ext_vector_type(4)));
typedef signed char v_i8_5 __attribute__((ext_vector_type(5)));
typedef signed char v_i8_6 __attribute__((ext_vector_type(6)));
typedef signed char v_i8_7 __attribute__((ext_vector_type(7)));
typedef signed char v_i8_8 __attribute__((ext_vector_type(8)));
typedef signed char v_i8_11 __attribute__((ext_vector_type(11)));
typedef signed char v_i8_16 __attribute__((ext_vector_type(16)));

typedef unsigned char v_u8_1 __attribute__((ext_vector_type(1)));
typedef unsigned char v_u8_2 __attribute__((ext_vector_type(2)));
typedef unsigned char v_u8_3 __attribute__((ext_vector_type(3)));
typedef unsigned char v_u8_4 __attribute__((ext_vector_type(4)));
typedef unsigned char v_u8_5 __attribute__((ext_vector_type(5)));
typedef unsigned char v_u8_6 __attribute__((ext_vector_type(6)));
typedef unsigned char v_u8_7 __attribute__((ext_vector_type(7)));
typedef unsigned char v_u8_8 __attribute__((ext_vector_type(8)));
typedef unsigned char v_u8_11 __attribute__((ext_vector_type(11)));
typedef unsigned char v_u8_16 __attribute__((ext_vector_type(16)));

typedef short v_i16_1 __attribute__((ext_vector_type(1)));
typedef short v_i16_2 __attribute__((ext_vector_type(2)));
typedef short v_i16_3 __attribute__((ext_vector_type(3)));
typedef short v_i16_4 __attribute__((ext_vector_type(4)));
typedef short v_i16_5 __attribute__((ext_vector_type(5)));
typedef short v_i16_6 __attribute__((ext_vector_type(6)));
typedef short v_i16_7 __attribute__((ext_vector_type(7)));
typedef short v_i16_8 __attribute__((ext_vector_type(8)));
typedef short v_i16_11 __attribute__((ext_vector_type(11)));
typedef short v_i16_16 __attribute__((ext_vector_type(16)));

typedef unsigned short v_u16_1 __attribute__((ext_vector_type(1)));
typedef unsigned short v_u16_2 __attribute__((ext_vector_type(2)));
typedef unsigned short v_u16_3 __attribute__((ext_vector_type(3)));
typedef unsigned short v_u16_4 __attribute__((ext_vector_type(4)));
typedef unsigned short v_u16_5 __attribute__((ext_vector_type(5)));
typedef unsigned short v_u16_6 __attribute__((ext_vector_type(6)));
typedef unsigned short v_u16_7 __attribute__((ext_vector_type(7)));
typedef unsigned short v_u16_8 __attribute__((ext_vector_type(8)));
typedef unsigned short v_u16_11 __attribute__((ext_vector_type(11)));
typedef unsigned short v_u16_16 __attribute__((ext_vector_type(16)));

typedef int v_i32_1 __attribute__((ext_vector_type(1)));
typedef int v_i32_2 __attribute__((ext_vector_type(2)));
typedef int v_i32_3 __attribute__((ext_vector_type(3)));
typedef int v_i32_4 __attribute__((ext_vector_type(4)));
typedef int v_i32_5 __attribute__((ext_vector_type(5)));
typedef int v_i32_6 __attribute__((ext_vector_type(6)));
typedef int v_i32_7 __attribute__((ext_vector_type(7)));
typedef int v_i32_8 __attribute__((ext_vector_type(8)));
typedef int v_i32_11 __attribute__((ext_vector_type(11)));
typedef int v_i32_16 __attribute__((ext_vector_type(16)));

typedef unsigned int v_u32_1 __attribute__((ext_vector_type(1)));
typedef unsigned int v_u32_2 __attribute__((ext_vector_type(2)));
typedef unsigned int v_u32_3 __attribute__((ext_vector_type(3)));
typedef unsigned int v_u32_4 __attribute__((ext_vector_type(4)));
typedef unsigned int v_u32_5 __attribute__((ext_vector_type(5)));
typedef unsigned int v_u32_6 __attribute__((ext_vector_type(6)));
typedef unsigned int v_u32_7 __attribute__((ext_vector_type(7)));
typedef unsigned int v_u32_8 __attribute__((ext_vector_type(8)));
typedef unsigned int v_u32_11 __attribute__((ext_vector_type(11)));
typedef unsigned int v_u32_16 __attribute__((ext_vector_type(16)));

typedef long v_i64_1 __attribute__((ext_vector_type(1)));
typedef long v_i64_2 __attribute__((ext_vector_type(2)));
typedef long v_i64_3 __attribute__((ext_vector_type(3)));
typedef long v_i64_4 __attribute__((ext_vector_type(4)));
typedef long v_i64_5 __attribute__((ext_vector_type(5)));
typedef long v_i64_6 __attribute__((ext_vector_type(6)));
typedef long v_i64_7 __attribute__((ext_vector_type(7)));
typedef long v_i64_8 __attribute__((ext_vector_type(8)));
typedef long v_i64_11 __attribute__((ext_vector_type(11)));
typedef long v_i64_16 __attribute__((ext_vector_type(16)));

typedef unsigned long v_u64_1 __attribute__((ext_vector_type(1)));
typedef unsigned long v_u64_2 __attribute__((ext_vector_type(2)));
typedef unsigned long v_u64_3 __attribute__((ext_vector_type(3)));
typedef unsigned long v_u64_4 __attribute__((ext_vector_type(4)));
typedef unsigned long v_u64_5 __attribute__((ext_vector_type(5)));
typedef unsigned long v_u64_6 __attribute__((ext_vector_type(6)));
typedef unsigned long v_u64_7 __attribute__((ext_vector_type(7)));
typedef unsigned long v_u64_8 __attribute__((ext_vector_type(8)));
typedef unsigned long v_u64_11 __attribute__((ext_vector_type(11)));
typedef unsigned long v_u64_16 __attribute__((ext_vector_type(16)));

typedef float v_f32_1 __attribute__((ext_vector_type(1)));
typedef float v_f32_2 __attribute__((ext_vector_type(2)));
typedef float v_f32_3 __attribute__((ext_vector_type(3)));
typedef float v_f32_4 __attribute__((ext_vector_type(4)));
typedef float v_f32_5 __attribute__((ext_vector_type(5)));
typedef float v_f32_6 __attribute__((ext_vector_type(6)));
typedef float v_f32_7 __attribute__((ext_vector_type(7)));
typedef float v_f32_8 __attribute__((ext_vector_type(8)));
typedef float v_f32_11 __attribute__((ext_vector_type(11)));
typedef float v_f32_16 __attribute__((ext_vector_type(16)));

typedef double v_f64_1 __attribute__((ext_vector_type(1)));
typedef double v_f64_2 __attribute__((ext_vector_type(2)));
typedef double v_f64_3 __attribute__((ext_vector_type(3)));
typedef double v_f64_4 __attribute__((ext_vector_type(4)));
typedef double v_f64_5 __attribute__((ext_vector_type(5)));
typedef double v_f64_6 __attribute__((ext_vector_type(6)));
typedef double v_f64_7 __attribute__((ext_vector_type(7)));
typedef double v_f64_8 __attribute__((ext_vector_type(8)));
typedef double v_f64_11 __attribute__((ext_vector_type(11)));
typedef double v_f64_16 __attribute__((ext_vector_type(16)));

typedef long double v_f80_1 __attribute__((ext_vector_type(1)));
typedef long double v_f80_2 __attribute__((ext_vector_type(2)));
typedef long double v_f80_3 __attribute__((ext_vector_type(3)));
typedef long double v_f80_4 __attribute__((ext_vector_type(4)));
typedef long double v_f80_5 __attribute__((ext_vector_type(5)));
typedef long double v_f80_6 __attribute__((ext_vector_type(6)));
typedef long double v_f80_7 __attribute__((ext_vector_type(7)));
typedef long double v_f80_8 __attribute__((ext_vector_type(8)));
typedef long double v_f80_11 __attribute__((ext_vector_type(11)));
typedef long double v_f80_16 __attribute__((ext_vector_type(16)));

/* Non-bool lane storage is at least `lanes * sizeof(elem)`: the padding may
 * add bytes, never drop a lane.  `_Bool` lanes are bit-packed, so the storage
 * is bounded by one byte per lane instead.  A `long double` lane occupies 10
 * bytes of x87 state on both x86 targets. */
#define LANES_AT_LEAST(T, N, SZ) \
    _Static_assert(sizeof(T) >= (N) * (SZ), #T " lost a lane");
#define BOOL_PACKED(T, N) \
    _Static_assert(sizeof(T) >= 1 && sizeof(T) <= (N), #T " is not bit-packed");
#define ALIGN_LAW(T, ELEM) \
    _Static_assert(_Alignof(T) >= _Alignof(ELEM), #T " under-aligns its element"); \
    _Static_assert(sizeof(T) % _Alignof(T) == 0, \
                   #T " storage is not a multiple of alignment"); \
    _Static_assert((_Alignof(T) & (_Alignof(T) - 1)) == 0, \
                   #T " alignment is not a power of two");

BOOL_PACKED(v_b1_1, 1)
ALIGN_LAW(v_b1_1, _Bool)
BOOL_PACKED(v_b1_2, 2)
ALIGN_LAW(v_b1_2, _Bool)
BOOL_PACKED(v_b1_3, 3)
ALIGN_LAW(v_b1_3, _Bool)
BOOL_PACKED(v_b1_4, 4)
ALIGN_LAW(v_b1_4, _Bool)
BOOL_PACKED(v_b1_5, 5)
ALIGN_LAW(v_b1_5, _Bool)
BOOL_PACKED(v_b1_6, 6)
ALIGN_LAW(v_b1_6, _Bool)
BOOL_PACKED(v_b1_7, 7)
ALIGN_LAW(v_b1_7, _Bool)
BOOL_PACKED(v_b1_8, 8)
ALIGN_LAW(v_b1_8, _Bool)
BOOL_PACKED(v_b1_11, 11)
ALIGN_LAW(v_b1_11, _Bool)
BOOL_PACKED(v_b1_16, 16)
ALIGN_LAW(v_b1_16, _Bool)

LANES_AT_LEAST(v_i8_1, 1, sizeof(signed char))
ALIGN_LAW(v_i8_1, signed char)
LANES_AT_LEAST(v_i8_2, 2, sizeof(signed char))
ALIGN_LAW(v_i8_2, signed char)
LANES_AT_LEAST(v_i8_3, 3, sizeof(signed char))
ALIGN_LAW(v_i8_3, signed char)
LANES_AT_LEAST(v_i8_4, 4, sizeof(signed char))
ALIGN_LAW(v_i8_4, signed char)
LANES_AT_LEAST(v_i8_5, 5, sizeof(signed char))
ALIGN_LAW(v_i8_5, signed char)
LANES_AT_LEAST(v_i8_6, 6, sizeof(signed char))
ALIGN_LAW(v_i8_6, signed char)
LANES_AT_LEAST(v_i8_7, 7, sizeof(signed char))
ALIGN_LAW(v_i8_7, signed char)
LANES_AT_LEAST(v_i8_8, 8, sizeof(signed char))
ALIGN_LAW(v_i8_8, signed char)
LANES_AT_LEAST(v_i8_11, 11, sizeof(signed char))
ALIGN_LAW(v_i8_11, signed char)
LANES_AT_LEAST(v_i8_16, 16, sizeof(signed char))
ALIGN_LAW(v_i8_16, signed char)

LANES_AT_LEAST(v_u8_1, 1, sizeof(unsigned char))
ALIGN_LAW(v_u8_1, unsigned char)
LANES_AT_LEAST(v_u8_2, 2, sizeof(unsigned char))
ALIGN_LAW(v_u8_2, unsigned char)
LANES_AT_LEAST(v_u8_3, 3, sizeof(unsigned char))
ALIGN_LAW(v_u8_3, unsigned char)
LANES_AT_LEAST(v_u8_4, 4, sizeof(unsigned char))
ALIGN_LAW(v_u8_4, unsigned char)
LANES_AT_LEAST(v_u8_5, 5, sizeof(unsigned char))
ALIGN_LAW(v_u8_5, unsigned char)
LANES_AT_LEAST(v_u8_6, 6, sizeof(unsigned char))
ALIGN_LAW(v_u8_6, unsigned char)
LANES_AT_LEAST(v_u8_7, 7, sizeof(unsigned char))
ALIGN_LAW(v_u8_7, unsigned char)
LANES_AT_LEAST(v_u8_8, 8, sizeof(unsigned char))
ALIGN_LAW(v_u8_8, unsigned char)
LANES_AT_LEAST(v_u8_11, 11, sizeof(unsigned char))
ALIGN_LAW(v_u8_11, unsigned char)
LANES_AT_LEAST(v_u8_16, 16, sizeof(unsigned char))
ALIGN_LAW(v_u8_16, unsigned char)

LANES_AT_LEAST(v_i16_1, 1, sizeof(short))
ALIGN_LAW(v_i16_1, short)
LANES_AT_LEAST(v_i16_2, 2, sizeof(short))
ALIGN_LAW(v_i16_2, short)
LANES_AT_LEAST(v_i16_3, 3, sizeof(short))
ALIGN_LAW(v_i16_3, short)
LANES_AT_LEAST(v_i16_4, 4, sizeof(short))
ALIGN_LAW(v_i16_4, short)
LANES_AT_LEAST(v_i16_5, 5, sizeof(short))
ALIGN_LAW(v_i16_5, short)
LANES_AT_LEAST(v_i16_6, 6, sizeof(short))
ALIGN_LAW(v_i16_6, short)
LANES_AT_LEAST(v_i16_7, 7, sizeof(short))
ALIGN_LAW(v_i16_7, short)
LANES_AT_LEAST(v_i16_8, 8, sizeof(short))
ALIGN_LAW(v_i16_8, short)
LANES_AT_LEAST(v_i16_11, 11, sizeof(short))
ALIGN_LAW(v_i16_11, short)
LANES_AT_LEAST(v_i16_16, 16, sizeof(short))
ALIGN_LAW(v_i16_16, short)

LANES_AT_LEAST(v_u16_1, 1, sizeof(unsigned short))
ALIGN_LAW(v_u16_1, unsigned short)
LANES_AT_LEAST(v_u16_2, 2, sizeof(unsigned short))
ALIGN_LAW(v_u16_2, unsigned short)
LANES_AT_LEAST(v_u16_3, 3, sizeof(unsigned short))
ALIGN_LAW(v_u16_3, unsigned short)
LANES_AT_LEAST(v_u16_4, 4, sizeof(unsigned short))
ALIGN_LAW(v_u16_4, unsigned short)
LANES_AT_LEAST(v_u16_5, 5, sizeof(unsigned short))
ALIGN_LAW(v_u16_5, unsigned short)
LANES_AT_LEAST(v_u16_6, 6, sizeof(unsigned short))
ALIGN_LAW(v_u16_6, unsigned short)
LANES_AT_LEAST(v_u16_7, 7, sizeof(unsigned short))
ALIGN_LAW(v_u16_7, unsigned short)
LANES_AT_LEAST(v_u16_8, 8, sizeof(unsigned short))
ALIGN_LAW(v_u16_8, unsigned short)
LANES_AT_LEAST(v_u16_11, 11, sizeof(unsigned short))
ALIGN_LAW(v_u16_11, unsigned short)
LANES_AT_LEAST(v_u16_16, 16, sizeof(unsigned short))
ALIGN_LAW(v_u16_16, unsigned short)

LANES_AT_LEAST(v_i32_1, 1, sizeof(int))
ALIGN_LAW(v_i32_1, int)
LANES_AT_LEAST(v_i32_2, 2, sizeof(int))
ALIGN_LAW(v_i32_2, int)
LANES_AT_LEAST(v_i32_3, 3, sizeof(int))
ALIGN_LAW(v_i32_3, int)
LANES_AT_LEAST(v_i32_4, 4, sizeof(int))
ALIGN_LAW(v_i32_4, int)
LANES_AT_LEAST(v_i32_5, 5, sizeof(int))
ALIGN_LAW(v_i32_5, int)
LANES_AT_LEAST(v_i32_6, 6, sizeof(int))
ALIGN_LAW(v_i32_6, int)
LANES_AT_LEAST(v_i32_7, 7, sizeof(int))
ALIGN_LAW(v_i32_7, int)
LANES_AT_LEAST(v_i32_8, 8, sizeof(int))
ALIGN_LAW(v_i32_8, int)
LANES_AT_LEAST(v_i32_11, 11, sizeof(int))
ALIGN_LAW(v_i32_11, int)
LANES_AT_LEAST(v_i32_16, 16, sizeof(int))
ALIGN_LAW(v_i32_16, int)

LANES_AT_LEAST(v_u32_1, 1, sizeof(unsigned int))
ALIGN_LAW(v_u32_1, unsigned int)
LANES_AT_LEAST(v_u32_2, 2, sizeof(unsigned int))
ALIGN_LAW(v_u32_2, unsigned int)
LANES_AT_LEAST(v_u32_3, 3, sizeof(unsigned int))
ALIGN_LAW(v_u32_3, unsigned int)
LANES_AT_LEAST(v_u32_4, 4, sizeof(unsigned int))
ALIGN_LAW(v_u32_4, unsigned int)
LANES_AT_LEAST(v_u32_5, 5, sizeof(unsigned int))
ALIGN_LAW(v_u32_5, unsigned int)
LANES_AT_LEAST(v_u32_6, 6, sizeof(unsigned int))
ALIGN_LAW(v_u32_6, unsigned int)
LANES_AT_LEAST(v_u32_7, 7, sizeof(unsigned int))
ALIGN_LAW(v_u32_7, unsigned int)
LANES_AT_LEAST(v_u32_8, 8, sizeof(unsigned int))
ALIGN_LAW(v_u32_8, unsigned int)
LANES_AT_LEAST(v_u32_11, 11, sizeof(unsigned int))
ALIGN_LAW(v_u32_11, unsigned int)
LANES_AT_LEAST(v_u32_16, 16, sizeof(unsigned int))
ALIGN_LAW(v_u32_16, unsigned int)

LANES_AT_LEAST(v_i64_1, 1, sizeof(long))
ALIGN_LAW(v_i64_1, long)
LANES_AT_LEAST(v_i64_2, 2, sizeof(long))
ALIGN_LAW(v_i64_2, long)
LANES_AT_LEAST(v_i64_3, 3, sizeof(long))
ALIGN_LAW(v_i64_3, long)
LANES_AT_LEAST(v_i64_4, 4, sizeof(long))
ALIGN_LAW(v_i64_4, long)
LANES_AT_LEAST(v_i64_5, 5, sizeof(long))
ALIGN_LAW(v_i64_5, long)
LANES_AT_LEAST(v_i64_6, 6, sizeof(long))
ALIGN_LAW(v_i64_6, long)
LANES_AT_LEAST(v_i64_7, 7, sizeof(long))
ALIGN_LAW(v_i64_7, long)
LANES_AT_LEAST(v_i64_8, 8, sizeof(long))
ALIGN_LAW(v_i64_8, long)
LANES_AT_LEAST(v_i64_11, 11, sizeof(long))
ALIGN_LAW(v_i64_11, long)
LANES_AT_LEAST(v_i64_16, 16, sizeof(long))
ALIGN_LAW(v_i64_16, long)

LANES_AT_LEAST(v_u64_1, 1, sizeof(unsigned long))
ALIGN_LAW(v_u64_1, unsigned long)
LANES_AT_LEAST(v_u64_2, 2, sizeof(unsigned long))
ALIGN_LAW(v_u64_2, unsigned long)
LANES_AT_LEAST(v_u64_3, 3, sizeof(unsigned long))
ALIGN_LAW(v_u64_3, unsigned long)
LANES_AT_LEAST(v_u64_4, 4, sizeof(unsigned long))
ALIGN_LAW(v_u64_4, unsigned long)
LANES_AT_LEAST(v_u64_5, 5, sizeof(unsigned long))
ALIGN_LAW(v_u64_5, unsigned long)
LANES_AT_LEAST(v_u64_6, 6, sizeof(unsigned long))
ALIGN_LAW(v_u64_6, unsigned long)
LANES_AT_LEAST(v_u64_7, 7, sizeof(unsigned long))
ALIGN_LAW(v_u64_7, unsigned long)
LANES_AT_LEAST(v_u64_8, 8, sizeof(unsigned long))
ALIGN_LAW(v_u64_8, unsigned long)
LANES_AT_LEAST(v_u64_11, 11, sizeof(unsigned long))
ALIGN_LAW(v_u64_11, unsigned long)
LANES_AT_LEAST(v_u64_16, 16, sizeof(unsigned long))
ALIGN_LAW(v_u64_16, unsigned long)

LANES_AT_LEAST(v_f32_1, 1, sizeof(float))
ALIGN_LAW(v_f32_1, float)
LANES_AT_LEAST(v_f32_2, 2, sizeof(float))
ALIGN_LAW(v_f32_2, float)
LANES_AT_LEAST(v_f32_3, 3, sizeof(float))
ALIGN_LAW(v_f32_3, float)
LANES_AT_LEAST(v_f32_4, 4, sizeof(float))
ALIGN_LAW(v_f32_4, float)
LANES_AT_LEAST(v_f32_5, 5, sizeof(float))
ALIGN_LAW(v_f32_5, float)
LANES_AT_LEAST(v_f32_6, 6, sizeof(float))
ALIGN_LAW(v_f32_6, float)
LANES_AT_LEAST(v_f32_7, 7, sizeof(float))
ALIGN_LAW(v_f32_7, float)
LANES_AT_LEAST(v_f32_8, 8, sizeof(float))
ALIGN_LAW(v_f32_8, float)
LANES_AT_LEAST(v_f32_11, 11, sizeof(float))
ALIGN_LAW(v_f32_11, float)
LANES_AT_LEAST(v_f32_16, 16, sizeof(float))
ALIGN_LAW(v_f32_16, float)

LANES_AT_LEAST(v_f64_1, 1, sizeof(double))
ALIGN_LAW(v_f64_1, double)
LANES_AT_LEAST(v_f64_2, 2, sizeof(double))
ALIGN_LAW(v_f64_2, double)
LANES_AT_LEAST(v_f64_3, 3, sizeof(double))
ALIGN_LAW(v_f64_3, double)
LANES_AT_LEAST(v_f64_4, 4, sizeof(double))
ALIGN_LAW(v_f64_4, double)
LANES_AT_LEAST(v_f64_5, 5, sizeof(double))
ALIGN_LAW(v_f64_5, double)
LANES_AT_LEAST(v_f64_6, 6, sizeof(double))
ALIGN_LAW(v_f64_6, double)
LANES_AT_LEAST(v_f64_7, 7, sizeof(double))
ALIGN_LAW(v_f64_7, double)
LANES_AT_LEAST(v_f64_8, 8, sizeof(double))
ALIGN_LAW(v_f64_8, double)
LANES_AT_LEAST(v_f64_11, 11, sizeof(double))
ALIGN_LAW(v_f64_11, double)
LANES_AT_LEAST(v_f64_16, 16, sizeof(double))
ALIGN_LAW(v_f64_16, double)

LANES_AT_LEAST(v_f80_1, 1, 10)
ALIGN_LAW(v_f80_1, long double)
LANES_AT_LEAST(v_f80_2, 2, 10)
ALIGN_LAW(v_f80_2, long double)
LANES_AT_LEAST(v_f80_3, 3, 10)
ALIGN_LAW(v_f80_3, long double)
LANES_AT_LEAST(v_f80_4, 4, 10)
ALIGN_LAW(v_f80_4, long double)
LANES_AT_LEAST(v_f80_5, 5, 10)
ALIGN_LAW(v_f80_5, long double)
LANES_AT_LEAST(v_f80_6, 6, 10)
ALIGN_LAW(v_f80_6, long double)
LANES_AT_LEAST(v_f80_7, 7, 10)
ALIGN_LAW(v_f80_7, long double)
LANES_AT_LEAST(v_f80_8, 8, 10)
ALIGN_LAW(v_f80_8, long double)
LANES_AT_LEAST(v_f80_11, 11, 10)
ALIGN_LAW(v_f80_11, long double)
LANES_AT_LEAST(v_f80_16, 16, 10)
ALIGN_LAW(v_f80_16, long double)

static void layout(void) {
    printf("b1 1 %lu %lu\n", (unsigned long)sizeof(v_b1_1), (unsigned long)_Alignof(v_b1_1));
    printf("b1 2 %lu %lu\n", (unsigned long)sizeof(v_b1_2), (unsigned long)_Alignof(v_b1_2));
    printf("b1 3 %lu %lu\n", (unsigned long)sizeof(v_b1_3), (unsigned long)_Alignof(v_b1_3));
    printf("b1 4 %lu %lu\n", (unsigned long)sizeof(v_b1_4), (unsigned long)_Alignof(v_b1_4));
    printf("b1 5 %lu %lu\n", (unsigned long)sizeof(v_b1_5), (unsigned long)_Alignof(v_b1_5));
    printf("b1 6 %lu %lu\n", (unsigned long)sizeof(v_b1_6), (unsigned long)_Alignof(v_b1_6));
    printf("b1 7 %lu %lu\n", (unsigned long)sizeof(v_b1_7), (unsigned long)_Alignof(v_b1_7));
    printf("b1 8 %lu %lu\n", (unsigned long)sizeof(v_b1_8), (unsigned long)_Alignof(v_b1_8));
    printf("b1 11 %lu %lu\n", (unsigned long)sizeof(v_b1_11), (unsigned long)_Alignof(v_b1_11));
    printf("b1 16 %lu %lu\n", (unsigned long)sizeof(v_b1_16), (unsigned long)_Alignof(v_b1_16));
    printf("i8 1 %lu %lu\n", (unsigned long)sizeof(v_i8_1), (unsigned long)_Alignof(v_i8_1));
    printf("i8 2 %lu %lu\n", (unsigned long)sizeof(v_i8_2), (unsigned long)_Alignof(v_i8_2));
    printf("i8 3 %lu %lu\n", (unsigned long)sizeof(v_i8_3), (unsigned long)_Alignof(v_i8_3));
    printf("i8 4 %lu %lu\n", (unsigned long)sizeof(v_i8_4), (unsigned long)_Alignof(v_i8_4));
    printf("i8 5 %lu %lu\n", (unsigned long)sizeof(v_i8_5), (unsigned long)_Alignof(v_i8_5));
    printf("i8 6 %lu %lu\n", (unsigned long)sizeof(v_i8_6), (unsigned long)_Alignof(v_i8_6));
    printf("i8 7 %lu %lu\n", (unsigned long)sizeof(v_i8_7), (unsigned long)_Alignof(v_i8_7));
    printf("i8 8 %lu %lu\n", (unsigned long)sizeof(v_i8_8), (unsigned long)_Alignof(v_i8_8));
    printf("i8 11 %lu %lu\n", (unsigned long)sizeof(v_i8_11), (unsigned long)_Alignof(v_i8_11));
    printf("i8 16 %lu %lu\n", (unsigned long)sizeof(v_i8_16), (unsigned long)_Alignof(v_i8_16));
    printf("u8 1 %lu %lu\n", (unsigned long)sizeof(v_u8_1), (unsigned long)_Alignof(v_u8_1));
    printf("u8 2 %lu %lu\n", (unsigned long)sizeof(v_u8_2), (unsigned long)_Alignof(v_u8_2));
    printf("u8 3 %lu %lu\n", (unsigned long)sizeof(v_u8_3), (unsigned long)_Alignof(v_u8_3));
    printf("u8 4 %lu %lu\n", (unsigned long)sizeof(v_u8_4), (unsigned long)_Alignof(v_u8_4));
    printf("u8 5 %lu %lu\n", (unsigned long)sizeof(v_u8_5), (unsigned long)_Alignof(v_u8_5));
    printf("u8 6 %lu %lu\n", (unsigned long)sizeof(v_u8_6), (unsigned long)_Alignof(v_u8_6));
    printf("u8 7 %lu %lu\n", (unsigned long)sizeof(v_u8_7), (unsigned long)_Alignof(v_u8_7));
    printf("u8 8 %lu %lu\n", (unsigned long)sizeof(v_u8_8), (unsigned long)_Alignof(v_u8_8));
    printf("u8 11 %lu %lu\n", (unsigned long)sizeof(v_u8_11), (unsigned long)_Alignof(v_u8_11));
    printf("u8 16 %lu %lu\n", (unsigned long)sizeof(v_u8_16), (unsigned long)_Alignof(v_u8_16));
    printf("i16 1 %lu %lu\n", (unsigned long)sizeof(v_i16_1), (unsigned long)_Alignof(v_i16_1));
    printf("i16 2 %lu %lu\n", (unsigned long)sizeof(v_i16_2), (unsigned long)_Alignof(v_i16_2));
    printf("i16 3 %lu %lu\n", (unsigned long)sizeof(v_i16_3), (unsigned long)_Alignof(v_i16_3));
    printf("i16 4 %lu %lu\n", (unsigned long)sizeof(v_i16_4), (unsigned long)_Alignof(v_i16_4));
    printf("i16 5 %lu %lu\n", (unsigned long)sizeof(v_i16_5), (unsigned long)_Alignof(v_i16_5));
    printf("i16 6 %lu %lu\n", (unsigned long)sizeof(v_i16_6), (unsigned long)_Alignof(v_i16_6));
    printf("i16 7 %lu %lu\n", (unsigned long)sizeof(v_i16_7), (unsigned long)_Alignof(v_i16_7));
    printf("i16 8 %lu %lu\n", (unsigned long)sizeof(v_i16_8), (unsigned long)_Alignof(v_i16_8));
    printf("i16 11 %lu %lu\n", (unsigned long)sizeof(v_i16_11), (unsigned long)_Alignof(v_i16_11));
    printf("i16 16 %lu %lu\n", (unsigned long)sizeof(v_i16_16), (unsigned long)_Alignof(v_i16_16));
    printf("u16 1 %lu %lu\n", (unsigned long)sizeof(v_u16_1), (unsigned long)_Alignof(v_u16_1));
    printf("u16 2 %lu %lu\n", (unsigned long)sizeof(v_u16_2), (unsigned long)_Alignof(v_u16_2));
    printf("u16 3 %lu %lu\n", (unsigned long)sizeof(v_u16_3), (unsigned long)_Alignof(v_u16_3));
    printf("u16 4 %lu %lu\n", (unsigned long)sizeof(v_u16_4), (unsigned long)_Alignof(v_u16_4));
    printf("u16 5 %lu %lu\n", (unsigned long)sizeof(v_u16_5), (unsigned long)_Alignof(v_u16_5));
    printf("u16 6 %lu %lu\n", (unsigned long)sizeof(v_u16_6), (unsigned long)_Alignof(v_u16_6));
    printf("u16 7 %lu %lu\n", (unsigned long)sizeof(v_u16_7), (unsigned long)_Alignof(v_u16_7));
    printf("u16 8 %lu %lu\n", (unsigned long)sizeof(v_u16_8), (unsigned long)_Alignof(v_u16_8));
    printf("u16 11 %lu %lu\n", (unsigned long)sizeof(v_u16_11), (unsigned long)_Alignof(v_u16_11));
    printf("u16 16 %lu %lu\n", (unsigned long)sizeof(v_u16_16), (unsigned long)_Alignof(v_u16_16));
    printf("i32 1 %lu %lu\n", (unsigned long)sizeof(v_i32_1), (unsigned long)_Alignof(v_i32_1));
    printf("i32 2 %lu %lu\n", (unsigned long)sizeof(v_i32_2), (unsigned long)_Alignof(v_i32_2));
    printf("i32 3 %lu %lu\n", (unsigned long)sizeof(v_i32_3), (unsigned long)_Alignof(v_i32_3));
    printf("i32 4 %lu %lu\n", (unsigned long)sizeof(v_i32_4), (unsigned long)_Alignof(v_i32_4));
    printf("i32 5 %lu %lu\n", (unsigned long)sizeof(v_i32_5), (unsigned long)_Alignof(v_i32_5));
    printf("i32 6 %lu %lu\n", (unsigned long)sizeof(v_i32_6), (unsigned long)_Alignof(v_i32_6));
    printf("i32 7 %lu %lu\n", (unsigned long)sizeof(v_i32_7), (unsigned long)_Alignof(v_i32_7));
    printf("i32 8 %lu %lu\n", (unsigned long)sizeof(v_i32_8), (unsigned long)_Alignof(v_i32_8));
    printf("i32 11 %lu %lu\n", (unsigned long)sizeof(v_i32_11), (unsigned long)_Alignof(v_i32_11));
    printf("i32 16 %lu %lu\n", (unsigned long)sizeof(v_i32_16), (unsigned long)_Alignof(v_i32_16));
    printf("u32 1 %lu %lu\n", (unsigned long)sizeof(v_u32_1), (unsigned long)_Alignof(v_u32_1));
    printf("u32 2 %lu %lu\n", (unsigned long)sizeof(v_u32_2), (unsigned long)_Alignof(v_u32_2));
    printf("u32 3 %lu %lu\n", (unsigned long)sizeof(v_u32_3), (unsigned long)_Alignof(v_u32_3));
    printf("u32 4 %lu %lu\n", (unsigned long)sizeof(v_u32_4), (unsigned long)_Alignof(v_u32_4));
    printf("u32 5 %lu %lu\n", (unsigned long)sizeof(v_u32_5), (unsigned long)_Alignof(v_u32_5));
    printf("u32 6 %lu %lu\n", (unsigned long)sizeof(v_u32_6), (unsigned long)_Alignof(v_u32_6));
    printf("u32 7 %lu %lu\n", (unsigned long)sizeof(v_u32_7), (unsigned long)_Alignof(v_u32_7));
    printf("u32 8 %lu %lu\n", (unsigned long)sizeof(v_u32_8), (unsigned long)_Alignof(v_u32_8));
    printf("u32 11 %lu %lu\n", (unsigned long)sizeof(v_u32_11), (unsigned long)_Alignof(v_u32_11));
    printf("u32 16 %lu %lu\n", (unsigned long)sizeof(v_u32_16), (unsigned long)_Alignof(v_u32_16));
    printf("i64 1 %lu %lu\n", (unsigned long)sizeof(v_i64_1), (unsigned long)_Alignof(v_i64_1));
    printf("i64 2 %lu %lu\n", (unsigned long)sizeof(v_i64_2), (unsigned long)_Alignof(v_i64_2));
    printf("i64 3 %lu %lu\n", (unsigned long)sizeof(v_i64_3), (unsigned long)_Alignof(v_i64_3));
    printf("i64 4 %lu %lu\n", (unsigned long)sizeof(v_i64_4), (unsigned long)_Alignof(v_i64_4));
    printf("i64 5 %lu %lu\n", (unsigned long)sizeof(v_i64_5), (unsigned long)_Alignof(v_i64_5));
    printf("i64 6 %lu %lu\n", (unsigned long)sizeof(v_i64_6), (unsigned long)_Alignof(v_i64_6));
    printf("i64 7 %lu %lu\n", (unsigned long)sizeof(v_i64_7), (unsigned long)_Alignof(v_i64_7));
    printf("i64 8 %lu %lu\n", (unsigned long)sizeof(v_i64_8), (unsigned long)_Alignof(v_i64_8));
    printf("i64 11 %lu %lu\n", (unsigned long)sizeof(v_i64_11), (unsigned long)_Alignof(v_i64_11));
    printf("i64 16 %lu %lu\n", (unsigned long)sizeof(v_i64_16), (unsigned long)_Alignof(v_i64_16));
    printf("u64 1 %lu %lu\n", (unsigned long)sizeof(v_u64_1), (unsigned long)_Alignof(v_u64_1));
    printf("u64 2 %lu %lu\n", (unsigned long)sizeof(v_u64_2), (unsigned long)_Alignof(v_u64_2));
    printf("u64 3 %lu %lu\n", (unsigned long)sizeof(v_u64_3), (unsigned long)_Alignof(v_u64_3));
    printf("u64 4 %lu %lu\n", (unsigned long)sizeof(v_u64_4), (unsigned long)_Alignof(v_u64_4));
    printf("u64 5 %lu %lu\n", (unsigned long)sizeof(v_u64_5), (unsigned long)_Alignof(v_u64_5));
    printf("u64 6 %lu %lu\n", (unsigned long)sizeof(v_u64_6), (unsigned long)_Alignof(v_u64_6));
    printf("u64 7 %lu %lu\n", (unsigned long)sizeof(v_u64_7), (unsigned long)_Alignof(v_u64_7));
    printf("u64 8 %lu %lu\n", (unsigned long)sizeof(v_u64_8), (unsigned long)_Alignof(v_u64_8));
    printf("u64 11 %lu %lu\n", (unsigned long)sizeof(v_u64_11), (unsigned long)_Alignof(v_u64_11));
    printf("u64 16 %lu %lu\n", (unsigned long)sizeof(v_u64_16), (unsigned long)_Alignof(v_u64_16));
    printf("f32 1 %lu %lu\n", (unsigned long)sizeof(v_f32_1), (unsigned long)_Alignof(v_f32_1));
    printf("f32 2 %lu %lu\n", (unsigned long)sizeof(v_f32_2), (unsigned long)_Alignof(v_f32_2));
    printf("f32 3 %lu %lu\n", (unsigned long)sizeof(v_f32_3), (unsigned long)_Alignof(v_f32_3));
    printf("f32 4 %lu %lu\n", (unsigned long)sizeof(v_f32_4), (unsigned long)_Alignof(v_f32_4));
    printf("f32 5 %lu %lu\n", (unsigned long)sizeof(v_f32_5), (unsigned long)_Alignof(v_f32_5));
    printf("f32 6 %lu %lu\n", (unsigned long)sizeof(v_f32_6), (unsigned long)_Alignof(v_f32_6));
    printf("f32 7 %lu %lu\n", (unsigned long)sizeof(v_f32_7), (unsigned long)_Alignof(v_f32_7));
    printf("f32 8 %lu %lu\n", (unsigned long)sizeof(v_f32_8), (unsigned long)_Alignof(v_f32_8));
    printf("f32 11 %lu %lu\n", (unsigned long)sizeof(v_f32_11), (unsigned long)_Alignof(v_f32_11));
    printf("f32 16 %lu %lu\n", (unsigned long)sizeof(v_f32_16), (unsigned long)_Alignof(v_f32_16));
    printf("f64 1 %lu %lu\n", (unsigned long)sizeof(v_f64_1), (unsigned long)_Alignof(v_f64_1));
    printf("f64 2 %lu %lu\n", (unsigned long)sizeof(v_f64_2), (unsigned long)_Alignof(v_f64_2));
    printf("f64 3 %lu %lu\n", (unsigned long)sizeof(v_f64_3), (unsigned long)_Alignof(v_f64_3));
    printf("f64 4 %lu %lu\n", (unsigned long)sizeof(v_f64_4), (unsigned long)_Alignof(v_f64_4));
    printf("f64 5 %lu %lu\n", (unsigned long)sizeof(v_f64_5), (unsigned long)_Alignof(v_f64_5));
    printf("f64 6 %lu %lu\n", (unsigned long)sizeof(v_f64_6), (unsigned long)_Alignof(v_f64_6));
    printf("f64 7 %lu %lu\n", (unsigned long)sizeof(v_f64_7), (unsigned long)_Alignof(v_f64_7));
    printf("f64 8 %lu %lu\n", (unsigned long)sizeof(v_f64_8), (unsigned long)_Alignof(v_f64_8));
    printf("f64 11 %lu %lu\n", (unsigned long)sizeof(v_f64_11), (unsigned long)_Alignof(v_f64_11));
    printf("f64 16 %lu %lu\n", (unsigned long)sizeof(v_f64_16), (unsigned long)_Alignof(v_f64_16));
    printf("f80 1 %lu %lu\n", (unsigned long)sizeof(v_f80_1), (unsigned long)_Alignof(v_f80_1));
    printf("f80 2 %lu %lu\n", (unsigned long)sizeof(v_f80_2), (unsigned long)_Alignof(v_f80_2));
    printf("f80 3 %lu %lu\n", (unsigned long)sizeof(v_f80_3), (unsigned long)_Alignof(v_f80_3));
    printf("f80 4 %lu %lu\n", (unsigned long)sizeof(v_f80_4), (unsigned long)_Alignof(v_f80_4));
    printf("f80 5 %lu %lu\n", (unsigned long)sizeof(v_f80_5), (unsigned long)_Alignof(v_f80_5));
    printf("f80 6 %lu %lu\n", (unsigned long)sizeof(v_f80_6), (unsigned long)_Alignof(v_f80_6));
    printf("f80 7 %lu %lu\n", (unsigned long)sizeof(v_f80_7), (unsigned long)_Alignof(v_f80_7));
    printf("f80 8 %lu %lu\n", (unsigned long)sizeof(v_f80_8), (unsigned long)_Alignof(v_f80_8));
    printf("f80 11 %lu %lu\n", (unsigned long)sizeof(v_f80_11), (unsigned long)_Alignof(v_f80_11));
    printf("f80 16 %lu %lu\n", (unsigned long)sizeof(v_f80_16), (unsigned long)_Alignof(v_f80_16));
}

/* Lane stores, then lane loads at every offset, for the odd/prime lane
 * counts (72 shapes), mixed into one hash.  A vector whose storage is one
 * element short reads a neighbour's byte here and the hash diverges. */
static long long lanes_hash(void) {
    long long h = 0;
    {
        v_b1_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (_Bool)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_b1_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (_Bool)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_b1_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (_Bool)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_b1_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (_Bool)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_b1_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (_Bool)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_b1_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (_Bool)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i8_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (signed char)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i8_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (signed char)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i8_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (signed char)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i8_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (signed char)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i8_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (signed char)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i8_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (signed char)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u8_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (unsigned char)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u8_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (unsigned char)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u8_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (unsigned char)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u8_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (unsigned char)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u8_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (unsigned char)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u8_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (unsigned char)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i16_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (short)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i16_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (short)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i16_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (short)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i16_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (short)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i16_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (short)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i16_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (short)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u16_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (unsigned short)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u16_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (unsigned short)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u16_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (unsigned short)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u16_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (unsigned short)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u16_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (unsigned short)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u16_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (unsigned short)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i32_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (int)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i32_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (int)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i32_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (int)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i32_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (int)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i32_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (int)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i32_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (int)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u32_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (unsigned int)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u32_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (unsigned int)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u32_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (unsigned int)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u32_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (unsigned int)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u32_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (unsigned int)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u32_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (unsigned int)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i64_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (long)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i64_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (long)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i64_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (long)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i64_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (long)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i64_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (long)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_i64_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (long)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u64_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (unsigned long)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u64_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (unsigned long)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u64_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (unsigned long)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u64_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (unsigned long)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u64_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (unsigned long)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_u64_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (unsigned long)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f32_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (float)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f32_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (float)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f32_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (float)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f32_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (float)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f32_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (float)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f32_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (float)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f64_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (double)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f64_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (double)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f64_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (double)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f64_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (double)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f64_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (double)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f64_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (double)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f80_2 v;
        for (int i = 0; i < 2; i++)
            v[i] = (long double)(i + 1);
        for (int i = 0; i < 2; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f80_3 v;
        for (int i = 0; i < 3; i++)
            v[i] = (long double)(i + 1);
        for (int i = 0; i < 3; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f80_5 v;
        for (int i = 0; i < 5; i++)
            v[i] = (long double)(i + 1);
        for (int i = 0; i < 5; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f80_6 v;
        for (int i = 0; i < 6; i++)
            v[i] = (long double)(i + 1);
        for (int i = 0; i < 6; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f80_7 v;
        for (int i = 0; i < 7; i++)
            v[i] = (long double)(i + 1);
        for (int i = 0; i < 7; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    {
        v_f80_11 v;
        for (int i = 0; i < 11; i++)
            v[i] = (long double)(i + 1);
        for (int i = 0; i < 11; i++)
            h = h * 31 + (long long)v[i];
        h = h * 31 + (long long)sizeof(v);
    }
    return h;
}

int main(void) {
    layout();
    printf("lanes %lld\n", lanes_hash());
    return 0;
}
