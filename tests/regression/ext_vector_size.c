/* ext_vector_type element widths in BOTH declaration and type-name paths.
 * On ILP32 long is 4 bytes; on LP64 long is 8. A chained typedef must
 * retain its underlying width when an attribute is applied to a type name.
 * Use runtime values: parser-time _Static_assert and lowering historically
 * followed different paths, and the former alone missed the wrong vector. */
typedef long v4l __attribute__((ext_vector_type(4)));
typedef unsigned long v4ul __attribute__((ext_vector_type(4)));
typedef long long v4ll __attribute__((ext_vector_type(4)));
typedef long base_long;
typedef base_long alias_long;
typedef alias_long v4alias __attribute__((ext_vector_type(4)));
typedef alias_long v_typedef_count __attribute__((ext_vector_type(sizeof(alias_long))));

int main(void) {
    if (sizeof(v4l) != 4 * sizeof(long)) return 1;
    if (sizeof((long __attribute__((ext_vector_type(4)))){0}) != 4 * sizeof(long)) return 2;
    if (sizeof((alias_long __attribute__((ext_vector_type(4)))){0}) != 4 * sizeof(long)) return 3;
    if (sizeof((long long __attribute__((ext_vector_type(4)))){0}) != 4 * sizeof(long long)) return 4;
    if (sizeof((unsigned long __attribute__((ext_vector_type(4)))){0}) != 4 * sizeof(unsigned long)) return 5;
    if (sizeof((float __attribute__((ext_vector_type(4)))){0}) != 4 * sizeof(float)) return 6;
    if (sizeof(v4ul) != 4 * sizeof(unsigned long)) return 7;
    if (sizeof(v4ll) != 4 * sizeof(long long)) return 8;
    if (sizeof(v4alias) != 4 * sizeof(long)) return 9;
    /* i686 scalar long double sizeof is 12, but vector storage lanes
       are 16 bytes; a generic scalar sizeof substitution is wrong here. */
    if (sizeof((long double __attribute__((ext_vector_type(4)))){0}) != 64) return 10;
    if (sizeof(v_typedef_count) != sizeof(alias_long) * sizeof(long)) return 11;
    if (sizeof((alias_long __attribute__((ext_vector_type(sizeof(alias_long))))){0})
        != sizeof(alias_long) * sizeof(long)) return 12;
    return 0;
}
