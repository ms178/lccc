/* Layout and lane semantics for Clang ext_vector_type vectors on x86. */
#include <stddef.h>

typedef _Bool bool4 __attribute__((ext_vector_type(4)));
typedef _Bool bool9 __attribute__((ext_vector_type(9)));
typedef _Bool bool17 __attribute__((ext_vector_type(17)));
typedef unsigned char u8x3 __attribute__((ext_vector_type(3)));
typedef unsigned int u32x3 __attribute__((ext_vector_type(3)));
typedef unsigned long ulong3 __attribute__((ext_vector_type(3)));
typedef long double ld3 __attribute__((ext_vector_type(3)));
typedef unsigned int u32x4_size __attribute__((vector_size(16)));

/* Declaration/type-name paths must agree, and lane count is not storage_size /
 * sizeof(element): ext vectors have target ABI padding, and bool lanes are
 * bit-packed. */
_Static_assert(sizeof(bool4) == 1, "four bool lanes occupy one byte");
_Static_assert(_Alignof(bool4) == 1, "bool4 alignment");
_Static_assert(sizeof(bool9) == 2, "nine bool lanes occupy two bytes");
_Static_assert(_Alignof(bool9) == 2, "bool9 alignment");
_Static_assert(sizeof(bool17) == 4, "seventeen bool lanes occupy four bytes");
_Static_assert(_Alignof(bool17) == 4, "bool17 alignment");
_Static_assert(sizeof(u8x3) == 4, "u8x3 is padded to four bytes");
_Static_assert(_Alignof(u8x3) == 4, "u8x3 alignment");
_Static_assert(sizeof(u32x3) == 16, "u32x3 is padded to sixteen bytes");
_Static_assert(_Alignof(u32x3) == 16, "u32x3 alignment");
_Static_assert(sizeof(ulong3) == (sizeof(long) == 4 ? 16 : 32), "ulong3 padded size");
_Static_assert(_Alignof(ulong3) == (sizeof(long) == 4 ? 16 : 32), "ulong3 alignment");
_Static_assert(sizeof(ld3) == 64, "long-double vector object size is 3*16 rounded to a power of two (lanes are 10 bytes apart inside it)");
_Static_assert(_Alignof(ld3) == 64, "long-double vector alignment");
_Static_assert(sizeof(u32x4_size) == 16, "vector_size retains its byte-size contract");
_Static_assert(_Alignof(u32x4_size) == 16, "vector_size alignment remains independent");
_Static_assert(sizeof(bool4[2]) == 2, "bool4 array uses the packed vector stride");
_Static_assert(_Alignof(bool4[2]) == 1, "bool4 array alignment follows its element");
_Static_assert(sizeof(u8x3[2]) == 8, "u8x3 array uses its padded four-byte stride");
_Static_assert(_Alignof(u8x3[2]) == 4, "u8x3 array alignment follows its element");
_Static_assert(sizeof(u32x3[2][2]) == 64, "matrix stride uses padded vector objects");
_Static_assert(_Alignof(u32x3[2][2]) == 16, "matrix alignment follows its vector element");
_Static_assert(sizeof(ld3[2]) == 128, "long-double vector array stride");
_Static_assert(_Alignof(ld3[2]) == 64, "long-double vector array alignment");

struct bool4_layout { char before; bool4 value; char after; };
struct bool9_layout { char before; bool9 value; char after; };
struct u32x3_layout { char before; u32x3 value; char after; };
struct ulong3_layout { char before; ulong3 value; char after; };
struct vector_global_fields { char before; bool4 bits; u32x3 values; };
_Static_assert(offsetof(struct bool4_layout, value) == 1, "bool4 field offset");
_Static_assert(offsetof(struct bool4_layout, after) == 2, "bool4 tail offset");
_Static_assert(sizeof(struct bool4_layout) == 3, "bool4 struct size");
_Static_assert(offsetof(struct bool9_layout, value) == 2, "bool9 field offset");
_Static_assert(offsetof(struct bool9_layout, after) == 4, "bool9 tail offset");
_Static_assert(sizeof(struct bool9_layout) == 6, "bool9 struct size");
_Static_assert(offsetof(struct u32x3_layout, value) == 16, "u32x3 field offset");
_Static_assert(offsetof(struct u32x3_layout, after) == 32, "u32x3 tail offset");
_Static_assert(sizeof(struct u32x3_layout) == 48, "u32x3 struct size");
_Static_assert(offsetof(struct ulong3_layout, value) == (sizeof(long) == 4 ? 16 : 32),
               "ulong3 struct field offset");
_Static_assert(offsetof(struct vector_global_fields, bits) == 1, "global bool field offset");
_Static_assert(offsetof(struct vector_global_fields, values) == 16, "global numeric field offset");
_Static_assert(sizeof(struct vector_global_fields) == 32, "global vector struct size");

