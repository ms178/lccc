/* Legal qualifier depths and adjusted array parameters must retain semantics. */
static int chosen;
static void adjusted(const int values[2], const int *replacement) {
    values = replacement;
    chosen = values[0];
}
static void change_pointee(int *const p) { *p += 2; }
static _Bool nonnull(int *p) { return p; }
typedef int *IntPtr;
typedef const int *ConstIntPtr;
struct PtrFields { const IntPtr p; const ConstIntPtr cp; };
int main(void) {
    const int values[2] = {13, 17};
    const int *p = values;
    const int **pp = &p;
    (*pp)++;
    if (**pp != 17) return 1;
    adjusted(values, &values[1]);
    if (chosen != 17 || p != &values[1]) return 2;
    int a = 3, b = 4;
    int *const pointers[2] = {&a, &b};
    *pointers[1] += 5;
    change_pointee(&a);
    if (a != 5 || b != 9) return 3;
    if (!nonnull(&a) || nonnull((int*)0)) return 4;
    const IntPtr alias = &a;
    const ConstIntPtr const_alias = &b;
    struct PtrFields fields = {&a, &b};
    if (_Generic(alias, int *: 1, const int *: 2, default: 3) != 1) return 5;
    if (_Generic(const_alias, int *: 1, const int *: 2, default: 3) != 2) return 6;
    if (_Generic(fields.p, int *: 1, const int *: 2, default: 3) != 1) return 7;
    if (_Generic(fields.cp, int *: 1, const int *: 2, default: 3) != 2) return 8;
    *alias += 1;
    *fields.p += 2;
    if (a != 8 || *fields.cp != 9) return 9;
    return 0;
}
