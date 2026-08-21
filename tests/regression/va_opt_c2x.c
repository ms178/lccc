/* C2x __VA_OPT__ selection, commas, nesting, and token paste. */
#define CALL(fn, ...) fn(1 __VA_OPT__(,) __VA_ARGS__)
#define PLUS(...) 10 __VA_OPT__(+ (__VA_ARGS__))
#define CAT(base, ...) base __VA_OPT__(## __VA_ARGS__)
#define NAMED(x, rest...) x __VA_OPT__(+ (rest))
static int one(int a) { return a; }
static int three(int a, int b, int c) { return a + b + c; }
int main(void) {
    int v = 4, v2 = 7;
    if (CALL(one) != 1) return 1;
    if (CALL(three, 2, 3) != 6) return 2;
    if (PLUS() != 10 || PLUS(2 + 3) != 15) return 3;
    if (CAT(v) != 4 || CAT(v, 2) != 7) return 4;
    if (NAMED(8) != 8 || NAMED(8, 4) != 12) return 5;
    return 0;
}
