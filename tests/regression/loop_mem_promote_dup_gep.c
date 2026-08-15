/* Regression: loop_memory_promote must treat a LOAD through a duplicate
 * GEP (same base+offset, different value id) as an alias of the promoted
 * location. The in-loop store is removed by promotion, so such a load
 * read stale memory: sqlite3FpDecode's `while(z[n-1]=='0') n--;` spun
 * forever (SELECT 1.5 hung). */
#include <stdio.h>
struct FpD { char sign; int n; int iDP; char *z; char zBuf[24]; };
static void __attribute__((noinline)) trim(struct FpD *p) {
    while (p->z[p->n - 1] == '0') {
        p->n--;
    }
}
int main(void){
    struct FpD d;
    d.n = 8;
    d.z = d.zBuf;
    __builtin_strcpy(d.zBuf, "15000000");
    trim(&d);
    printf("n=%d\n", d.n);
    return d.n == 2 ? 0 : 1;
}
