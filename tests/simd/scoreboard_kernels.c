/* SIMD auto-vectorization probe corpus (TSVC-style). One noinline function per
 * kernel; main() runs each REPS times and prints "<name> <checksum> <ns/elem>". */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <math.h>
#define N 4096
#define NOINL __attribute__((noinline))
typedef unsigned char u8; typedef unsigned short u16; typedef signed char s8;
static float fa[N+8] __attribute__((aligned(64))), fb[N+8] __attribute__((aligned(64))), fc[N+8] __attribute__((aligned(64))), fd[N+8] __attribute__((aligned(64)));
static double da[N+8] __attribute__((aligned(64))), db[N+8] __attribute__((aligned(64))), dc[N+8] __attribute__((aligned(64)));
static int ia[N+8] __attribute__((aligned(64))), ib[N+8] __attribute__((aligned(64))), ic[N+8] __attribute__((aligned(64))), idx[N+8] __attribute__((aligned(64)));
static short sa[N+8] __attribute__((aligned(64))), sb[N+8] __attribute__((aligned(64)));
static u8 ba[N+8] __attribute__((aligned(64))), bb[N+8] __attribute__((aligned(64))), bc[N+8] __attribute__((aligned(64)));
static float cplx[2*N+16] __attribute__((aligned(64))), cplx2[2*N+16] __attribute__((aligned(64))), cplxo[2*N+16] __attribute__((aligned(64)));