static bool4 global_bool4 = {1, 0, 1, 1};
static bool9 global_bool9 = {1, 0, 1, 0, 1, 0, 1, 0, 1};
static bool17 global_bool17 = {1};
static u32x3 global_u32x3 = {11, 22, 33};
static u32x3 global_u32x3_pair[2] = {{1, 2, 3}, {4, 5, 6}};
static bool4 global_bool4_pair[2] = {{1, 0, 1, 0}, {0, 1, 0, 1}};
static u8x3 global_u8x3_pair[2] = {{1, 2, 3}, {4, 5, 6}};
static u32x3 global_u32x3_matrix[2][2] = {
    {{1, 2, 3}, {4, 5, 6}},
    {{7, 8, 9}, {10, 11, 12}},
};
static u32x3 global_u32x3_designated_matrix[2][2] = {
    [1][0] = {13, 14, 15},
    [0][1] = {16, 17, 18},
};
static u32x3 global_u32x3_compounds[2] = {
    (u32x3){19, 20, 21},
    (u32x3){22, 23, 24},
};
static ld3 global_ld3 = {1.25L, 2.5L, 3.75L};
static struct vector_global_fields global_vector_fields = {
    0x5a, {1, 0, 1, 1}, {7, 8, 9}
};

static u8x3 make_u8x3(unsigned char first) {
    return (u8x3){first, first + 1U, first + 2U};
}

int main(void) {
    /* Type-name path: runtime sizeof must match declaration layout. */
    if (sizeof((_Bool __attribute__((ext_vector_type(4)))){0}) != sizeof(bool4)) return 1;
    if (sizeof((unsigned int __attribute__((ext_vector_type(3)))){0}) != sizeof(u32x3)) return 2;

    /* Initializers, lane reads/writes, and object-representation packing. */
    bool4 local = {1, 0, 1, 1};
    if (((const unsigned char *)&local)[0] != 0x0d) return 3;
    if (local[0] != 1 || local[1] != 0 || local[2] != 1 || local[3] != 1) return 4;
    local[1] = 1;
    local[3] = 0;
    if (((const unsigned char *)&local)[0] != 0x07) return 5;
    if (local[0] != 1 || local[1] != 1 || local[2] != 1 || local[3] != 0) return 6;
    bool4 copied = local;
    if (((const unsigned char *)&copied)[0] != 0x07) return 7;

    if (((const unsigned char *)&global_bool4)[0] != 0x0d) return 8;
    if (((const unsigned char *)&global_bool9)[0] != 0x55 ||
        ((const unsigned char *)&global_bool9)[1] != 0x01) return 9;
    if (((const unsigned char *)&global_bool17)[0] != 0x01 ||
        ((const unsigned char *)&global_bool17)[1] != 0 ||
        ((const unsigned char *)&global_bool17)[2] != 0 ||
        ((const unsigned char *)&global_bool17)[3] != 0) return 10;
    if (global_bool9[8] != 1 || global_bool17[16] != 0) return 11;
    if (((const unsigned char *)&global_vector_fields)[1] != 0x0d) return 21;
    if (global_vector_fields.values[0] != 7 || global_vector_fields.values[1] != 8 ||
        global_vector_fields.values[2] != 9) return 22;
    if (global_bool4_pair[0][0] != 1 || global_bool4_pair[0][2] != 1 ||
        global_bool4_pair[1][0] != 0 || global_bool4_pair[1][1] != 1 ||
        ((const unsigned char *)&global_bool4_pair[1])[0] != 0x0a) return 23;
    if (global_u8x3_pair[0][2] != 3 || global_u8x3_pair[1][0] != 4 ||
        global_u8x3_pair[1][2] != 6 ||
        ((const unsigned char *)&global_u8x3_pair[1])[3] != 0) return 24;
    if (global_u32x3_matrix[0][1][2] != 6 ||
        global_u32x3_matrix[1][0][0] != 7 ||
        global_u32x3_matrix[1][1][2] != 12) return 25;
    if (global_u32x3_designated_matrix[0][0][0] != 0 ||
        global_u32x3_designated_matrix[0][1][2] != 18 ||
        global_u32x3_designated_matrix[1][0][0] != 13 ||
        global_u32x3_designated_matrix[1][1][0] != 0) return 26;
    if (global_u32x3_compounds[0][0] != 19 || global_u32x3_compounds[0][2] != 21 ||
        global_u32x3_compounds[1][0] != 22 || global_u32x3_compounds[1][2] != 24) return 27;
    if (global_ld3[0] != 1.25L || global_ld3[1] != 2.5L || global_ld3[2] != 3.75L) return 37;

    /* Logical lane count must remain three although storage is four/sixteen
     * bytes; vector operations must not process padded pseudo-lanes. */
    if (global_u32x3[0] != 11) return 31;
    if (global_u32x3[1] != 22) return 32;
    if (global_u32x3[2] != 33) return 33;
    if (global_u32x3_pair[1][0] != 4 || global_u32x3_pair[1][1] != 5 ||
        global_u32x3_pair[1][2] != 6) return 13;
    u32x3 a = {1, 2, 3};
    u32x3 b = {10, 20, 30};
    u32x3 sum = a + b;
    if (sum[0] != 11 || sum[1] != 22 || sum[2] != 33) return 14;

    /* Local arrays use the padded vector object stride, including nested
     * dimensions, designators, vector-expression copies, and small returns. */
    bool4 local_bool4_pair[2] = {{1, 1, 0, 0}, {0, 1, 0, 1}};
    if (((const unsigned char *)&local_bool4_pair[0])[0] != 0x03 ||
        ((const unsigned char *)&local_bool4_pair[1])[0] != 0x0a ||
        local_bool4_pair[1][3] != 1) return 28;
    u8x3 local_u8x3_pair[2] = {{7, 8, 9}, {10, 11, 12}};
    if (local_u8x3_pair[0][2] != 9 || local_u8x3_pair[1][0] != 10 ||
        local_u8x3_pair[1][2] != 12 ||
        ((const unsigned char *)&local_u8x3_pair[1])[3] != 0) return 29;
    u32x3 local_matrix[2][2] = {
        {{1, 2, 3}, {4, 5, 6}},
        {{7, 8, 9}, {10, 11, 12}},
    };
    if (local_matrix[0][1][2] != 6 || local_matrix[1][0][1] != 8 ||
        local_matrix[1][1][2] != 12) return 30;
    u32x3 local_designated_matrix[2][2] = {
        [1][0] = {31, 32, 33},
        [0][1] = {34, 35, 36},
    };
    if (local_designated_matrix[0][0][0] != 0 ||
        local_designated_matrix[0][1][2] != 36 ||
        local_designated_matrix[1][0][0] != 31 ||
        local_designated_matrix[1][1][0] != 0) return 34;
    u32x3 vector_copies[2] = {a, b};
    if (vector_copies[0][0] != 1 || vector_copies[0][2] != 3 ||
        vector_copies[1][0] != 10 || vector_copies[1][2] != 30) return 35;
    u8x3 return_copies[2] = {make_u8x3(40), make_u8x3(50)};
    if (return_copies[0][0] != 40 || return_copies[0][2] != 42 ||
        return_copies[1][0] != 50 || return_copies[1][2] != 52) return 36;
    ld3 local_ld3 = {4.5L, 5.5L, 6.5L};
    if (local_ld3[0] != 4.5L || local_ld3[1] != 5.5L || local_ld3[2] != 6.5L) return 38;

    /* Bitwise and comparison results are bool vectors and must also be packed. */
    bool4 p = {1, 0, 1, 0};
    bool4 q = {0, 1, 1, 0};
    bool4 xored = p ^ q;
    if (xored[0] != 1 || xored[1] != 1 || xored[2] != 0 || xored[3] != 0) return 15;
    if (((const unsigned char *)&xored)[0] != 0x03) return 16;
    bool4 equal = p == q;
    if (equal[0] != 0 || equal[1] != 0 || equal[2] != 1 || equal[3] != 1) return 17;
    if (((const unsigned char *)&equal)[0] != 0x0c) return 18;
    bool4 inverted = ~p;
    if (inverted[0] != 0 || inverted[1] != 1 || inverted[2] != 0 || inverted[3] != 1) return 19;
    if (((const unsigned char *)&inverted)[0] != 0x0a) return 20;

    return 0;
}


