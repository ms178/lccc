#include <stdio.h>
struct S { int member; };
int main(void) {
    int i = 7;
    typeof(i) j = 8;
    typeof(&i) p = &i;
    typeof(*p) k = *p;
    typeof(i + j) m = i + j;
    int arr[2] = {3, 4};
    typeof(arr[0]) n = arr[1];
    struct S s = {5};
    typeof(s.member) t = s.member;
    __typeof__(t) u = t + 1;
    int r = ({ int q = 11; typeof(q) z = q + 1; z; });
    printf("%d %d %d %d %d %d %d\n", j, k, m, n, t, u, r);
    return 0;
}
