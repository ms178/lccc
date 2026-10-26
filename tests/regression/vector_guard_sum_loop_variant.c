/* Guarded I32 sums against the guard forms the vectorizer must classify:
 * loop-VARIANT guards (`x[i] > i`, `x[i] > y[i]`, `x[i] > (i & 7) * 100`,
 * `i < x[i]`, and `x[i] > *p` whose load may or may not be hoisted) must stay
 * scalar or be proven invariant — the masked composites read the guard once
 * per vector step; invariant run-time thresholds (hoisted preheader splat),
 * constants, `>=`/mirrored spellings, the zero guard, the widening siblings,
 * an unguarded widening sum and a loop pair sharing a threshold. Trip counts
 * 0..40 then every 97 up to 4098, base offsets 0..2. UB-free; output must
 * equal GCC's. Default flags: the SSE2 baseline (VecMaskedAddValueI32x4 /
 * VecWidenMaskedAddValueI32x4ToI64x2); the _avx2 sibling covers AVX2. */
#include <stdio.h>
#define N 4099
int a[N], b[N];
#define F __attribute__((noinline))
F int s_iv(const int *x,int n){int s=0;for(int i=0;i<n;i++) if(x[i]>i) s+=x[i];return s;}
F int s_b(const int *x,const int *y,int n){int s=0;for(int i=0;i<n;i++) if(x[i]>y[i]) s+=x[i];return s;}
F int s_m(const int *x,int n){int s=0;for(int i=0;i<n;i++) if(x[i]>(i&7)*100) s+=x[i];return s;}
F int s_lt(const int *x,int n){int s=0;for(int i=0;i<n;i++) if(i<x[i]) s+=x[i];return s;}
F long long w_iv(const int *x,int n){long long s=0;for(int i=0;i<n;i++) if(x[i]>i) s+=x[i];return s;}
F long long w_b(const int *x,const int *y,int n){long long s=0;for(int i=0;i<n;i++) if(x[i]>y[i]) s+=x[i];return s;}
F int s_t(const int *x,int n,int t){int s=0;for(int i=0;i<n;i++) if(x[i]>t) s+=x[i];return s;}
F long long w_t(const int *x,int n,int t){long long s=0;for(int i=0;i<n;i++) if(x[i]>t) s+=x[i];return s;}
F int s_ld(const int *x,const int *p,int n){int s=0;for(int i=0;i<n;i++) if(x[i]>*p) s+=x[i];return s;}
F int s_0(const int *x,int n){int s=0;for(int i=0;i<n;i++) if(x[i]>0) s+=x[i];return s;}
F int s_ge(const int *x,int n){int s=0;for(int i=0;i<n;i++) if(x[i]>=-7) s+=x[i];return s;}
F int s_k(const int *x,int n){int s=0;for(int i=0;i<n;i++) if(1000<x[i]) s+=x[i];return s;}
F long long w_k(const int *x,int n){long long s=0;for(int i=0;i<n;i++) if(x[i]>1000) s+=x[i];return s;}
F long long w_0(const int *x,int n){long long s=0;for(int i=0;i<n;i++) if(x[i]>0) s+=x[i];return s;}
F long long w_n(const int *x,int n){long long s=0;for(int i=0;i<n;i++) if(x[i]>-5000) s+=x[i];return s;}
F long long w_u(const int *x,int n){long long s=0;for(int i=0;i<n;i++) s+=x[i];return s;}
F long long w_two(const int *x,int n,int t){long long s=0,u=0;for(int i=0;i<n;i++) s+=x[i]; for(int i=0;i<n;i++) if(x[i]>t) u+=x[i]; return s*3+u;}
int main(void){unsigned h=12345;for(int i=0;i<N;i++){h=h*1103515245u+12345u;a[i]=(int)((h>>8)%9000)-1000;h=h*1103515245u+12345u;b[i]=(int)((h>>8)%9000)-1000;}
 a[5]=2147483647/8; a[6]=-2147483647/8; /* large magnitudes, still UB-free sums */
 unsigned long long acc=0;
 for(int n=0;n<N;n+=(n<40?1:97)) for(int o=0;o<3;o++){int m=n-o<0?0:n-o; int q=n%3000; int t=n-500;
  acc=acc*31+(unsigned)s_iv(a+o,m); acc=acc*31+(unsigned)s_b(a+o,b,m); acc=acc*31+(unsigned)s_m(a+o,m); acc=acc*31+(unsigned)s_lt(a+o,m);
  acc=acc*31+(unsigned long long)w_iv(a+o,m); acc=acc*31+(unsigned long long)w_b(a+o,b,m); acc=acc*31+(unsigned)s_t(a+o,m,t);
  acc=acc*31+(unsigned long long)w_t(a+o,m,1000-n); acc=acc*31+(unsigned)s_ld(a+o,&b[q],m);
  acc=acc*31+(unsigned)s_0(a+o,m); acc=acc*31+(unsigned)s_ge(a+o,m); acc=acc*31+(unsigned)s_k(a+o,m);
  acc=acc*31+(unsigned long long)w_k(a+o,m); acc=acc*31+(unsigned long long)w_0(a+o,m); acc=acc*31+(unsigned long long)w_n(a+o,m);
  acc=acc*31+(unsigned long long)w_u(a+o,m); acc=acc*31+(unsigned long long)w_two(a+o,m,t);}
 printf("%016llx\n",acc);return 0;}
