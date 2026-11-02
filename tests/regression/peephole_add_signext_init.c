/* Regression: the removed fuse_add_sign_extend peephole rewrote
 * `addl %X,%X; movslq %X,%DST` to `addl %X,%DSTd`, reading uninitialized
 * %DSTd and dropping the sign extension. expat reportProcessingInstruction
 * shape: double a loaded int, sign-extend, use as pointer offset. */
#include <stdio.h>
#include <string.h>
struct enc { int min_bytes; };
static const char *g_text;
static int __attribute__((noinline)) name_len(const char *p) {
    const char *s = p;
    while (*p == 'p' || *p == 'i') p++;
    return (int)(p - s);
}
static int __attribute__((noinline)) report(const struct enc *e, const char *start) {
    /* start += e->min_bytes * 2  — the addl/movslq shape */
    start += e->min_bytes * 2;
    int n = name_len(start);
    return n;
}
int main(void){
    struct enc e = { 1 };
    g_text = "<?pi unknown?>";
    int n = report(&e, g_text);
    printf("%d\n", n);
    return n == 2 ? 0 : 1;
}