/* ---- float/double elementwise ---- */
NOINL void k_add_f32(void){ for(int i=0;i<N;i++) fa[i]=fb[i]+fc[i]; }
NOINL void k_saxpy(void){ for(int i=0;i<N;i++) fa[i]=2.5f*fb[i]+fa[i]; }
NOINL void k_daxpy(void){ for(int i=0;i<N;i++) da[i]=2.5*db[i]+da[i]; }
NOINL void k_fma3(void){ for(int i=0;i<N;i++) fa[i]=fb[i]*fc[i]+fd[i]; }
NOINL void k_sqrt_f32(void){ for(int i=0;i<N;i++) fa[i]=sqrtf(fb[i]); }
NOINL void k_f2i(void){ for(int i=0;i<N;i++) ia[i]=(int)fb[i]; }
NOINL void k_i2f(void){ for(int i=0;i<N;i++) fa[i]=(float)ib[i]; }
NOINL void k_f2d(void){ for(int i=0;i<N;i++) da[i]=(double)fb[i]; }
NOINL void k_stencil3(void){ for(int i=1;i<N-1;i++) fa[i]=fb[i-1]+fb[i]+fb[i+1]; }
/* ---- reductions ---- */
NOINL int k_sum_i32(void){ int s=0; for(int i=0;i<N;i++) s+=ib[i]; return s; }
NOINL int k_max_i32(void){ int m=-2147483647-1; for(int i=0;i<N;i++) if(ib[i]>m) m=ib[i]; return m; }
NOINL int k_dot_i32(void){ int s=0; for(int i=0;i<N;i++) s+=ib[i]*ic[i]; return s; }
NOINL float k_dot_f32_fast(void){ float s=0; for(int i=0;i<N;i++) s+=fb[i]*fc[i]; return s; }
NOINL double k_sum_f64_fast(void){ double s=0; for(int i=0;i<N;i++) s+=db[i]; return s; }
NOINL int k_sad_u8(void){ int s=0; for(int i=0;i<N;i++) s+= ba[i]>bb[i]? ba[i]-bb[i] : bb[i]-ba[i]; return s; }
NOINL int k_popcnt(void){ int s=0; for(int i=0;i<N;i++) s+=__builtin_popcount((unsigned)ib[i]); return s; }
NOINL int k_count_eq_u8(void){ int s=0; for(int i=0;i<N;i++) s+=(ba[i]==bb[i]); return s; }
/* ---- integer elementwise / widths ---- */
NOINL void k_add_i32(void){ for(int i=0;i<N;i++) ia[i]=ib[i]+ic[i]; }
NOINL void k_mul_i32(void){ for(int i=0;i<N;i++) ia[i]=ib[i]*ic[i]; }
NOINL void k_add_i16(void){ for(int i=0;i<N;i++) sa[i]=(short)(sb[i]+sa[i]); }
NOINL void k_add_u8(void){ for(int i=0;i<N;i++) bc[i]=(u8)(ba[i]+bb[i]); }
NOINL void k_satadd_u8(void){ for(int i=0;i<N;i++){ int t=ba[i]+bb[i]; bc[i]=(u8)(t>255?255:t);} }
NOINL void k_widen_u8_u16(void){ for(int i=0;i<N;i++) sa[i]=(short)(ba[i]*3); }
NOINL void k_narrow_i16_u8(void){ for(int i=0;i<N;i++) bc[i]=(u8)(sb[i]>>8); }
NOINL void k_shift_i32(void){ for(int i=0;i<N;i++) ia[i]=(int)(((unsigned)ib[i]<<3)^(unsigned)(ic[i]>>2)); }
/* ---- control flow ---- */
NOINL void k_abs_f32(void){ for(int i=0;i<N;i++) fa[i]=fb[i]>0?fb[i]:-fb[i]; }
NOINL void k_select_f32(void){ for(int i=0;i<N;i++) fa[i]=fc[i]<0?fb[i]:fd[i]; }
NOINL void k_min_i32(void){ for(int i=0;i<N;i++) ia[i]=ib[i]<ic[i]?ib[i]:ic[i]; }
NOINL void k_condstore(void){ for(int i=0;i<N;i++) if(fb[i]>0) fa[i]=fc[i]; }
NOINL void k_clamp_i32(void){ for(int i=0;i<N;i++){ int v=ib[i]; ia[i]= v<0?0:(v>255?255:v);} }
/* ---- memory access shapes ---- */
NOINL void k_stride2(void){ for(int i=0;i<N/2;i++) fa[i]=fb[2*i]; }
NOINL void k_reverse(void){ for(int i=0;i<N;i++) fa[i]=fb[N-1-i]; }
NOINL void k_gather(void){ for(int i=0;i<N;i++) fa[i]=fb[idx[i]]; }
NOINL void k_scatter(void){ for(int i=0;i<N;i++) fa[idx[i]]=fb[i]; }
NOINL void k_cmul(void){ for(int i=0;i<N;i++){ float ar=cplx[2*i],ai=cplx[2*i+1],br=cplx2[2*i],bi=cplx2[2*i+1]; cplxo[2*i]=ar*br-ai*bi; cplxo[2*i+1]=ar*bi+ai*br; } }
NOINL void k_aos_soa(void){ for(int i=0;i<N;i++){ fa[i]=cplx[2*i]; fb[i]=cplx[2*i+1]; } }
/* ---- aliasing / trip counts ---- */
NOINL void k_alias_ptr(float*a,const float*b,const float*c,int n){ for(int i=0;i<n;i++) a[i]=b[i]+c[i]; }
NOINL void k_restrict(float*restrict a,const float*restrict b,const float*restrict c,int n){ for(int i=0;i<n;i++) a[i]=b[i]+c[i]; }
NOINL void k_unknown_n(int n){ for(int i=0;i<n;i++) fa[i]=fb[i]*1.5f; }
/* ---- not vectorizable (controls: must stay correct) ---- */
NOINL void k_recur(void){ for(int i=1;i<N;i++) fa[i]=fa[i-1]*0.5f+fb[i]; }
NOINL void k_prefix_i32(void){ for(int i=1;i<N;i++) ia[i]=ia[i-1]+ib[i]; }
NOINL void k_overlap_dist1(void){ for(int i=0;i<N-1;i++) fa[i]=fa[i+1]+fb[i]; }
/* ---- matmul-style (the pass's design target) ---- */
static double M1[64][64], M2[64][64], M3[64][64];
NOINL void k_matmul(void){ for(int i=0;i<64;i++) for(int k=0;k<64;k++) for(int j=0;j<64;j++) M3[i][j]+=M1[i][k]*M2[k][j]; }

