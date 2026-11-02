/* gcc.c-torture/execute/20070919-1.c reduced.
 * Block-scope VLA structs need dynamic allocation and runtime-sized copies for
 * by-value assignment. Zero-byte alloca/memcpy left all copies empty.
 */
typedef __SIZE_TYPE__ size_t;
int memcmp(const void *, const void *, size_t);
void abort(void);
__attribute__((noinline)) void check(void *x) {
    struct S { char w[8]; } *q = x;
    if (memcmp(q[0].w, "abcdefg", 8)) abort();
    if (memcmp(q[1].w, "ABCDEFG", 8)) abort();
    if (memcmp(q[2].w, "zyxwvut", 8)) abort();
    if (memcmp(q[3].w, "zyxwvut", 8)) abort();
}
__attribute__((noinline)) void copy_vla_struct(void *x, int n) {
    struct S { char w[n]; } *p = x, tmp;
    tmp = p[2];
    p[3] = tmp;
    check(x);
}
int main(void) {
    struct S { char w[8]; } p[4] = { "abcdefg", "ABCDEFG", "zyxwvut", "ZYXWVUT" };
    copy_vla_struct(p, 8);
    return 0;
}
