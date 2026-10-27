/* Legal qualifier depths and adjusted array parameters must retain semantics. */
static int chosen;
static void adjusted(const int values[2], const int *replacement) {
    values = replacement;
    chosen = values[0];
}
static void change_pointee(int *const p) { *p += 2; }
static _Bool nonnull(int *p) { return p; }
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
    return 0;
}