static double now(void){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return t.tv_sec*1e9+t.tv_nsec; }
static uint64_t cs_f(const float*p,int n){ uint64_t h=1469598103934665603ULL; for(int i=0;i<n;i++){ uint32_t u; memcpy(&u,&p[i],4); h=(h^u)*1099511628211ULL;} return h; }
static uint64_t cs_i(const int*p,int n){ uint64_t h=1469598103934665603ULL; for(int i=0;i<n;i++) h=(h^(uint32_t)p[i])*1099511628211ULL; return h; }
static uint64_t cs_s(const short*p,int n){ uint64_t h=1469598103934665603ULL; for(int i=0;i<n;i++) h=(h^(uint16_t)p[i])*1099511628211ULL; return h; }
static uint64_t cs_b(const u8*p,int n){ uint64_t h=1469598103934665603ULL; for(int i=0;i<n;i++) h=(h^p[i])*1099511628211ULL; return h; }
static uint64_t cs_d(const double*p,int n){ uint64_t h=1469598103934665603ULL; for(int i=0;i<n;i++){ uint64_t u; memcpy(&u,&p[i],8); h=(h^u)*1099511628211ULL;} return h; }
static void init(void){
  uint32_t s=12345; for(int i=0;i<N+8;i++){ s=s*1664525u+1013904223u;
    fb[i]=((int)(s>>8)%2001-1000)/37.0f; s=s*1664525u+1013904223u; fc[i]=((int)(s>>8)%2001-1000)/41.0f; s=s*1664525u+1013904223u; fd[i]=((int)(s>>8)%2001-1000)/43.0f; fa[i]=(float)(i%17);
    db[i]=fb[i]*0.5; da[i]=fc[i]; dc[i]=fd[i];
    s=s*1664525u+1013904223u; ib[i]=(int)(s>>4)%2000-1000; s=s*1664525u+1013904223u; ic[i]=(int)(s>>4)%2000-1000; ia[i]=i%13;
    s=s*1664525u+1013904223u; idx[i]=(s>>8)%N; sb[i]=(short)(s>>3); sa[i]=(short)i;
    s=s*1664525u+1013904223u; ba[i]=(u8)(s>>5); bb[i]=(u8)(s>>11); bc[i]=0; }
  for(int i=0;i<2*N+16;i++){ s=s*1664525u+1013904223u; cplx[i]=((int)(s>>8)%201-100)/13.0f; s=s*1664525u+1013904223u; cplx2[i]=((int)(s>>8)%201-100)/17.0f; }
  for(int i=0;i<64;i++) for(int j=0;j<64;j++){ M1[i][j]=(i*7+j)%11*0.25; M2[i][j]=(i+j*5)%13*0.125; M3[i][j]=0; }
}
#ifndef REPS
#define REPS 4000
#endif
#if REPS <= 0
#error REPS must be positive
#endif
#define MAT_REPS (REPS / 16 > 0 ? REPS / 16 : 1)
#define RUNV(NAME,CALL,CS) do{ init(); double t0=now(); for(int r=0;r<REPS;r++){ CALL; } double t1=now(); \
   printf("%s %016llx %.4f\n",NAME,(unsigned long long)(CS),(t1-t0)/REPS/N);}while(0)
#define RUNR(NAME,CALL) do{ init(); volatile uint32_t sink=0; double t0=now(); for(int r=0;r<REPS;r++){ sink+=(uint32_t)(CALL); } double t1=now(); \
   printf("%s %016llx %.4f\n",NAME,(unsigned long long)(uint32_t)sink,(t1-t0)/REPS/N);}while(0)
