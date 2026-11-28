/* C11 6.2.1p7: a name enters scope after its declarator, before its
   initializer and later comma siblings. The GNU inferred-type exception has
   its own GCC-only fixture, declarator_inferred_scope.c. No uninitialized value is read.
   Inspired by sametyilmaztemel/claudes-c-compiler c6e8d717b8. */
typedef double FileScalar, (*FileFn)(FileScalar);
typedef int FileInt, FileArray[(FileInt)2.5 + 1];
typedef char T;
typedef char Aligned[16];
enum { Enum = 99 };

__attribute__((noinline)) static double plus_half(double x) { return x + 0.5; }

static int own_initializer(void) {
    int T = sizeof(T);
    return T == sizeof(int);
}
static int later_initializer(void) {
    int before = sizeof(T), T = 5, after = sizeof(T), difference = (T) - 1;
    return before == 1 && T == 5 && after == sizeof(int) && difference == 4;
}
static int static_initializer(void) {
    static int T = sizeof(T);
    return T == sizeof(int);
}
static int array_declarator(void) {
    typedef long long T;
    {
        char T[sizeof(T)];
        return sizeof(T) == sizeof(long long);
    }
}
static int restored_scope(void) {
    { long long T = sizeof(T); if (T != sizeof(long long)) return 0; }
    if (sizeof(T) != 1) return 0;
    { typedef volatile long long T; T x = 3; if (x != 3) return 0; }
    return sizeof(T) == 1;
}
static int sibling_typedef(void) {
    typedef double A, (*B)(A);
    B fn = plus_half;
    return fn(1.5) == 2.0;
}
static int typedef_array_size(void) {
    int Local = 9;
    {
        typedef char Local, Bytes[sizeof(Local)];
        return sizeof(Bytes) == 1;
    }
}
static int later_attribute(void) {
    int Aligned = 0, later __attribute__((aligned(sizeof(Aligned))));
    return __alignof__(later) == sizeof(int);
}
static int own_attribute(void) {
    int Aligned __attribute__((aligned(sizeof(Aligned)))) = 0;
    return __alignof__(Aligned) == 16;
}
static int typedef_attribute(void) {
    typedef long long Wide, Other __attribute__((aligned(sizeof(Wide))));
    return __alignof__(Other) == sizeof(long long);
}
static int for_initializer(void) {
    int result = 0;
    for (int T = sizeof(T); T == sizeof(int); ++T) result = T;
    return result == sizeof(int) && sizeof(T) == 1;
}
static int enum_shadow(void) {
    int Enum = 3, extent = Enum;
    int array[Enum];
    return extent == 3 && sizeof(array) == 3 * sizeof(int);
}

#define CHECK(expr) do { if (!(expr)) return __LINE__; } while (0)
int main(void) {
    FileFn fn = plus_half;
    CHECK(fn(1.5) == 2.0 && sizeof(FileArray) == 3 * sizeof(int));
    CHECK(own_initializer());
    CHECK(later_initializer());
    CHECK(static_initializer());
    CHECK(array_declarator());
    CHECK(restored_scope());
    CHECK(sibling_typedef());
    CHECK(typedef_array_size());
    CHECK(later_attribute());
    CHECK(own_attribute());
    CHECK(typedef_attribute());
    CHECK(for_initializer());
    CHECK(enum_shadow());
    CHECK(Enum == 99);
    return 0;
}