/* Compile-time-only shape checks that lock down the padded-storage and
 * bit-packing rules for shapes too large for the register-return ABI: bool
 * vectors of 9/17/33 lanes exercise the ceil(N/8) byte payload rule across
 * byte/half-word/word/dword boundaries, and s8x7 exercises the seven-lane
 * signed-char padding that the u8x3 rule generalizes to. These are
 * compile-time only so a backend that does not yet lower vectors wider than
 * 16 bytes still passes the gate. */
typedef _Bool bool33 __attribute__((ext_vector_type(33)));
typedef signed char s8x7 __attribute__((ext_vector_type(7)));
typedef unsigned short u16x3 __attribute__((ext_vector_type(3)));
/* The ceil(33/8) = 5-byte payload is padded to power-of-two storage, as for
 * every other shape (Clang 23.1.4: 8 on x86-64 and i686). This assertion
 * read 5 while the parser still accepted every sizeof it could not fold. */
_Static_assert(sizeof(bool33) == 8, "bool33 payload padded to eight-byte storage");
_Static_assert(_Alignof(bool33) == 8, "bool33 alignment follows next-power-of-two storage");
_Static_assert(sizeof(s8x7) == 8, "s8x7 padded to next power of two (7 -> 8)");
_Static_assert(_Alignof(s8x7) == 8, "s8x7 alignment is 8 bytes");
_Static_assert(sizeof(u16x3) == 8, "u16x3 is padded to eight bytes (3*2 -> 8)");
_Static_assert(_Alignof(u16x3) == 8, "u16x3 alignment is eight bytes");