/* Preserve the complete FP reduction result; truncating to int hid errors. */
#define RUNFP(NAME,CALL,TYPE,CS) do{ init(); volatile TYPE sink=0; double t0=now(); for(int r=0;r<REPS;r++){ sink=(CALL); } double t1=now(); TYPE result=sink; \
   printf("%s %016llx %.4f\n",NAME,(unsigned long long)CS(&result,1),(t1-t0)/REPS/N);}while(0)
int main(void){
  RUNV("add_f32",k_add_f32(),cs_f(fa,N)); RUNV("saxpy",(init(),k_saxpy()),cs_f(fa,N)); RUNV("daxpy",k_daxpy(),cs_d(da,N));
  RUNV("fma3",k_fma3(),cs_f(fa,N)); RUNV("sqrt_f32",k_sqrt_f32(),cs_f(fa,N)); RUNV("f2i",k_f2i(),cs_i(ia,N)); RUNV("i2f",k_i2f(),cs_f(fa,N));
  RUNV("f2d",k_f2d(),cs_d(da,N)); RUNV("stencil3",k_stencil3(),cs_f(fa,N));
  RUNR("sum_i32",k_sum_i32()); RUNR("max_i32",k_max_i32()); RUNR("dot_i32",k_dot_i32());
  RUNFP("dot_f32_fast",k_dot_f32_fast(),float,cs_f); RUNFP("sum_f64_fast",k_sum_f64_fast(),double,cs_d);
  RUNR("sad_u8",k_sad_u8()); RUNR("popcnt",k_popcnt()); RUNR("count_eq_u8",k_count_eq_u8());
  RUNV("add_i32",k_add_i32(),cs_i(ia,N)); RUNV("mul_i32",k_mul_i32(),cs_i(ia,N)); RUNV("add_i16",k_add_i16(),cs_s(sa,N)); RUNV("add_u8",k_add_u8(),cs_b(bc,N));
  RUNV("satadd_u8",k_satadd_u8(),cs_b(bc,N)); RUNV("widen_u8_u16",k_widen_u8_u16(),cs_s(sa,N)); RUNV("narrow_i16_u8",k_narrow_i16_u8(),cs_b(bc,N));
  RUNV("shift_i32",k_shift_i32(),cs_i(ia,N));
  RUNV("abs_f32",k_abs_f32(),cs_f(fa,N)); RUNV("select_f32",k_select_f32(),cs_f(fa,N)); RUNV("min_i32",k_min_i32(),cs_i(ia,N));
  RUNV("condstore",k_condstore(),cs_f(fa,N)); RUNV("clamp_i32",k_clamp_i32(),cs_i(ia,N));
  RUNV("stride2",k_stride2(),cs_f(fa,N/2)); RUNV("reverse",k_reverse(),cs_f(fa,N)); RUNV("gather",k_gather(),cs_f(fa,N)); RUNV("scatter",k_scatter(),cs_f(fa,N));
  RUNV("cmul",k_cmul(),cs_f(cplxo,2*N)); RUNV("aos_soa",k_aos_soa(),cs_f(fa,N)+cs_f(fb,N));
  RUNV("alias_ptr",k_alias_ptr(fa,fb,fc,N),cs_f(fa,N)); RUNV("restrict",k_restrict(fa,fb,fc,N),cs_f(fa,N)); RUNV("unknown_n",k_unknown_n(N-3),cs_f(fa,N));
  RUNV("alias_overlap",k_alias_ptr(fa+1,fa,fb,N-1),cs_f(fa,N));
  RUNV("recur",k_recur(),cs_f(fa,N)); RUNV("prefix_i32",k_prefix_i32(),cs_i(ia,N)); RUNV("overlap_dist1",k_overlap_dist1(),cs_f(fa,N));
  { init(); double t0=now(); for(int r=0;r<MAT_REPS;r++) k_matmul(); double t1=now(); uint64_t h=1469598103934665603ULL; for(int row=0;row<64;row++) h=(h^cs_d(M3[row],64))*1099511628211ULL; printf("matmul %016llx %.4f\n",(unsigned long long)h,(t1-t0)/MAT_REPS/(64.0*64*64)); }
  return 0; }
