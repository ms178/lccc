/* A discarded post-increment/decrement whose operand contains another
 * post-increment: `(p++)->w++;`. The outer result is unused, but the inner
 * `p++` still yields the OLD pointer, so the increment lands in arr[0].
 * lccc once let the discarded-result fast path leak into the operand, which
 * made the nested `p++` yield the new pointer (writing arr[1], or past the
 * end of the array for the last element). Each statement below is a discarded
 * expression statement; every form reaches the inc/dec lowering differently
 * (member, bitfield, subscript, dereference, vector lane, compound). */
#include <stdio.h>
#include <stdlib.h>

struct S { int a, w; };
struct B { unsigned lo : 3, bf : 5; };
typedef int V4 __attribute__((vector_size(16)));

static int failures;
#define CHECK(c) do { if (!(c)) { printf("FAIL line %d\n", __LINE__); failures++; } } while (0)

int main(void) {
    struct S s[3] = {{0}};
    struct S *p = s;
    (p++)->w++;
    (p++)->w--;
    (p++)->w += 5;
    CHECK(p == s + 3 && s[0].w == 1 && s[1].w == -1 && s[2].w == 5);
    printf("member %d %d %d\n", s[0].w, s[1].w, s[2].w);

    struct B b[2] = {{0}};
    struct B *q = b;
    (q++)->bf++;
    (q++)->bf--;
    CHECK(q == b + 2 && b[0].bf == 1 && b[1].bf == 31 && b[0].lo == 0 && b[1].lo == 0);
    printf("bitfield %u %u %u %u\n", b[0].lo, b[0].bf, b[1].lo, b[1].bf);

    int a[3] = {0}, i = 0;
    a[i++]++;
    a[i++]--;
    CHECK(i == 2 && a[0] == 1 && a[1] == -1 && a[2] == 0);
    printf("subscript %d %d %d\n", a[0], a[1], a[2]);

    int j = 0;
    s[j++].a++;
    s[j++].a--;
    CHECK(j == 2 && s[0].a == 1 && s[1].a == -1 && s[2].a == 0);
    printf("element %d %d %d\n", s[0].a, s[1].a, s[2].a);

    int *r = a;
    (*r++)++;
    (*r++)--;
    CHECK(r == a + 2 && a[0] == 2 && a[1] == -2 && a[2] == 0);
    printf("deref %d %d %d\n", a[0], a[1], a[2]);

    double d[2] = {0.5, 0.5}, *dp = d;
    (*dp++)++;
    CHECK(dp == d + 1 && d[0] == 1.5 && d[1] == 0.5);
    printf("double %g %g\n", d[0], d[1]);

    V4 v = {10, 20, 30, 40};
    int k = 1;
    v[k++]++;
    v[k++]--;
    CHECK(k == 3 && v[0] == 10 && v[1] == 21 && v[2] == 29 && v[3] == 40);
    printf("lane %d %d %d %d\n", v[0], v[1], v[2], v[3]);

    int *pa[2] = {&a[0], &a[1]}, **pp = pa;
    (**pp++)++;
    CHECK(pp == pa + 1 && a[0] == 3 && a[1] == -2);
    printf("double-deref %d %d\n", a[0], a[1]);

    printf("failures %d\n", failures);
    return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
