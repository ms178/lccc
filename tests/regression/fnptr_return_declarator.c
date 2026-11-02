/* Function declarators returning function pointers (C89 "obscure" form).
 *
 * SQLite 3.53.4 amalgamation (fts3_hash.c) failed with:
 *   static int (*ftsHashFunction(int keyClass))(const void*,int){...}
 *   xHash = ftsHashFunction(pH->keyClass);
 *   incompatible pointer types (have 'int *' but expected
 *   'int (*)(void *, int)')
 *
 * Two defects combined:
 *  1. combine_declarator_parts() had no case for a parenthesized inner
 *     declarator that is itself a *function* declarator (inner ends with
 *     Function) plus an outer Function suffix, so the derived list came out
 *     as [Function(B), Pointer, Function(A)] instead of
 *     [Pointer, FunctionPointer(B), Function(A)].
 *  2. build_return_type() silently skipped FunctionPointer entries in the
 *     pre-Function prefix (`_ => {}`), truncating such return types to
 *     `int *`.
 *
 * Covers: plain form, address-of vs plain function returns, a second
 * signature via typedef, pointer-valued return type, and the nested
 * two-level form `int (*(*f(int))(...))(...)`.
 */

static int hashA(const void *p, int n) { return n + (p != 0); }
static int hashB(const void *p, int n) { return n - (p != 0); }

/* Plain form, both `&f` and `f` return styles. */
static int (*pick(int k))(const void *, int) {
    if (k == 1)
        return &hashA;
    else
        return hashB;
}

/* Second signature, assigned into a typedef'd pointer. */
typedef int (*cmp_t)(const void *, int, const void *, int);
static int cmp3(const void *a, int x, const void *b, int y) {
    return x * y + (a == b);
}
static int (*pick2(int k))(const void *, int, const void *, int) {
    (void)k;
    return cmp3;
}

/* Nested: function returning ptr-to-function returning ptr-to-function. */
static int addmul(const void *p, int n) {
    (void)p;
    return 3 * n;
}
static int (*inner_pick(const void *p, int n))(const void *, int) {
    (void)p;
    (void)n;
    return addmul;
}
static int (*(*outer_pick(int k))(const void *, int))(const void *, int) {
    (void)k;
    return inner_pick;
}

/* Returned function pointer whose own return type is a pointer. */
static char *idstr(int n) {
    static char b[2] = "x";
    (void)n;
    return b;
}
static char *(*pickstr(void))(int) { return idstr; }

int main(void) {
    int (*h)(const void *, int) = pick(1);
    int (*h2)(const void *, int) = pick(0);
    cmp_t c = pick2(0);
    int (*(*op)(const void *, int))(const void *, int) = outer_pick(5);
    int (*ip)(const void *, int) = op((void *)0, 2);
    char *(*ps)(int) = pickstr();

    /* 5 + 5 + 13 + 21 + 1 = 45 */
    int r = h((void *)0, 5) + h2((void *)0, 5) + c((void *)0, 3, (void *)0, 4)
        + ip((void *)0, 7) + (ps(0)[0] == 'x');
    return r == 45 ? 0 : 1;
}
