/* Regression: single-use vector-load sinking + VLFOLD-transparent deferral.
 *
 * `y[i] += a*x[i]`-shaped bodies at the default -ffp-contract=off used to
 * stage the `y[i]` stream through %ymm0 and a 32-byte spill slot because the
 * vectorizer emits every stream load at the top of the body, outside the
 * emitter's memory-fold window.  `passes/vec_load_sink.rs` moves each
 * single-use pure vector load directly before its consumer, and the deferred
 * store analysis treats such an elided load as transparent, giving GCC's
 * 3-instruction body: `vmulps mem, vaddps mem, vmovups`.
 *
 * Shapes covered: saxpy (F32x8), daxpy (F64x4 mul+add), addv (I32x8),
 * two-product map (fma3: both products need staging), reductions (sumi,
 * dot), an aliasing in-place update (no restrict), and a body followed by a
 * volatile store.  Every result is hashed bit-exactly and compared against
 * GCC by run_regression_suite.sh.  Kill switch: CCC_NO_VEC_LOAD_SINK=1.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
void saxpy(float * __restrict y, const float * __restrict x, float a, int n){ for(int i=0;i<n;i++) y[i]+=a*x[i]; }
void daxpy(double * __restrict y, const double * __restrict x, double a, int n){ for(int i=0;i<n;i++) y[i]=y[i]*a+x[i]; }
void addv(int * __restrict c,const int * __restrict a,const int * __restrict b,int n){ for(int i=0;i<n;i++) c[i]=a[i]+b[i]; }
void fma3(float * __restrict d,const float * __restrict a,const float * __restrict b,const float * __restrict c,int n){ for(int i=0;i<n;i++) d[i]=a[i]*b[i]+c[i]*d[i]; }
int sumi(const int *a, int n){ int s=0; for(int i=0;i<n;i++) s+=a[i]; return s; }
double dot(const double *a,const double *b,int n){ double s=0; for(int i=0;i<n;i++) s+=a[i]*b[i]; return s; }
void alias_inplace(float *y, int n){ for(int i=0;i<n;i++) y[i]=y[i]*2.0f+y[i]; }
volatile int sink;
void with_volatile(float * __restrict y,const float * __restrict x,int n){ for(int i=0;i<n;i++){ y[i]=y[i]+x[i]*3.0f; } sink=n; }

static unsigned fnv(unsigned h,const void*p,size_t n){const unsigned char*b=p;for(size_t i=0;i<n;i++){h^=b[i];h*=16777619u;}return h;}
int main(void){
  enum{N=1031}; static float y[N],x[N],d[N],a[N],b[N],c[N],y0[N],d0[N]; static double dy[N],dx[N],dy0[N]; static int ia[N],ib[N],ic[N];
  unsigned s=12345; for(int i=0;i<N;i++){ s=s*1103515245u+12345u; y0[i]=(s>>8)%97*0.5f; x[i]=(s>>4)%53*0.25f; d0[i]=(s>>3)%7; a[i]=(s>>5)%11; b[i]=(s>>6)%13; c[i]=(s>>7)%17; dy0[i]=(s>>9)%31*0.125; dx[i]=(s>>2)%29; ia[i]=(int)(s>>1); ib[i]=(int)(s>>3); }
  unsigned h=2166136261u; long is=0; double ds=0;
  for(int n=0;n<N;n+=7){
    memcpy(y,y0,sizeof y); saxpy(y,x,1.5f,n); h=fnv(h,y,sizeof y);
    memcpy(dy,dy0,sizeof dy); daxpy(dy,dx,0.75,n); h=fnv(h,dy,sizeof dy);
    memset(ic,0,sizeof ic); addv(ic,ia,ib,n); h=fnv(h,ic,sizeof ic);
    memcpy(d,d0,sizeof d); fma3(d,a,b,c,n); h=fnv(h,d,sizeof d);
    memcpy(y,y0,sizeof y); alias_inplace(y,n); h=fnv(h,y,sizeof y);
    memcpy(y,y0,sizeof y); with_volatile(y,x,n); h=fnv(h,y,sizeof y);
    is+=sumi(ia,n); ds+=dot(dx,dy0,n);
  }
  printf("%08x %ld %.17g %d\n",h,is,ds,sink);
  return 0;
}
