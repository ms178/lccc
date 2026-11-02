/* Audit replay: bottom-tested comparisons already fuse at upstream 08f4e1a.
 * Also pin the counterexample to LOOP-PREHEADER-3's proposed dominance rule:
 * the header dominating a dereference does NOT make it must-execute.
 */
#include <stdio.h>
#define NI __attribute__((noinline))
NI int dw_lt(int n) { int i=0; do { ++i; } while(i<n); return i; }
NI int dw_le(int n) { int i=0; do { ++i; } while(i<=n); return i; }
NI int dw_ne(int n) { int i=0; do { ++i; } while(i!=n); return i; }
NI int dw_step2(int n) { int i=0; do { i+=2; } while(i<n); return i; }
NI int dw_break(int n) { int i=0; for (;;) { ++i; if(i>=n)break; } return i; }
NI int guarded_load(const int *p, int n, int take) {
    int sum=0;
    for (int i=0; i<n; ++i) if(take) sum+=*p;
    return sum;
}
NI int early_exit_load(const int *p, int n, int stop) {
    int sum=0;
    for (int i=0; i<n; ++i) { if(i==stop)break; sum+=*p; }
    return sum;
}
int main(void) {
    for(int n=-32; n<=255; ++n) {
        int a=n>1?n:1, b=n>=0?n+1:1, c=n>2?((n+1)/2)*2:2;
        if(dw_lt(n)!=a || dw_le(n)!=b || dw_step2(n)!=c || dw_break(n)!=a) return 1;
        if(n>0 && dw_ne(n)!=n) return 2;
    }
    int v=7;
    if(guarded_load(0,17,0)!=0 || guarded_load(0,0,1)!=0 ||
       guarded_load(&v,17,1)!=119) return 3;
    if(early_exit_load(0,17,0)!=0 || early_exit_load(0,0,9)!=0 ||
       early_exit_load(&v,17,3)!=21) return 4;
    puts("audit_loop_contracts: OK");
    return 0;
}
