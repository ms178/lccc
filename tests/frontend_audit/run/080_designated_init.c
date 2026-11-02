#include <stdio.h>
struct P { int x, y; char name[4]; };
struct Q { int a; struct P p; int b; };
#define PR(e) printf("%s = %d\n", #e, (int)(e))
int main(void) {
    int a[8] = { [3] = 7, [1] = 2, 9 };
    for (int i = 0; i < 8; i++) printf("%d ", a[i]); printf("\n");
    struct P p = { .y = 2, .x = 1, .name = "ab" };
    printf("%d %d %s\n", p.x, p.y, p.name);
    struct Q q = { .p.y = 5, .b = 6, .a = 4, .p.name = "zz" };
    printf("%d %d %d %s\n", q.a, q.p.x, q.p.y, q.p.name);
    struct Q q2 = { 1, { 2, 3, "cd" }, 4 };
    printf("%d %d %d %s\n", q2.a, q2.p.x, q2.p.y, q2.p.name);
    union U { int i; char c[4]; } u = { .c = { 1, 2, 3, 4 } };
    PR(u.i);
    int matrix[2][3] = { [1] = { 4, 5, 6 }, [0][1] = 2 };
    for (int i = 0; i < 2; i++) for (int j = 0; j < 3; j++) printf("%d ", matrix[i][j]);
    printf("\n");
    struct P *cp = &(struct P){ .x = 8, .y = 9 };
    printf("%d %d\n", cp->x, cp->y);
    int n = (int){ 42 };
    printf("%d\n", n);
    return 0;
}
