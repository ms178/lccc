/*
 * Focused regression for real memory access to an initialized long-double
 * ext-vector global. Keep this LCCC-only: GCC ignores ext_vector_type.
 * Clang 19.1.7 and ICX 2026.2 report sizeof/alignof(ld3) as 64/64 but emit
 * a 32-byte ELF object for this initializer -- that is not an oracle defect
 * but the layout itself: LLVM packs `<3 x x86_fp80>` at the 80-bit element
 * size (3 * 10 = 30 bytes, allocated as 32) inside the 64-byte C object, and
 * lccc now uses the same 10-byte lane stride (ABI-EXTVEC-LD-1).
 */
typedef long double ld3 __attribute__((ext_vector_type(3)));

_Static_assert(sizeof(ld3) == 64, "ld3 object size");
_Static_assert(_Alignof(ld3) == 64, "ld3 object alignment");

volatile ld3 global_ld3 = {1.25L, 2.5L, 3.75L};

__attribute__((noinline)) static int check_global(volatile ld3 *value) {
    long double lane0 = (*value)[0];
    long double lane1 = (*value)[1];
    long double lane2 = (*value)[2];
    return lane0 != 1.25L || lane1 != 2.5L || lane2 != 3.75L;
}

int main(void) {
    return check_global(&global_ld3);
}
