/* gcc.c-torture/execute/20040423-1.c and 20041218-2.c
 *
 * A local struct (or typedef of one) may contain a VLA member. sizeof of
 * that type is evaluated at the type definition; later stores to the bound
 * expression must not change it. */
void abort(void);

int sub1(int i, int j) {
    typedef struct {
        int c[i + 2];
    } c;
    int x[10], y[10];

    if (j == 2) {
        __builtin_memcpy(x, y, 10 * sizeof(int));
        return sizeof(c);
    } else {
        return sizeof(c) * 3;
    }
}

int test_packed(int n) {
    struct s {
        char b[n];
    } __attribute__((packed));
    n++;
    return sizeof(struct s);
}

int main(void) {
    typedef struct {
        int c[22];
    } c;
    if (sub1(20, 3) != sizeof(c) * 3)
        abort();
    if (test_packed(123) != 123)
        abort();
    return 0;
}
