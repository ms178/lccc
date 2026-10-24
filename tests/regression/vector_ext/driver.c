#include <stdio.h>
#include <string.h>
#include <stdint.h>

typedef char v16c __attribute__((vector_size(16)));
typedef char v32c __attribute__((vector_size(32)));
typedef short v16s __attribute__((vector_size(16)));
typedef short v32s __attribute__((vector_size(32)));
typedef int v16i __attribute__((vector_size(16)));
typedef int v32i __attribute__((vector_size(32)));
typedef long v16l __attribute__((vector_size(16)));
typedef long v32l __attribute__((vector_size(32)));
typedef float v16f __attribute__((vector_size(16)));
typedef float v32f __attribute__((vector_size(32)));
typedef double v16d __attribute__((vector_size(16)));
typedef double v32d __attribute__((vector_size(32)));
struct S8f { v32f v; };
v16c G_add_v16c(v16c a,v16c b);
v16c G_sub_v16c(v16c a,v16c b);
v16c G_mul_v16c(v16c a,v16c b);
v16c G_and_v16c(v16c a,v16c b);
v16c G_or_v16c(v16c a,v16c b);
v16c G_xor_v16c(v16c a,v16c b);
v16c G_shl_v16c(v16c a,v16c b);
v16c G_shr_v16c(v16c a,v16c b);
v16c G_neg_v16c(v16c a,v16c b);
v16c G_not_v16c(v16c a,v16c b);
v16c G_eq_v16c(v16c a,v16c b);
v16c G_lt_v16c(v16c a,v16c b);
void G_loop_v16c(v16c*r,const v16c*a,const v16c*b,int n);
v16c G_splat_v16c(char x);
char G_lane2_v16c(v16c a);
v16c G_setlane_v16c(v16c a,char x);
v16c G_ld_v16c(const char*p);
void G_st_v16c(char*p,v16c v);
v16c G_cond_v16c(v16c a,v16c b,int c);
v16c G_acc_v16c(const v16c*p,int n);
v32c G_add_v32c(v32c a,v32c b);
v32c G_sub_v32c(v32c a,v32c b);
v32c G_mul_v32c(v32c a,v32c b);
v32c G_and_v32c(v32c a,v32c b);
v32c G_or_v32c(v32c a,v32c b);
v32c G_xor_v32c(v32c a,v32c b);
v32c G_shl_v32c(v32c a,v32c b);
v32c G_shr_v32c(v32c a,v32c b);
v32c G_neg_v32c(v32c a,v32c b);
v32c G_not_v32c(v32c a,v32c b);
v32c G_eq_v32c(v32c a,v32c b);
v32c G_lt_v32c(v32c a,v32c b);
void G_loop_v32c(v32c*r,const v32c*a,const v32c*b,int n);
v32c G_splat_v32c(char x);
char G_lane2_v32c(v32c a);
v32c G_setlane_v32c(v32c a,char x);
v32c G_ld_v32c(const char*p);
void G_st_v32c(char*p,v32c v);
v32c G_cond_v32c(v32c a,v32c b,int c);
v32c G_acc_v32c(const v32c*p,int n);
v16s G_add_v16s(v16s a,v16s b);
v16s G_sub_v16s(v16s a,v16s b);
v16s G_mul_v16s(v16s a,v16s b);
v16s G_and_v16s(v16s a,v16s b);
v16s G_or_v16s(v16s a,v16s b);
v16s G_xor_v16s(v16s a,v16s b);
v16s G_shl_v16s(v16s a,v16s b);
v16s G_shr_v16s(v16s a,v16s b);
v16s G_neg_v16s(v16s a,v16s b);
v16s G_not_v16s(v16s a,v16s b);
v16s G_eq_v16s(v16s a,v16s b);
v16s G_lt_v16s(v16s a,v16s b);
void G_loop_v16s(v16s*r,const v16s*a,const v16s*b,int n);
v16s G_splat_v16s(short x);
short G_lane2_v16s(v16s a);
v16s G_setlane_v16s(v16s a,short x);
v16s G_ld_v16s(const short*p);
void G_st_v16s(short*p,v16s v);
v16s G_cond_v16s(v16s a,v16s b,int c);
v16s G_acc_v16s(const v16s*p,int n);
v32s G_add_v32s(v32s a,v32s b);
v32s G_sub_v32s(v32s a,v32s b);
v32s G_mul_v32s(v32s a,v32s b);
v32s G_and_v32s(v32s a,v32s b);
v32s G_or_v32s(v32s a,v32s b);
v32s G_xor_v32s(v32s a,v32s b);
v32s G_shl_v32s(v32s a,v32s b);
v32s G_shr_v32s(v32s a,v32s b);
v32s G_neg_v32s(v32s a,v32s b);
v32s G_not_v32s(v32s a,v32s b);
v32s G_eq_v32s(v32s a,v32s b);
v32s G_lt_v32s(v32s a,v32s b);
void G_loop_v32s(v32s*r,const v32s*a,const v32s*b,int n);
v32s G_splat_v32s(short x);
short G_lane2_v32s(v32s a);
v32s G_setlane_v32s(v32s a,short x);
v32s G_ld_v32s(const short*p);
void G_st_v32s(short*p,v32s v);
v32s G_cond_v32s(v32s a,v32s b,int c);
v32s G_acc_v32s(const v32s*p,int n);
v16i G_add_v16i(v16i a,v16i b);
v16i G_sub_v16i(v16i a,v16i b);
v16i G_mul_v16i(v16i a,v16i b);
v16i G_and_v16i(v16i a,v16i b);
v16i G_or_v16i(v16i a,v16i b);
v16i G_xor_v16i(v16i a,v16i b);
v16i G_shl_v16i(v16i a,v16i b);
v16i G_shr_v16i(v16i a,v16i b);
v16i G_neg_v16i(v16i a,v16i b);
v16i G_not_v16i(v16i a,v16i b);
v16i G_eq_v16i(v16i a,v16i b);
v16i G_lt_v16i(v16i a,v16i b);
void G_loop_v16i(v16i*r,const v16i*a,const v16i*b,int n);
v16i G_splat_v16i(int x);
int G_lane2_v16i(v16i a);
v16i G_setlane_v16i(v16i a,int x);
v16i G_ld_v16i(const int*p);
void G_st_v16i(int*p,v16i v);
v16i G_cond_v16i(v16i a,v16i b,int c);
v16i G_acc_v16i(const v16i*p,int n);
v32i G_add_v32i(v32i a,v32i b);
v32i G_sub_v32i(v32i a,v32i b);
v32i G_mul_v32i(v32i a,v32i b);
v32i G_and_v32i(v32i a,v32i b);
v32i G_or_v32i(v32i a,v32i b);
v32i G_xor_v32i(v32i a,v32i b);
v32i G_shl_v32i(v32i a,v32i b);
v32i G_shr_v32i(v32i a,v32i b);
v32i G_neg_v32i(v32i a,v32i b);
v32i G_not_v32i(v32i a,v32i b);
v32i G_eq_v32i(v32i a,v32i b);
v32i G_lt_v32i(v32i a,v32i b);
void G_loop_v32i(v32i*r,const v32i*a,const v32i*b,int n);
v32i G_splat_v32i(int x);
int G_lane2_v32i(v32i a);
v32i G_setlane_v32i(v32i a,int x);
v32i G_ld_v32i(const int*p);
void G_st_v32i(int*p,v32i v);
v32i G_cond_v32i(v32i a,v32i b,int c);
v32i G_acc_v32i(const v32i*p,int n);
v16l G_add_v16l(v16l a,v16l b);
v16l G_sub_v16l(v16l a,v16l b);
v16l G_mul_v16l(v16l a,v16l b);
v16l G_and_v16l(v16l a,v16l b);
v16l G_or_v16l(v16l a,v16l b);
v16l G_xor_v16l(v16l a,v16l b);
v16l G_shl_v16l(v16l a,v16l b);
v16l G_shr_v16l(v16l a,v16l b);
v16l G_neg_v16l(v16l a,v16l b);
v16l G_not_v16l(v16l a,v16l b);
v16l G_eq_v16l(v16l a,v16l b);
v16l G_lt_v16l(v16l a,v16l b);
void G_loop_v16l(v16l*r,const v16l*a,const v16l*b,int n);
v16l G_splat_v16l(long x);
long G_lane1_v16l(v16l a);
v16l G_setlane_v16l(v16l a,long x);
v16l G_ld_v16l(const long*p);
void G_st_v16l(long*p,v16l v);
v16l G_cond_v16l(v16l a,v16l b,int c);
v16l G_acc_v16l(const v16l*p,int n);
v32l G_add_v32l(v32l a,v32l b);
v32l G_sub_v32l(v32l a,v32l b);
v32l G_mul_v32l(v32l a,v32l b);
v32l G_and_v32l(v32l a,v32l b);
v32l G_or_v32l(v32l a,v32l b);
v32l G_xor_v32l(v32l a,v32l b);
v32l G_shl_v32l(v32l a,v32l b);
v32l G_shr_v32l(v32l a,v32l b);
v32l G_neg_v32l(v32l a,v32l b);
v32l G_not_v32l(v32l a,v32l b);
v32l G_eq_v32l(v32l a,v32l b);
v32l G_lt_v32l(v32l a,v32l b);
void G_loop_v32l(v32l*r,const v32l*a,const v32l*b,int n);
v32l G_splat_v32l(long x);
long G_lane2_v32l(v32l a);
v32l G_setlane_v32l(v32l a,long x);
v32l G_ld_v32l(const long*p);
void G_st_v32l(long*p,v32l v);
v32l G_cond_v32l(v32l a,v32l b,int c);
v32l G_acc_v32l(const v32l*p,int n);
v16f G_add_v16f(v16f a,v16f b);
v16f G_sub_v16f(v16f a,v16f b);
v16f G_mul_v16f(v16f a,v16f b);
v16f G_neg_v16f(v16f a,v16f b);
v16i G_eq_v16f(v16f a,v16f b);
v16i G_lt_v16f(v16f a,v16f b);
v16f G_div_v16f(v16f a,v16f b);
void G_loop_v16f(v16f*r,const v16f*a,const v16f*b,int n);
v16f G_splat_v16f(float x);
float G_lane2_v16f(v16f a);
v16f G_setlane_v16f(v16f a,float x);
v16f G_ld_v16f(const float*p);
void G_st_v16f(float*p,v16f v);
v16f G_cond_v16f(v16f a,v16f b,int c);
v16f G_acc_v16f(const v16f*p,int n);
v32f G_add_v32f(v32f a,v32f b);
v32f G_sub_v32f(v32f a,v32f b);
v32f G_mul_v32f(v32f a,v32f b);
v32f G_neg_v32f(v32f a,v32f b);
v32i G_eq_v32f(v32f a,v32f b);
v32i G_lt_v32f(v32f a,v32f b);
v32f G_div_v32f(v32f a,v32f b);
void G_loop_v32f(v32f*r,const v32f*a,const v32f*b,int n);
v32f G_splat_v32f(float x);
float G_lane2_v32f(v32f a);
v32f G_setlane_v32f(v32f a,float x);
v32f G_ld_v32f(const float*p);
void G_st_v32f(float*p,v32f v);
v32f G_cond_v32f(v32f a,v32f b,int c);
v32f G_acc_v32f(const v32f*p,int n);
v16d G_add_v16d(v16d a,v16d b);
v16d G_sub_v16d(v16d a,v16d b);
v16d G_mul_v16d(v16d a,v16d b);
v16d G_neg_v16d(v16d a,v16d b);
v16l G_eq_v16d(v16d a,v16d b);
v16l G_lt_v16d(v16d a,v16d b);
v16d G_div_v16d(v16d a,v16d b);
void G_loop_v16d(v16d*r,const v16d*a,const v16d*b,int n);
v16d G_splat_v16d(double x);
double G_lane1_v16d(v16d a);
v16d G_setlane_v16d(v16d a,double x);
v16d G_ld_v16d(const double*p);
void G_st_v16d(double*p,v16d v);
v16d G_cond_v16d(v16d a,v16d b,int c);
v16d G_acc_v16d(const v16d*p,int n);
v32d G_add_v32d(v32d a,v32d b);
v32d G_sub_v32d(v32d a,v32d b);
v32d G_mul_v32d(v32d a,v32d b);
v32d G_neg_v32d(v32d a,v32d b);
v32l G_eq_v32d(v32d a,v32d b);
v32l G_lt_v32d(v32d a,v32d b);
v32d G_div_v32d(v32d a,v32d b);
void G_loop_v32d(v32d*r,const v32d*a,const v32d*b,int n);
v32d G_splat_v32d(double x);
double G_lane2_v32d(v32d a);
v32d G_setlane_v32d(v32d a,double x);
v32d G_ld_v32d(const double*p);
void G_st_v32d(double*p,v32d v);
v32d G_cond_v32d(v32d a,v32d b,int c);
v32d G_acc_v32d(const v32d*p,int n);
struct S8f G_sadd(struct S8f a, struct S8f b);
v32f G_viacall(v32f a, v32f b);
v32f G_call8(v32f a){ return a*a - a; }
static int G_sink_count; void G_sink(void){ G_sink_count++; }
v16c L_add_v16c(v16c a,v16c b);
v16c L_sub_v16c(v16c a,v16c b);
v16c L_mul_v16c(v16c a,v16c b);
v16c L_and_v16c(v16c a,v16c b);
v16c L_or_v16c(v16c a,v16c b);
v16c L_xor_v16c(v16c a,v16c b);
v16c L_shl_v16c(v16c a,v16c b);
v16c L_shr_v16c(v16c a,v16c b);
v16c L_neg_v16c(v16c a,v16c b);
v16c L_not_v16c(v16c a,v16c b);
v16c L_eq_v16c(v16c a,v16c b);
v16c L_lt_v16c(v16c a,v16c b);
void L_loop_v16c(v16c*r,const v16c*a,const v16c*b,int n);
v16c L_splat_v16c(char x);
char L_lane2_v16c(v16c a);
v16c L_setlane_v16c(v16c a,char x);
v16c L_ld_v16c(const char*p);
void L_st_v16c(char*p,v16c v);
v16c L_cond_v16c(v16c a,v16c b,int c);
v16c L_acc_v16c(const v16c*p,int n);
v32c L_add_v32c(v32c a,v32c b);
v32c L_sub_v32c(v32c a,v32c b);
v32c L_mul_v32c(v32c a,v32c b);
v32c L_and_v32c(v32c a,v32c b);
v32c L_or_v32c(v32c a,v32c b);
v32c L_xor_v32c(v32c a,v32c b);
v32c L_shl_v32c(v32c a,v32c b);
v32c L_shr_v32c(v32c a,v32c b);
v32c L_neg_v32c(v32c a,v32c b);
v32c L_not_v32c(v32c a,v32c b);
v32c L_eq_v32c(v32c a,v32c b);
v32c L_lt_v32c(v32c a,v32c b);
void L_loop_v32c(v32c*r,const v32c*a,const v32c*b,int n);
v32c L_splat_v32c(char x);
char L_lane2_v32c(v32c a);
v32c L_setlane_v32c(v32c a,char x);
v32c L_ld_v32c(const char*p);
void L_st_v32c(char*p,v32c v);
v32c L_cond_v32c(v32c a,v32c b,int c);
v32c L_acc_v32c(const v32c*p,int n);
v16s L_add_v16s(v16s a,v16s b);
v16s L_sub_v16s(v16s a,v16s b);
v16s L_mul_v16s(v16s a,v16s b);
v16s L_and_v16s(v16s a,v16s b);
v16s L_or_v16s(v16s a,v16s b);
v16s L_xor_v16s(v16s a,v16s b);
v16s L_shl_v16s(v16s a,v16s b);
v16s L_shr_v16s(v16s a,v16s b);
v16s L_neg_v16s(v16s a,v16s b);
v16s L_not_v16s(v16s a,v16s b);
v16s L_eq_v16s(v16s a,v16s b);
v16s L_lt_v16s(v16s a,v16s b);
void L_loop_v16s(v16s*r,const v16s*a,const v16s*b,int n);
v16s L_splat_v16s(short x);
short L_lane2_v16s(v16s a);
v16s L_setlane_v16s(v16s a,short x);
v16s L_ld_v16s(const short*p);
void L_st_v16s(short*p,v16s v);
v16s L_cond_v16s(v16s a,v16s b,int c);
v16s L_acc_v16s(const v16s*p,int n);
v32s L_add_v32s(v32s a,v32s b);
v32s L_sub_v32s(v32s a,v32s b);
v32s L_mul_v32s(v32s a,v32s b);
v32s L_and_v32s(v32s a,v32s b);
v32s L_or_v32s(v32s a,v32s b);
v32s L_xor_v32s(v32s a,v32s b);
v32s L_shl_v32s(v32s a,v32s b);
v32s L_shr_v32s(v32s a,v32s b);
v32s L_neg_v32s(v32s a,v32s b);
v32s L_not_v32s(v32s a,v32s b);
v32s L_eq_v32s(v32s a,v32s b);
v32s L_lt_v32s(v32s a,v32s b);
void L_loop_v32s(v32s*r,const v32s*a,const v32s*b,int n);
v32s L_splat_v32s(short x);
short L_lane2_v32s(v32s a);
v32s L_setlane_v32s(v32s a,short x);
v32s L_ld_v32s(const short*p);
void L_st_v32s(short*p,v32s v);
v32s L_cond_v32s(v32s a,v32s b,int c);
v32s L_acc_v32s(const v32s*p,int n);
v16i L_add_v16i(v16i a,v16i b);
v16i L_sub_v16i(v16i a,v16i b);
v16i L_mul_v16i(v16i a,v16i b);
v16i L_and_v16i(v16i a,v16i b);
v16i L_or_v16i(v16i a,v16i b);
v16i L_xor_v16i(v16i a,v16i b);
v16i L_shl_v16i(v16i a,v16i b);
v16i L_shr_v16i(v16i a,v16i b);
v16i L_neg_v16i(v16i a,v16i b);
v16i L_not_v16i(v16i a,v16i b);
v16i L_eq_v16i(v16i a,v16i b);
v16i L_lt_v16i(v16i a,v16i b);
void L_loop_v16i(v16i*r,const v16i*a,const v16i*b,int n);
v16i L_splat_v16i(int x);
int L_lane2_v16i(v16i a);
v16i L_setlane_v16i(v16i a,int x);
v16i L_ld_v16i(const int*p);
void L_st_v16i(int*p,v16i v);
v16i L_cond_v16i(v16i a,v16i b,int c);
v16i L_acc_v16i(const v16i*p,int n);
v32i L_add_v32i(v32i a,v32i b);
v32i L_sub_v32i(v32i a,v32i b);
v32i L_mul_v32i(v32i a,v32i b);
v32i L_and_v32i(v32i a,v32i b);
v32i L_or_v32i(v32i a,v32i b);
v32i L_xor_v32i(v32i a,v32i b);
v32i L_shl_v32i(v32i a,v32i b);
v32i L_shr_v32i(v32i a,v32i b);
v32i L_neg_v32i(v32i a,v32i b);
v32i L_not_v32i(v32i a,v32i b);
v32i L_eq_v32i(v32i a,v32i b);
v32i L_lt_v32i(v32i a,v32i b);
void L_loop_v32i(v32i*r,const v32i*a,const v32i*b,int n);
v32i L_splat_v32i(int x);
int L_lane2_v32i(v32i a);
v32i L_setlane_v32i(v32i a,int x);
v32i L_ld_v32i(const int*p);
void L_st_v32i(int*p,v32i v);
v32i L_cond_v32i(v32i a,v32i b,int c);
v32i L_acc_v32i(const v32i*p,int n);
v16l L_add_v16l(v16l a,v16l b);
v16l L_sub_v16l(v16l a,v16l b);
v16l L_mul_v16l(v16l a,v16l b);
v16l L_and_v16l(v16l a,v16l b);
v16l L_or_v16l(v16l a,v16l b);
v16l L_xor_v16l(v16l a,v16l b);
v16l L_shl_v16l(v16l a,v16l b);
v16l L_shr_v16l(v16l a,v16l b);
v16l L_neg_v16l(v16l a,v16l b);
v16l L_not_v16l(v16l a,v16l b);
v16l L_eq_v16l(v16l a,v16l b);
v16l L_lt_v16l(v16l a,v16l b);
void L_loop_v16l(v16l*r,const v16l*a,const v16l*b,int n);
v16l L_splat_v16l(long x);
long L_lane1_v16l(v16l a);
v16l L_setlane_v16l(v16l a,long x);
v16l L_ld_v16l(const long*p);
void L_st_v16l(long*p,v16l v);
v16l L_cond_v16l(v16l a,v16l b,int c);
v16l L_acc_v16l(const v16l*p,int n);
v32l L_add_v32l(v32l a,v32l b);
v32l L_sub_v32l(v32l a,v32l b);
v32l L_mul_v32l(v32l a,v32l b);
v32l L_and_v32l(v32l a,v32l b);
v32l L_or_v32l(v32l a,v32l b);
v32l L_xor_v32l(v32l a,v32l b);
v32l L_shl_v32l(v32l a,v32l b);
v32l L_shr_v32l(v32l a,v32l b);
v32l L_neg_v32l(v32l a,v32l b);
v32l L_not_v32l(v32l a,v32l b);
v32l L_eq_v32l(v32l a,v32l b);
v32l L_lt_v32l(v32l a,v32l b);
void L_loop_v32l(v32l*r,const v32l*a,const v32l*b,int n);
v32l L_splat_v32l(long x);
long L_lane2_v32l(v32l a);
v32l L_setlane_v32l(v32l a,long x);
v32l L_ld_v32l(const long*p);
void L_st_v32l(long*p,v32l v);
v32l L_cond_v32l(v32l a,v32l b,int c);
v32l L_acc_v32l(const v32l*p,int n);
v16f L_add_v16f(v16f a,v16f b);
v16f L_sub_v16f(v16f a,v16f b);
v16f L_mul_v16f(v16f a,v16f b);
v16f L_neg_v16f(v16f a,v16f b);
v16i L_eq_v16f(v16f a,v16f b);
v16i L_lt_v16f(v16f a,v16f b);
v16f L_div_v16f(v16f a,v16f b);
void L_loop_v16f(v16f*r,const v16f*a,const v16f*b,int n);
v16f L_splat_v16f(float x);
float L_lane2_v16f(v16f a);
v16f L_setlane_v16f(v16f a,float x);
v16f L_ld_v16f(const float*p);
void L_st_v16f(float*p,v16f v);
v16f L_cond_v16f(v16f a,v16f b,int c);
v16f L_acc_v16f(const v16f*p,int n);
v32f L_add_v32f(v32f a,v32f b);
v32f L_sub_v32f(v32f a,v32f b);
v32f L_mul_v32f(v32f a,v32f b);
v32f L_neg_v32f(v32f a,v32f b);
v32i L_eq_v32f(v32f a,v32f b);
v32i L_lt_v32f(v32f a,v32f b);
v32f L_div_v32f(v32f a,v32f b);
void L_loop_v32f(v32f*r,const v32f*a,const v32f*b,int n);
v32f L_splat_v32f(float x);
float L_lane2_v32f(v32f a);
v32f L_setlane_v32f(v32f a,float x);
v32f L_ld_v32f(const float*p);
void L_st_v32f(float*p,v32f v);
v32f L_cond_v32f(v32f a,v32f b,int c);
v32f L_acc_v32f(const v32f*p,int n);
v16d L_add_v16d(v16d a,v16d b);
v16d L_sub_v16d(v16d a,v16d b);
v16d L_mul_v16d(v16d a,v16d b);
v16d L_neg_v16d(v16d a,v16d b);
v16l L_eq_v16d(v16d a,v16d b);
v16l L_lt_v16d(v16d a,v16d b);
v16d L_div_v16d(v16d a,v16d b);
void L_loop_v16d(v16d*r,const v16d*a,const v16d*b,int n);
v16d L_splat_v16d(double x);
double L_lane1_v16d(v16d a);
v16d L_setlane_v16d(v16d a,double x);
v16d L_ld_v16d(const double*p);
void L_st_v16d(double*p,v16d v);
v16d L_cond_v16d(v16d a,v16d b,int c);
v16d L_acc_v16d(const v16d*p,int n);
v32d L_add_v32d(v32d a,v32d b);
v32d L_sub_v32d(v32d a,v32d b);
v32d L_mul_v32d(v32d a,v32d b);
v32d L_neg_v32d(v32d a,v32d b);
v32l L_eq_v32d(v32d a,v32d b);
v32l L_lt_v32d(v32d a,v32d b);
v32d L_div_v32d(v32d a,v32d b);
void L_loop_v32d(v32d*r,const v32d*a,const v32d*b,int n);
v32d L_splat_v32d(double x);
double L_lane2_v32d(v32d a);
v32d L_setlane_v32d(v32d a,double x);
v32d L_ld_v32d(const double*p);
void L_st_v32d(double*p,v32d v);
v32d L_cond_v32d(v32d a,v32d b,int c);
v32d L_acc_v32d(const v32d*p,int n);
struct S8f L_sadd(struct S8f a, struct S8f b);
v32f L_viacall(v32f a, v32f b);
v32f L_call8(v32f a){ return a*a - a; }
static int L_sink_count; void L_sink(void){ L_sink_count++; }

static uint64_t seed=0x9E3779B97F4A7C15ull;
static uint64_t rnd(void){ seed^=seed<<13; seed^=seed>>7; seed^=seed<<17; return seed; }
static void fill(void*p,int n,int kind){ unsigned char*b=p; for(int i=0;i<n;i++) b[i]=(unsigned char)rnd(); (void)kind; }
/* Signed vector arithmetic has the scalar C overflow/left-shift rules. Keep
   raw bits for bitwise/compare/right-shift tests, but prove these arithmetic
   inputs defined: [-3,3] cannot overflow the loop/nine-argument sums, and
   0..3 shifted by 0..lane_bits-3 stays representable and nonnegative. */
#define SIGNED_INPUT(V) do { \
  for(unsigned i=0;i<sizeof(V)/sizeof((V)[0]);i++) \
    (V)[i]=(__typeof__((V)[0]))((int)(rnd()%7)-3); \
} while(0)
#define SIGNED_LEFT_INPUT(A,B) do { \
  for(unsigned i=0;i<sizeof(A)/sizeof((A)[0]);i++) { \
    (A)[i]=(__typeof__((A)[0]))(rnd()%4); \
    (B)[i]=(__typeof__((B)[0]))(rnd()%(sizeof((A)[0])*8-2)); \
  } \
} while(0)
#define SIGNED_LEFT_SCALAR_INPUT(A,N) do { \
  for(unsigned i=0;i<sizeof(A)/sizeof((A)[0]);i++) \
    (A)[i]=(__typeof__((A)[0]))(rnd()%4); \
  (N)=(__typeof__(N))(rnd()%(sizeof((A)[0])*8-2)); \
} while(0)
static int fails=0, total=0;
static void check(const char*n,const void*a,const void*b,int sz){ total++; if(memcmp(a,b,sz)){ fails++; printf("MISMATCH %s\n",n);} }


struct S4i { v16i v; int tag; };
union U { v32f v; v16f h[2]; };
v32f G_mix_sab(float,v32f,v32f); v32f L_mix_sab(float,v32f,v32f);
v16f G_mix_sab4(float,v16f,v16f); v16f L_mix_sab4(float,v16f,v16f);
v32i G_splat_r(v32i); v32i L_splat_r(v32i);
v32f G_splat_l(v32f); v32f L_splat_l(v32f);
v32f G_nine(v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f); v32f L_nine(v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f);
v16i G_nine4(v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i); v16i L_nine4(v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i);
v32f G_ident(v32f); v32f L_ident(v32f);
v16d G_ident2(v16d); v16d L_ident2(v16d);
void G_asg(v32f*,const v32f*); void L_asg(v32f*,const v32f*);
void G_asg_self(v32f*); void L_asg_self(v32f*);
void G_asg4(v16i*,const v16i*); void L_asg4(v16i*,const v16i*);
void G_asg_union(union U*); void L_asg_union(union U*);
v32f G_tern_lv(v32f,v32f,int); v32f L_tern_lv(v32f,v32f,int);
extern int G_tern_count, L_tern_count;
v32f G_tern_se(v32f,v32f,int); v32f L_tern_se(v32f,v32f,int);
v32f G_tern_nest(v32f,v32f,int); v32f L_tern_nest(v32f,v32f,int);
v32f G_tern_assign(v32f,v32f,int); v32f L_tern_assign(v32f,v32f,int);
v32f G_negz(v32f); v32f L_negz(v32f);
v16d G_negz2(v16d); v16d L_negz2(v16d);
struct S4i G_s4i_add(struct S4i, v16i); struct S4i L_s4i_add(struct S4i, v16i);
void G_cas_ptr(v32f*,v32f); void L_cas_ptr(v32f*,v32f);
void G_cas_arr(v16i*,v16i,int); void L_cas_arr(v16i*,v16i,int);
v32i G_sel_mask(v32f,v32f,v32i,v32i); v32i L_sel_mask(v32f,v32f,v32i,v32i);
#define FMA3(T,N) T G_##N(T,T,T); T L_##N(T,T,T);
#define FMA2(T,N) T G_##N(T,T); T L_##N(T,T);
FMA3(v32f,fma_f8) FMA3(v32f,fma_f8r) FMA3(v32f,fnma_f8) FMA3(v32f,fms_f8)
FMA3(v32d,fma_d4) FMA3(v32d,fnma_d4) FMA3(v32d,fms_d4)
FMA3(v16f,fma_f4) FMA3(v16f,fnma_f4) FMA3(v16f,fms_f4)
FMA3(v16d,fma_d2) FMA3(v16d,fnma_d2)
FMA2(v32f,fma_sq) FMA2(v16f,fma_sq4) FMA2(v32f,fma_self) FMA3(v32f,fma_nouse)
v32f G_fma_chain(v32f,v32f,v32f,v32f); v32f L_fma_chain(v32f,v32f,v32f,v32f);
v32f G_fma_two(v32f,v32f,v32f,v32f); v32f L_fma_two(v32f,v32f,v32f,v32f);
void G_fma_loop(v32f*,const v32f*,const v32f*,const v32f*,int); void L_fma_loop(v32f*,const v32f*,const v32f*,const v32f*,int);
void G_fma_acc(v32d*,const v32d*,const v32d*,int); void L_fma_acc(v32d*,const v32d*,const v32d*,int);
struct W8 { v32f v; int tag; };
typedef unsigned v32u __attribute__((vector_size(32)));
typedef unsigned long v32ul __attribute__((vector_size(32)));
typedef unsigned v16u __attribute__((vector_size(16)));
#define SH(pfx) v32u pfx##sh_vv_u(v32u,v32u); v32ul pfx##sh_vv_ul(v32ul,v32ul); v16u pfx##sh_vv_u4(v16u,v16u); \
  v32i pfx##sh_vs_shl(v32i,int); v32i pfx##sh_vs_sar(v32i,int); v32u pfx##sh_vs_shr(v32u,unsigned); \
  v32l pfx##sh_vs_shl_l(v32l,long); v32ul pfx##sh_vs_shr_ul(v32ul,int); v32i pfx##sh_mixsign(v32i,v32u); \
  v32u pfx##sh_mixsign2(v32u,v32i); v32i pfx##sh_chain(v32i,v32i,v32i); v32i pfx##sh_then_add(v32i,v32i); \
  v16i pfx##sh4_vs_sar(v16i,int); v16l pfx##sh4_vv_shl_l(v16l,v16l); v16i pfx##sh4_chain(v16i,v16i,v16i);
SH(G_) SH(L_)
typedef unsigned long v16ul __attribute__((vector_size(16)));
typedef short v16s_ __attribute__((vector_size(16)));
#define CQ(pfx) v32l pfx##cq_lt_s(v32l,v32l); v32l pfx##cq_le_s(v32l,v32l); v32l pfx##cq_gt_s(v32l,v32l); v32l pfx##cq_ne_s(v32l,v32l); \
  v32l pfx##cq_lt_u(v32ul,v32ul); v32l pfx##cq_le_u(v32ul,v32ul); v32l pfx##cq_ge_u(v32ul,v32ul); v32l pfx##cq_eq_u(v32ul,v32ul); \
  v16l pfx##cq4_lt_s(v16l,v16l); v16l pfx##cq4_ne_s(v16l,v16l); v16l pfx##cq4_lt_u(v16ul,v16ul); v16l pfx##cq4_le_u(v16ul,v16ul); \
  v16i pfx##cd4_le_s(v16i,v16i); v16i pfx##cd4_ne_s(v16i,v16i); v16i pfx##cd4_lt_u(v16u,v16u); v16i pfx##cd4_le_u(v16u,v16u); \
  v16s_ pfx##cw4_ne_s(v16s_,v16s_); v16c pfx##cb4_le_s(v16c,v16c); v32l pfx##cq_sel(v32l,v32l,v32l,v32l);
CQ(G_) CQ(L_)
#define CF(pfx) v32f pfx##cf_phi_call(v32f,v32f,int); v32f pfx##cf_loop_acc(const v32f*,int); v32f pfx##cf_loop_acc_call(const v32f*,int); \
  v32f pfx##cf_nested_tern(v32f,v32f,int); v32f pfx##cf_sw(v32f,v32f,int); v32f pfx##cf_goto_loop(const v32f*,int); v32f pfx##cf_two_lives(v32f,v32f); \
  v32l pfx##cf_l_phi(v32l,v32l,int); v32d pfx##cf_d_mix(v32d,v32d,double); float pfx##cf_lane_after(v32f,v32f); v32f pfx##cf_cond_assign(v32f,v32f,int); \
  v32f pfx##cf_wrap_phi(struct W8,v32f,int); v32f pfx##cf_addr_escape(v32f,v32f); v32f pfx##cf_volatile_t(v32f,v32f); v32i pfx##cf_cmp_chain(v32f,v32f,v32f); \
  v32f pfx##cf_self_assign(v32f); v32f pfx##cf_dead_store(v32f,v32f); v32f pfx##cf_all_lanes(v32f); v32f pfx##cf_uninit(v32f);
CF(G_) CF(L_)
#define HOFFD(T,N) FMA3(T,N##_p0) FMA3(T,N##_p1) FMA3(T,N##_p2) FMA3(T,N##_p1s) FMA3(T,N##_p2s) FMA3(T,N##_p2n) \
  FMA3(T,N##_sq) FMA3(T,N##_al) FMA3(T,N##_al2) FMA3(T,N##_ret0) FMA3(T,N##_ret1) \
  T G_##N##_ch(T,T,T,T); T L_##N##_ch(T,T,T,T); T G_##N##_ch2(T,T,T,T); T L_##N##_ch2(T,T,T,T); \
  T G_##N##_two(T,T,T,T); T L_##N##_two(T,T,T,T); T G_##N##_live(T,T,T,T*); T L_##N##_live(T,T,T,T*);
HOFFD(v16f,ho_f4) HOFFD(v16d,ho_d2) HOFFD(v32f,ho_f8) HOFFD(v32d,ho_d4)
/* inexact lane values (odd / 7): the product rounds, so a fused and a
   separately rounded evaluation differ in the last bit */
static void fillx8(v32f*v){ for(int i=0;i<8;i++) (*v)[i]=(float)((int)(rnd()%20000)-10000)/7.0f; }
static void fillx4(v16f*v){ for(int i=0;i<4;i++) (*v)[i]=(float)((int)(rnd()%20000)-10000)/7.0f; }
static void fillxd4(v32d*v){ for(int i=0;i<4;i++) (*v)[i]=(double)((int)(rnd()%20000)-10000)/7.0; }
static void fillxd2(v16d*v){ for(int i=0;i<2;i++) (*v)[i]=(double)((int)(rnd()%20000)-10000)/7.0; }
#define CHK3(T,N,FILL) { T a,b,c; FILL(&a); FILL(&b); FILL(&c); T r1=G_##N(a,b,c), r2=L_##N(a,b,c); check(#N,&r1,&r2,sizeof r1); }
#define CHK2(T,N,FILL) { T a,b; FILL(&a); FILL(&b); T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
static void fma_checks(void){
  for(int rep=0;rep<4;rep++){
  CHK3(v32f,fma_f8,fillx8) CHK3(v32f,fma_f8r,fillx8) CHK3(v32f,fnma_f8,fillx8) CHK3(v32f,fms_f8,fillx8)
  CHK3(v32d,fma_d4,fillxd4) CHK3(v32d,fnma_d4,fillxd4) CHK3(v32d,fms_d4,fillxd4)
  CHK3(v16f,fma_f4,fillx4) CHK3(v16f,fnma_f4,fillx4) CHK3(v16f,fms_f4,fillx4)
  CHK3(v16d,fma_d2,fillxd2) CHK3(v16d,fnma_d2,fillxd2)
  CHK2(v32f,fma_sq,fillx8) CHK2(v16f,fma_sq4,fillx4) CHK2(v32f,fma_self,fillx8) CHK3(v32f,fma_nouse,fillx8)
  { v32f a,b,c,d; fillx8(&a); fillx8(&b); fillx8(&c); fillx8(&d); v32f r1=G_fma_chain(a,b,c,d), r2=L_fma_chain(a,b,c,d); check("fma_chain",&r1,&r2,sizeof r1);
    r1=G_fma_two(a,b,c,d); r2=L_fma_two(a,b,c,d); check("fma_two",&r1,&r2,sizeof r1); }
  { v32f a[5],b[5],c[5],r1[5],r2[5]; for(int k=0;k<5;k++){ fillx8(&a[k]); fillx8(&b[k]); fillx8(&c[k]); } G_fma_loop(r1,a,b,c,5); L_fma_loop(r2,a,b,c,5); check("fma_loop",r1,r2,sizeof r1); }
  { v32d a[5],b[5],r1,r2; for(int k=0;k<5;k++){ fillxd4(&a[k]); fillxd4(&b[k]); } G_fma_acc(&r1,a,b,5); L_fma_acc(&r2,a,b,5); check("fma_acc",&r1,&r2,sizeof r1); }
  }
}
#define CHK4(T,N,FILL) { T a,b,c,d; FILL(&a); FILL(&b); FILL(&c); FILL(&d); T r1=G_##N(a,b,c,d), r2=L_##N(a,b,c,d); check(#N,&r1,&r2,sizeof r1); }
#define CHKL(T,N,FILL) { T a,b,c,o1,o2; FILL(&a); FILL(&b); FILL(&c); T r1=G_##N(a,b,c,&o1), r2=L_##N(a,b,c,&o2); check(#N,&r1,&r2,sizeof r1); check(#N ".o",&o1,&o2,sizeof o1); }
#define HOFFC(T,N,FILL) CHK3(T,N##_p0,FILL) CHK3(T,N##_p1,FILL) CHK3(T,N##_p2,FILL) CHK3(T,N##_p1s,FILL) CHK3(T,N##_p2s,FILL) CHK3(T,N##_p2n,FILL) \
  CHK3(T,N##_sq,FILL) CHK3(T,N##_al,FILL) CHK3(T,N##_al2,FILL) CHK3(T,N##_ret0,FILL) CHK3(T,N##_ret1,FILL) \
  CHK4(T,N##_ch,FILL) CHK4(T,N##_ch2,FILL) CHK4(T,N##_two,FILL) CHKL(T,N##_live,FILL)
static void fillf8(v32f*v);
#define CFEQ(N,...) { v32f r1=G_##N(__VA_ARGS__), r2=L_##N(__VA_ARGS__); check(#N,&r1,&r2,sizeof r1); }
static void cf_checks(void){
  v32f arr[7]; for(int k=0;k<7;k++) fillf8(&arr[k]);
  v32f a,b,c; fillf8(&a); fillf8(&b); fillf8(&c);
  for(int n=0;n<=6;n++){ CFEQ(cf_loop_acc,arr,n) CFEQ(cf_loop_acc_call,arr,n) CFEQ(cf_goto_loop,arr,n<1?1:n) }
  for(int k=0;k<4;k++){
    CFEQ(cf_phi_call,a,b,k) CFEQ(cf_nested_tern,a,b,k) CFEQ(cf_sw,a,b,k) CFEQ(cf_cond_assign,a,b,k)
    { v32l x,y; fill(&x,sizeof x,0); fill(&y,sizeof y,0); v32l r1=G_cf_l_phi(x,y,k), r2=L_cf_l_phi(x,y,k); check("cf_l_phi",&r1,&r2,sizeof r1); }
    { struct W8 w; w.v=a; w.tag=k; CFEQ(cf_wrap_phi,w,b,k) }
  }
  CFEQ(cf_two_lives,a,b) CFEQ(cf_addr_escape,a,b) CFEQ(cf_volatile_t,a,b) CFEQ(cf_self_assign,a) CFEQ(cf_dead_store,a,b) CFEQ(cf_all_lanes,a) CFEQ(cf_uninit,a)
  { v32d x,y; fillxd4(&x); fillxd4(&y); double s=(double)(rnd()%40)/8.0; v32d r1=G_cf_d_mix(x,y,s), r2=L_cf_d_mix(x,y,s); check("cf_d_mix",&r1,&r2,sizeof r1); }
  { float r1=G_cf_lane_after(a,b), r2=L_cf_lane_after(a,b); check("cf_lane_after",&r1,&r2,sizeof r1); }
  { v32i r1=G_cf_cmp_chain(a,b,c), r2=L_cf_cmp_chain(a,b,c); check("cf_cmp_chain",&r1,&r2,sizeof r1); }
  check("cf_sink_count",&G_sink_count,&L_sink_count,sizeof(int));
}
/* counts reduced modulo the lane width: wider counts are undefined for
   GNU vector shifts exactly as for the scalar lanes */
#define SHVV(T,N,W) { T a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(unsigned i=0;i<sizeof a/sizeof a[0];i++) b[i]=(b[i]&(W-1)); \
  T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
#define SHVS(T,N,CT,W) { T a; fill(&a,sizeof a,0); CT n=(CT)(rnd()%W); T r1=G_##N(a,n), r2=L_##N(a,n); check(#N,&r1,&r2,sizeof r1); }
#define SHLV(T,N) { T a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); SIGNED_LEFT_INPUT(a,b); \
  T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
#define SHLS(T,N,CT) { T a; fill(&a,sizeof a,0); CT n; SIGNED_LEFT_SCALAR_INPUT(a,n); T r1=G_##N(a,n), r2=L_##N(a,n); check(#N,&r1,&r2,sizeof r1); }
static void shift_checks(void){
  for(int rep=0;rep<4;rep++){
    SHVV(v32u,sh_vv_u,32) SHVV(v32ul,sh_vv_ul,64) SHVV(v16u,sh_vv_u4,32) SHLV(v32i,sh_then_add)
    SHLS(v32i,sh_vs_shl,int) SHVS(v32i,sh_vs_sar,int,32) SHVS(v32u,sh_vs_shr,unsigned,32)
    SHLS(v32l,sh_vs_shl_l,long) SHVS(v32ul,sh_vs_shr_ul,int,64)
    SHVS(v16i,sh4_vs_sar,int,32) SHLV(v16l,sh4_vv_shl_l)
    { v16i a,b,c; fill(&a,sizeof a,0); fill(&b,sizeof b,0); fill(&c,sizeof c,0); SIGNED_LEFT_INPUT(a,b); for(int i=0;i<4;i++) c[i]&=31; v16i r1=G_sh4_chain(a,b,c), r2=L_sh4_chain(a,b,c); check("sh4_chain",&r1,&r2,sizeof r1); }
    { v32i a; v32u b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); SIGNED_LEFT_INPUT(a,b); v32i r1=G_sh_mixsign(a,b), r2=L_sh_mixsign(a,b); check("sh_mixsign",&r1,&r2,sizeof r1); }
    { v32u a; v32i b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<8;i++) b[i]&=31; v32u r1=G_sh_mixsign2(a,b), r2=L_sh_mixsign2(a,b); check("sh_mixsign2",&r1,&r2,sizeof r1); }
    { v32i a,b,c; fill(&a,sizeof a,0); fill(&b,sizeof b,0); fill(&c,sizeof c,0); SIGNED_LEFT_INPUT(a,b); for(int i=0;i<8;i++) c[i]&=31; v32i r1=G_sh_chain(a,b,c), r2=L_sh_chain(a,b,c); check("sh_chain",&r1,&r2,sizeof r1); }
  }
}
/* compare operands: random lanes, with some lanes forced equal and some to
   the sign-bit boundary so eq/le/unsigned-order paths are all exercised */
#define CMPV(T,N,AT) { AT a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); \
  for(unsigned i=0;i<sizeof a/sizeof a[0];i++){ if(i%3==0) b[i]=a[i]; if(i%4==1) a[i]=(__typeof__(a[0]))((uint64_t)1<<(sizeof a[0]*8-1)); } \
  T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
static void cmp64_checks(void){
  for(int rep=0;rep<4;rep++){
    CMPV(v32l,cq_lt_s,v32l) CMPV(v32l,cq_le_s,v32l) CMPV(v32l,cq_gt_s,v32l) CMPV(v32l,cq_ne_s,v32l)
    CMPV(v32l,cq_lt_u,v32ul) CMPV(v32l,cq_le_u,v32ul) CMPV(v32l,cq_ge_u,v32ul) CMPV(v32l,cq_eq_u,v32ul)
    CMPV(v16l,cq4_lt_s,v16l) CMPV(v16l,cq4_ne_s,v16l) CMPV(v16l,cq4_lt_u,v16ul) CMPV(v16l,cq4_le_u,v16ul)
    CMPV(v16i,cd4_le_s,v16i) CMPV(v16i,cd4_ne_s,v16i) CMPV(v16i,cd4_lt_u,v16u) CMPV(v16i,cd4_le_u,v16u)
    CMPV(v16s_,cw4_ne_s,v16s_) CMPV(v16c,cb4_le_s,v16c)
    { v32l a,b,x,y; fill(&a,sizeof a,0); fill(&b,sizeof b,0); fill(&x,sizeof x,0); fill(&y,sizeof y,0); b[1]=a[1];
      v32l r1=G_cq_sel(a,b,x,y), r2=L_cq_sel(a,b,x,y); check("cq_sel",&r1,&r2,sizeof r1); }
  }
}
static void handoff_checks(void){
  for(int rep=0;rep<3;rep++){
    HOFFC(v16f,ho_f4,fillx4) HOFFC(v16d,ho_d2,fillxd2) HOFFC(v32f,ho_f8,fillx8) HOFFC(v32d,ho_d4,fillxd4)
  }
}
static void fillf8(v32f*v){ for(int i=0;i<8;i++) (*v)[i]=(float)((int)(rnd()%2000)-1000)/8.0f; }
static void fillf4(v16f*v){ for(int i=0;i<4;i++) (*v)[i]=(float)((int)(rnd()%2000)-1000)/8.0f; }
static void filld2(v16d*v){ for(int i=0;i<2;i++) (*v)[i]=(double)((int)(rnd()%2000)-1000)/8.0; }
static void extra_checks(void){
  { v32f a,b; fillf8(&a); fillf8(&b); float s=(float)(rnd()%50)/4.0f; v32f r1=G_mix_sab(s,a,b), r2=L_mix_sab(s,a,b); check("mix_sab",&r1,&r2,sizeof r1); }
  { v16f a,b; fillf4(&a); fillf4(&b); float s=(float)(rnd()%50)/4.0f; v16f r1=G_mix_sab4(s,a,b), r2=L_mix_sab4(s,a,b); check("mix_sab4",&r1,&r2,sizeof r1); }
  { v32i a; fill(&a,sizeof a,0); SIGNED_INPUT(a); v32i r1=G_splat_r(a), r2=L_splat_r(a); check("splat_r",&r1,&r2,sizeof r1); }
  { v32f a; fillf8(&a); v32f r1=G_splat_l(a), r2=L_splat_l(a); check("splat_l",&r1,&r2,sizeof r1); }
  { v32f v[9]; for(int k=0;k<9;k++) fillf8(&v[k]); v32f r1=G_nine(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]), r2=L_nine(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]); check("nine",&r1,&r2,sizeof r1); }
  { v16i v[9]; fill(v,sizeof v,0); for(int k=0;k<9;k++) SIGNED_INPUT(v[k]); v16i r1=G_nine4(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]), r2=L_nine4(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]); check("nine4",&r1,&r2,sizeof r1); }
  { v32f a; fillf8(&a); v32f r1=G_ident(a), r2=L_ident(a); check("ident",&r1,&r2,sizeof r1); }
  { v16d a; filld2(&a); v16d r1=G_ident2(a), r2=L_ident2(a); check("ident2",&r1,&r2,sizeof r1); }
  { v32f a,r1,r2; fillf8(&a); fillf8(&r1); r2=r1; G_asg(&r1,&a); L_asg(&r2,&a); check("asg",&r1,&r2,sizeof r1); G_asg_self(&r1); L_asg_self(&r2); check("asg_self",&r1,&r2,sizeof r1); }
  { v16i a,r1,r2; fill(&a,sizeof a,0); fill(&r1,sizeof r1,0); r2=r1; G_asg4(&r1,&a); L_asg4(&r2,&a); check("asg4",&r1,&r2,sizeof r1); }
  { union U u1,u2; fillf8(&u1.v); u2=u1; G_asg_union(&u1); L_asg_union(&u2); check("asg_union",&u1,&u2,sizeof u1); }
  { v32f a,b; fillf8(&a); fillf8(&b); for(int c=0;c<3;c++){ v32f r1=G_tern_lv(a,b,c), r2=L_tern_lv(a,b,c); check("tern_lv",&r1,&r2,sizeof r1);
      r1=G_tern_nest(a,b,c); r2=L_tern_nest(a,b,c); check("tern_nest",&r1,&r2,sizeof r1);
      r1=G_tern_assign(a,b,c); r2=L_tern_assign(a,b,c); check("tern_assign",&r1,&r2,sizeof r1);
      G_tern_count=L_tern_count=0; r1=G_tern_se(a,b,c); r2=L_tern_se(a,b,c); check("tern_se",&r1,&r2,sizeof r1); check("tern_se_count",&G_tern_count,&L_tern_count,sizeof(int)); } }
  { v32f a={0.0f,-0.0f,1.0f,-1.0f,0.0f,-0.0f,2.5f,-2.5f}; v32f r1=G_negz(a), r2=L_negz(a); check("negz",&r1,&r2,sizeof r1); }
  { v16d a={0.0,-0.0}; v16d r1=G_negz2(a), r2=L_negz2(a); check("negz2",&r1,&r2,sizeof r1); }
  { struct S4i a; v16i b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); SIGNED_INPUT(a.v); SIGNED_INPUT(b); a.tag=(int)(rnd()%200)-100; struct S4i r1=G_s4i_add(a,b), r2=L_s4i_add(a,b); check("s4i_add.v",&r1.v,&r2.v,sizeof r1.v); check("s4i_add.tag",&r1.tag,&r2.tag,sizeof r1.tag); }
  { v32f p1,p2,b; fillf8(&p1); p2=p1; fillf8(&b); G_cas_ptr(&p1,b); L_cas_ptr(&p2,b); check("cas_ptr",&p1,&p2,sizeof p1); }
  { v16i p1[3],p2[3],b; fill(p1,sizeof p1,0); memcpy(p2,p1,sizeof p1); fill(&b,sizeof b,0); G_cas_arr(p1,b,2); L_cas_arr(p2,b,2); check("cas_arr",p1,p2,sizeof p1); }
  { v32f a,b; v32i x,y; fillf8(&a); fillf8(&b); fill(&x,sizeof x,0); fill(&y,sizeof y,0); v32i r1=G_sel_mask(a,b,x,y), r2=L_sel_mask(a,b,x,y); check("sel_mask",&r1,&r2,sizeof r1); }
}

int main(void){ extra_checks(); fma_checks(); handoff_checks(); cf_checks(); shift_checks(); cmp64_checks();
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16c r1=G_add_v16c(a,b), r2=L_add_v16c(a,b); check("add_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16c r1=G_sub_v16c(a,b), r2=L_sub_v16c(a,b); check("sub_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16c r1=G_mul_v16c(a,b), r2=L_mul_v16c(a,b); check("mul_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16c r1=G_and_v16c(a,b), r2=L_and_v16c(a,b); check("and_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16c r1=G_or_v16c(a,b), r2=L_or_v16c(a,b); check("or_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16c r1=G_xor_v16c(a,b), r2=L_xor_v16c(a,b); check("xor_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v16c r1=G_shl_v16c(a,b), r2=L_shl_v16c(a,b); check("shl_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<16;i++) b[i]=(char)(((unsigned)b[i])%8);
  v16c r1=G_shr_v16c(a,b), r2=L_shr_v16c(a,b); check("shr_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16c r1=G_neg_v16c(a,b), r2=L_neg_v16c(a,b); check("neg_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16c r1=G_not_v16c(a,b), r2=L_not_v16c(a,b); check("not_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16c r1=G_eq_v16c(a,b), r2=L_eq_v16c(a,b); check("eq_v16c",&r1,&r2,sizeof r1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16c r1=G_lt_v16c(a,b), r2=L_lt_v16c(a,b); check("lt_v16c",&r1,&r2,sizeof r1); }
{ v16c r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v16c(r1,a,b,9); L_loop_v16c(r2,a,b,9); check("loop_v16c",r1,r2,sizeof r1); }
{ char x=(char)(rnd()%200); v16c r1=G_splat_v16c(x), r2=L_splat_v16c(x); check("splat_v16c",&r1,&r2,sizeof r1); }
{ v16c a; fill(&a,sizeof a,0); for(int i=0;i<16;i++) a[i]=(char)(int)(rnd()%200); char r1=G_lane2_v16c(a), r2=L_lane2_v16c(a); check("lane2_v16c",&r1,&r2,sizeof r1); }
{ v16c a; fill(&a,sizeof a,0); for(int i=0;i<16;i++) a[i]=(char)(int)(rnd()%200); char x=(char)(rnd()%200); v16c r1=G_setlane_v16c(a,x), r2=L_setlane_v16c(a,x); check("setlane_v16c",&r1,&r2,sizeof r1); }
{ char p[16]; fill(p,sizeof p,0); v16c r1=G_ld_v16c(p), r2=L_ld_v16c(p); check("ld_v16c",&r1,&r2,sizeof r1); }
{ v16c v; fill(&v,sizeof v,0); char p1[16],p2[16]; G_st_v16c(p1,v); L_st_v16c(p2,v); check("st_v16c",p1,p2,sizeof p1); }
{ v16c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); SIGNED_INPUT(a); SIGNED_INPUT(b); for(int c=0;c<2;c++) { v16c r1=G_cond_v16c(a,b,c), r2=L_cond_v16c(a,b,c); check("cond_v16c",&r1,&r2,sizeof r1); } }
{ v16c p[6]; for(int k=0;k<6;k++) for(int i=0;i<16;i++) p[k][i]=(char)(int)(rnd()%10); for(int n=0;n<=6;n+=3) { v16c r1=G_acc_v16c(p,n), r2=L_acc_v16c(p,n); check("acc_v16c",&r1,&r2,sizeof r1); } }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32c r1=G_add_v32c(a,b), r2=L_add_v32c(a,b); check("add_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32c r1=G_sub_v32c(a,b), r2=L_sub_v32c(a,b); check("sub_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32c r1=G_mul_v32c(a,b), r2=L_mul_v32c(a,b); check("mul_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32c r1=G_and_v32c(a,b), r2=L_and_v32c(a,b); check("and_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32c r1=G_or_v32c(a,b), r2=L_or_v32c(a,b); check("or_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32c r1=G_xor_v32c(a,b), r2=L_xor_v32c(a,b); check("xor_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v32c r1=G_shl_v32c(a,b), r2=L_shl_v32c(a,b); check("shl_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<32;i++) b[i]=(char)(((unsigned)b[i])%8);
  v32c r1=G_shr_v32c(a,b), r2=L_shr_v32c(a,b); check("shr_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32c r1=G_neg_v32c(a,b), r2=L_neg_v32c(a,b); check("neg_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32c r1=G_not_v32c(a,b), r2=L_not_v32c(a,b); check("not_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32c r1=G_eq_v32c(a,b), r2=L_eq_v32c(a,b); check("eq_v32c",&r1,&r2,sizeof r1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32c r1=G_lt_v32c(a,b), r2=L_lt_v32c(a,b); check("lt_v32c",&r1,&r2,sizeof r1); }
{ v32c r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v32c(r1,a,b,9); L_loop_v32c(r2,a,b,9); check("loop_v32c",r1,r2,sizeof r1); }
{ char x=(char)(rnd()%200); v32c r1=G_splat_v32c(x), r2=L_splat_v32c(x); check("splat_v32c",&r1,&r2,sizeof r1); }
{ v32c a; fill(&a,sizeof a,0); for(int i=0;i<32;i++) a[i]=(char)(int)(rnd()%200); char r1=G_lane2_v32c(a), r2=L_lane2_v32c(a); check("lane2_v32c",&r1,&r2,sizeof r1); }
{ v32c a; fill(&a,sizeof a,0); for(int i=0;i<32;i++) a[i]=(char)(int)(rnd()%200); char x=(char)(rnd()%200); v32c r1=G_setlane_v32c(a,x), r2=L_setlane_v32c(a,x); check("setlane_v32c",&r1,&r2,sizeof r1); }
{ char p[32]; fill(p,sizeof p,0); v32c r1=G_ld_v32c(p), r2=L_ld_v32c(p); check("ld_v32c",&r1,&r2,sizeof r1); }
{ v32c v; fill(&v,sizeof v,0); char p1[32],p2[32]; G_st_v32c(p1,v); L_st_v32c(p2,v); check("st_v32c",p1,p2,sizeof p1); }
{ v32c a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); SIGNED_INPUT(a); SIGNED_INPUT(b); for(int c=0;c<2;c++) { v32c r1=G_cond_v32c(a,b,c), r2=L_cond_v32c(a,b,c); check("cond_v32c",&r1,&r2,sizeof r1); } }
{ v32c p[6]; for(int k=0;k<6;k++) for(int i=0;i<32;i++) p[k][i]=(char)(int)(rnd()%10); for(int n=0;n<=6;n+=3) { v32c r1=G_acc_v32c(p,n), r2=L_acc_v32c(p,n); check("acc_v32c",&r1,&r2,sizeof r1); } }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16s r1=G_add_v16s(a,b), r2=L_add_v16s(a,b); check("add_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16s r1=G_sub_v16s(a,b), r2=L_sub_v16s(a,b); check("sub_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16s r1=G_mul_v16s(a,b), r2=L_mul_v16s(a,b); check("mul_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16s r1=G_and_v16s(a,b), r2=L_and_v16s(a,b); check("and_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16s r1=G_or_v16s(a,b), r2=L_or_v16s(a,b); check("or_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16s r1=G_xor_v16s(a,b), r2=L_xor_v16s(a,b); check("xor_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v16s r1=G_shl_v16s(a,b), r2=L_shl_v16s(a,b); check("shl_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) b[i]=(short)(((unsigned)b[i])%16);
  v16s r1=G_shr_v16s(a,b), r2=L_shr_v16s(a,b); check("shr_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16s r1=G_neg_v16s(a,b), r2=L_neg_v16s(a,b); check("neg_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16s r1=G_not_v16s(a,b), r2=L_not_v16s(a,b); check("not_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16s r1=G_eq_v16s(a,b), r2=L_eq_v16s(a,b); check("eq_v16s",&r1,&r2,sizeof r1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16s r1=G_lt_v16s(a,b), r2=L_lt_v16s(a,b); check("lt_v16s",&r1,&r2,sizeof r1); }
{ v16s r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v16s(r1,a,b,9); L_loop_v16s(r2,a,b,9); check("loop_v16s",r1,r2,sizeof r1); }
{ short x=(short)(rnd()%200); v16s r1=G_splat_v16s(x), r2=L_splat_v16s(x); check("splat_v16s",&r1,&r2,sizeof r1); }
{ v16s a; fill(&a,sizeof a,0); for(int i=0;i<8;i++) a[i]=(short)(int)(rnd()%200); short r1=G_lane2_v16s(a), r2=L_lane2_v16s(a); check("lane2_v16s",&r1,&r2,sizeof r1); }
{ v16s a; fill(&a,sizeof a,0); for(int i=0;i<8;i++) a[i]=(short)(int)(rnd()%200); short x=(short)(rnd()%200); v16s r1=G_setlane_v16s(a,x), r2=L_setlane_v16s(a,x); check("setlane_v16s",&r1,&r2,sizeof r1); }
{ short p[8]; fill(p,sizeof p,0); v16s r1=G_ld_v16s(p), r2=L_ld_v16s(p); check("ld_v16s",&r1,&r2,sizeof r1); }
{ v16s v; fill(&v,sizeof v,0); short p1[8],p2[8]; G_st_v16s(p1,v); L_st_v16s(p2,v); check("st_v16s",p1,p2,sizeof p1); }
{ v16s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<8;i++) { a[i]=(short)(int)(rnd()%200); b[i]=(short)(int)(rnd()%200); } for(int c=0;c<2;c++) { v16s r1=G_cond_v16s(a,b,c), r2=L_cond_v16s(a,b,c); check("cond_v16s",&r1,&r2,sizeof r1); } }
{ v16s p[6]; for(int k=0;k<6;k++) for(int i=0;i<8;i++) p[k][i]=(short)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v16s r1=G_acc_v16s(p,n), r2=L_acc_v16s(p,n); check("acc_v16s",&r1,&r2,sizeof r1); } }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32s r1=G_add_v32s(a,b), r2=L_add_v32s(a,b); check("add_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32s r1=G_sub_v32s(a,b), r2=L_sub_v32s(a,b); check("sub_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32s r1=G_mul_v32s(a,b), r2=L_mul_v32s(a,b); check("mul_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32s r1=G_and_v32s(a,b), r2=L_and_v32s(a,b); check("and_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32s r1=G_or_v32s(a,b), r2=L_or_v32s(a,b); check("or_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32s r1=G_xor_v32s(a,b), r2=L_xor_v32s(a,b); check("xor_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v32s r1=G_shl_v32s(a,b), r2=L_shl_v32s(a,b); check("shl_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<16;i++) b[i]=(short)(((unsigned)b[i])%16);
  v32s r1=G_shr_v32s(a,b), r2=L_shr_v32s(a,b); check("shr_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32s r1=G_neg_v32s(a,b), r2=L_neg_v32s(a,b); check("neg_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32s r1=G_not_v32s(a,b), r2=L_not_v32s(a,b); check("not_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32s r1=G_eq_v32s(a,b), r2=L_eq_v32s(a,b); check("eq_v32s",&r1,&r2,sizeof r1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32s r1=G_lt_v32s(a,b), r2=L_lt_v32s(a,b); check("lt_v32s",&r1,&r2,sizeof r1); }
{ v32s r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v32s(r1,a,b,9); L_loop_v32s(r2,a,b,9); check("loop_v32s",r1,r2,sizeof r1); }
{ short x=(short)(rnd()%200); v32s r1=G_splat_v32s(x), r2=L_splat_v32s(x); check("splat_v32s",&r1,&r2,sizeof r1); }
{ v32s a; fill(&a,sizeof a,0); for(int i=0;i<16;i++) a[i]=(short)(int)(rnd()%200); short r1=G_lane2_v32s(a), r2=L_lane2_v32s(a); check("lane2_v32s",&r1,&r2,sizeof r1); }
{ v32s a; fill(&a,sizeof a,0); for(int i=0;i<16;i++) a[i]=(short)(int)(rnd()%200); short x=(short)(rnd()%200); v32s r1=G_setlane_v32s(a,x), r2=L_setlane_v32s(a,x); check("setlane_v32s",&r1,&r2,sizeof r1); }
{ short p[16]; fill(p,sizeof p,0); v32s r1=G_ld_v32s(p), r2=L_ld_v32s(p); check("ld_v32s",&r1,&r2,sizeof r1); }
{ v32s v; fill(&v,sizeof v,0); short p1[16],p2[16]; G_st_v32s(p1,v); L_st_v32s(p2,v); check("st_v32s",p1,p2,sizeof p1); }
{ v32s a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<16;i++) { a[i]=(short)(int)(rnd()%200); b[i]=(short)(int)(rnd()%200); } for(int c=0;c<2;c++) { v32s r1=G_cond_v32s(a,b,c), r2=L_cond_v32s(a,b,c); check("cond_v32s",&r1,&r2,sizeof r1); } }
{ v32s p[6]; for(int k=0;k<6;k++) for(int i=0;i<16;i++) p[k][i]=(short)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v32s r1=G_acc_v32s(p,n), r2=L_acc_v32s(p,n); check("acc_v32s",&r1,&r2,sizeof r1); } }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16i r1=G_add_v16i(a,b), r2=L_add_v16i(a,b); check("add_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16i r1=G_sub_v16i(a,b), r2=L_sub_v16i(a,b); check("sub_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16i r1=G_mul_v16i(a,b), r2=L_mul_v16i(a,b); check("mul_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16i r1=G_and_v16i(a,b), r2=L_and_v16i(a,b); check("and_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16i r1=G_or_v16i(a,b), r2=L_or_v16i(a,b); check("or_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16i r1=G_xor_v16i(a,b), r2=L_xor_v16i(a,b); check("xor_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v16i r1=G_shl_v16i(a,b), r2=L_shl_v16i(a,b); check("shl_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) b[i]=(int)(((unsigned)b[i])%32);
  v16i r1=G_shr_v16i(a,b), r2=L_shr_v16i(a,b); check("shr_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16i r1=G_neg_v16i(a,b), r2=L_neg_v16i(a,b); check("neg_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16i r1=G_not_v16i(a,b), r2=L_not_v16i(a,b); check("not_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16i r1=G_eq_v16i(a,b), r2=L_eq_v16i(a,b); check("eq_v16i",&r1,&r2,sizeof r1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16i r1=G_lt_v16i(a,b), r2=L_lt_v16i(a,b); check("lt_v16i",&r1,&r2,sizeof r1); }
{ v16i r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v16i(r1,a,b,9); L_loop_v16i(r2,a,b,9); check("loop_v16i",r1,r2,sizeof r1); }
{ int x=(int)(rnd()%200); v16i r1=G_splat_v16i(x), r2=L_splat_v16i(x); check("splat_v16i",&r1,&r2,sizeof r1); }
{ v16i a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(int)(int)(rnd()%200); int r1=G_lane2_v16i(a), r2=L_lane2_v16i(a); check("lane2_v16i",&r1,&r2,sizeof r1); }
{ v16i a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(int)(int)(rnd()%200); int x=(int)(rnd()%200); v16i r1=G_setlane_v16i(a,x), r2=L_setlane_v16i(a,x); check("setlane_v16i",&r1,&r2,sizeof r1); }
{ int p[4]; fill(p,sizeof p,0); v16i r1=G_ld_v16i(p), r2=L_ld_v16i(p); check("ld_v16i",&r1,&r2,sizeof r1); }
{ v16i v; fill(&v,sizeof v,0); int p1[4],p2[4]; G_st_v16i(p1,v); L_st_v16i(p2,v); check("st_v16i",p1,p2,sizeof p1); }
{ v16i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<4;i++) { a[i]=(int)(int)(rnd()%200); b[i]=(int)(int)(rnd()%200); } for(int c=0;c<2;c++) { v16i r1=G_cond_v16i(a,b,c), r2=L_cond_v16i(a,b,c); check("cond_v16i",&r1,&r2,sizeof r1); } }
{ v16i p[6]; for(int k=0;k<6;k++) for(int i=0;i<4;i++) p[k][i]=(int)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v16i r1=G_acc_v16i(p,n), r2=L_acc_v16i(p,n); check("acc_v16i",&r1,&r2,sizeof r1); } }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32i r1=G_add_v32i(a,b), r2=L_add_v32i(a,b); check("add_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32i r1=G_sub_v32i(a,b), r2=L_sub_v32i(a,b); check("sub_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32i r1=G_mul_v32i(a,b), r2=L_mul_v32i(a,b); check("mul_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32i r1=G_and_v32i(a,b), r2=L_and_v32i(a,b); check("and_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32i r1=G_or_v32i(a,b), r2=L_or_v32i(a,b); check("or_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32i r1=G_xor_v32i(a,b), r2=L_xor_v32i(a,b); check("xor_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v32i r1=G_shl_v32i(a,b), r2=L_shl_v32i(a,b); check("shl_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) b[i]=(int)(((unsigned)b[i])%32);
  v32i r1=G_shr_v32i(a,b), r2=L_shr_v32i(a,b); check("shr_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32i r1=G_neg_v32i(a,b), r2=L_neg_v32i(a,b); check("neg_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32i r1=G_not_v32i(a,b), r2=L_not_v32i(a,b); check("not_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32i r1=G_eq_v32i(a,b), r2=L_eq_v32i(a,b); check("eq_v32i",&r1,&r2,sizeof r1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32i r1=G_lt_v32i(a,b), r2=L_lt_v32i(a,b); check("lt_v32i",&r1,&r2,sizeof r1); }
{ v32i r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v32i(r1,a,b,9); L_loop_v32i(r2,a,b,9); check("loop_v32i",r1,r2,sizeof r1); }
{ int x=(int)(rnd()%200); v32i r1=G_splat_v32i(x), r2=L_splat_v32i(x); check("splat_v32i",&r1,&r2,sizeof r1); }
{ v32i a; fill(&a,sizeof a,0); for(int i=0;i<8;i++) a[i]=(int)(int)(rnd()%200); int r1=G_lane2_v32i(a), r2=L_lane2_v32i(a); check("lane2_v32i",&r1,&r2,sizeof r1); }
{ v32i a; fill(&a,sizeof a,0); for(int i=0;i<8;i++) a[i]=(int)(int)(rnd()%200); int x=(int)(rnd()%200); v32i r1=G_setlane_v32i(a,x), r2=L_setlane_v32i(a,x); check("setlane_v32i",&r1,&r2,sizeof r1); }
{ int p[8]; fill(p,sizeof p,0); v32i r1=G_ld_v32i(p), r2=L_ld_v32i(p); check("ld_v32i",&r1,&r2,sizeof r1); }
{ v32i v; fill(&v,sizeof v,0); int p1[8],p2[8]; G_st_v32i(p1,v); L_st_v32i(p2,v); check("st_v32i",p1,p2,sizeof p1); }
{ v32i a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<8;i++) { a[i]=(int)(int)(rnd()%200); b[i]=(int)(int)(rnd()%200); } for(int c=0;c<2;c++) { v32i r1=G_cond_v32i(a,b,c), r2=L_cond_v32i(a,b,c); check("cond_v32i",&r1,&r2,sizeof r1); } }
{ v32i p[6]; for(int k=0;k<6;k++) for(int i=0;i<8;i++) p[k][i]=(int)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v32i r1=G_acc_v32i(p,n), r2=L_acc_v32i(p,n); check("acc_v32i",&r1,&r2,sizeof r1); } }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16l r1=G_add_v16l(a,b), r2=L_add_v16l(a,b); check("add_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16l r1=G_sub_v16l(a,b), r2=L_sub_v16l(a,b); check("sub_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16l r1=G_mul_v16l(a,b), r2=L_mul_v16l(a,b); check("mul_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16l r1=G_and_v16l(a,b), r2=L_and_v16l(a,b); check("and_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16l r1=G_or_v16l(a,b), r2=L_or_v16l(a,b); check("or_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16l r1=G_xor_v16l(a,b), r2=L_xor_v16l(a,b); check("xor_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v16l r1=G_shl_v16l(a,b), r2=L_shl_v16l(a,b); check("shl_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) b[i]=(long)(((unsigned)b[i])%64);
  v16l r1=G_shr_v16l(a,b), r2=L_shr_v16l(a,b); check("shr_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v16l r1=G_neg_v16l(a,b), r2=L_neg_v16l(a,b); check("neg_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16l r1=G_not_v16l(a,b), r2=L_not_v16l(a,b); check("not_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16l r1=G_eq_v16l(a,b), r2=L_eq_v16l(a,b); check("eq_v16l",&r1,&r2,sizeof r1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v16l r1=G_lt_v16l(a,b), r2=L_lt_v16l(a,b); check("lt_v16l",&r1,&r2,sizeof r1); }
{ v16l r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v16l(r1,a,b,9); L_loop_v16l(r2,a,b,9); check("loop_v16l",r1,r2,sizeof r1); }
{ long x=(long)(rnd()%200); v16l r1=G_splat_v16l(x), r2=L_splat_v16l(x); check("splat_v16l",&r1,&r2,sizeof r1); }
{ v16l a; fill(&a,sizeof a,0); for(int i=0;i<2;i++) a[i]=(long)(int)(rnd()%200); long r1=G_lane1_v16l(a), r2=L_lane1_v16l(a); check("lane1_v16l",&r1,&r2,sizeof r1); }
{ v16l a; fill(&a,sizeof a,0); for(int i=0;i<2;i++) a[i]=(long)(int)(rnd()%200); long x=(long)(rnd()%200); v16l r1=G_setlane_v16l(a,x), r2=L_setlane_v16l(a,x); check("setlane_v16l",&r1,&r2,sizeof r1); }
{ long p[2]; fill(p,sizeof p,0); v16l r1=G_ld_v16l(p), r2=L_ld_v16l(p); check("ld_v16l",&r1,&r2,sizeof r1); }
{ v16l v; fill(&v,sizeof v,0); long p1[2],p2[2]; G_st_v16l(p1,v); L_st_v16l(p2,v); check("st_v16l",p1,p2,sizeof p1); }
{ v16l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<2;i++) { a[i]=(long)(int)(rnd()%200); b[i]=(long)(int)(rnd()%200); } for(int c=0;c<2;c++) { v16l r1=G_cond_v16l(a,b,c), r2=L_cond_v16l(a,b,c); check("cond_v16l",&r1,&r2,sizeof r1); } }
{ v16l p[6]; for(int k=0;k<6;k++) for(int i=0;i<2;i++) p[k][i]=(long)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v16l r1=G_acc_v16l(p,n), r2=L_acc_v16l(p,n); check("acc_v16l",&r1,&r2,sizeof r1); } }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32l r1=G_add_v32l(a,b), r2=L_add_v32l(a,b); check("add_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32l r1=G_sub_v32l(a,b), r2=L_sub_v32l(a,b); check("sub_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32l r1=G_mul_v32l(a,b), r2=L_mul_v32l(a,b); check("mul_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32l r1=G_and_v32l(a,b), r2=L_and_v32l(a,b); check("and_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32l r1=G_or_v32l(a,b), r2=L_or_v32l(a,b); check("or_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32l r1=G_xor_v32l(a,b), r2=L_xor_v32l(a,b); check("xor_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_LEFT_INPUT(a,b);
  v32l r1=G_shl_v32l(a,b), r2=L_shl_v32l(a,b); check("shl_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) b[i]=(long)(((unsigned)b[i])%64);
  v32l r1=G_shr_v32l(a,b), r2=L_shr_v32l(a,b); check("shr_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  SIGNED_INPUT(a); SIGNED_INPUT(b);
  v32l r1=G_neg_v32l(a,b), r2=L_neg_v32l(a,b); check("neg_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32l r1=G_not_v32l(a,b), r2=L_not_v32l(a,b); check("not_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32l r1=G_eq_v32l(a,b), r2=L_eq_v32l(a,b); check("eq_v32l",&r1,&r2,sizeof r1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  v32l r1=G_lt_v32l(a,b), r2=L_lt_v32l(a,b); check("lt_v32l",&r1,&r2,sizeof r1); }
{ v32l r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0);
  for(int k=0;k<9;k++) { SIGNED_INPUT(a[k]); SIGNED_INPUT(b[k]); SIGNED_INPUT(r1[k]); }
  memcpy(r2,r1,sizeof r1);
  G_loop_v32l(r1,a,b,9); L_loop_v32l(r2,a,b,9); check("loop_v32l",r1,r2,sizeof r1); }
{ long x=(long)(rnd()%200); v32l r1=G_splat_v32l(x), r2=L_splat_v32l(x); check("splat_v32l",&r1,&r2,sizeof r1); }
{ v32l a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(long)(int)(rnd()%200); long r1=G_lane2_v32l(a), r2=L_lane2_v32l(a); check("lane2_v32l",&r1,&r2,sizeof r1); }
{ v32l a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(long)(int)(rnd()%200); long x=(long)(rnd()%200); v32l r1=G_setlane_v32l(a,x), r2=L_setlane_v32l(a,x); check("setlane_v32l",&r1,&r2,sizeof r1); }
{ long p[4]; fill(p,sizeof p,0); v32l r1=G_ld_v32l(p), r2=L_ld_v32l(p); check("ld_v32l",&r1,&r2,sizeof r1); }
{ v32l v; fill(&v,sizeof v,0); long p1[4],p2[4]; G_st_v32l(p1,v); L_st_v32l(p2,v); check("st_v32l",p1,p2,sizeof p1); }
{ v32l a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<4;i++) { a[i]=(long)(int)(rnd()%200); b[i]=(long)(int)(rnd()%200); } for(int c=0;c<2;c++) { v32l r1=G_cond_v32l(a,b,c), r2=L_cond_v32l(a,b,c); check("cond_v32l",&r1,&r2,sizeof r1); } }
{ v32l p[6]; for(int k=0;k<6;k++) for(int i=0;i<4;i++) p[k][i]=(long)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v32l r1=G_acc_v32l(p,n), r2=L_acc_v32l(p,n); check("acc_v32l",&r1,&r2,sizeof r1); } }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16f r1=G_add_v16f(a,b), r2=L_add_v16f(a,b); check("add_v16f",&r1,&r2,sizeof r1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16f r1=G_sub_v16f(a,b), r2=L_sub_v16f(a,b); check("sub_v16f",&r1,&r2,sizeof r1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16f r1=G_mul_v16f(a,b), r2=L_mul_v16f(a,b); check("mul_v16f",&r1,&r2,sizeof r1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) a[i]=(float)((int)rnd()%1000)/7.0;
  v16f r1=G_neg_v16f(a,b), r2=L_neg_v16f(a,b); check("neg_v16f",&r1,&r2,sizeof r1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16i r1=G_eq_v16f(a,b), r2=L_eq_v16f(a,b); check("eq_v16f",&r1,&r2,sizeof r1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16i r1=G_lt_v16f(a,b), r2=L_lt_v16f(a,b); check("lt_v16f",&r1,&r2,sizeof r1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { if(b[i]==0) b[i]=3; if(a[i]==(float)(1ull<<31) && b[i]==(float)-1) b[i]=2; }
  for(int i=0;i<4;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16f r1=G_div_v16f(a,b), r2=L_div_v16f(a,b); check("div_v16f",&r1,&r2,sizeof r1); }
{ v16f r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0); memcpy(r2,r1,sizeof r1);
  for(int k=0;k<9;k++) for(int i=0;i<4;i++) { a[k][i]=(float)((int)rnd()%100); b[k][i]=(float)((int)rnd()%100); r1[k][i]=r2[k][i]=(float)((int)rnd()%100); }
  G_loop_v16f(r1,a,b,9); L_loop_v16f(r2,a,b,9); check("loop_v16f",r1,r2,sizeof r1); }
{ float x=(float)(rnd()%200); v16f r1=G_splat_v16f(x), r2=L_splat_v16f(x); check("splat_v16f",&r1,&r2,sizeof r1); }
{ v16f a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(float)(int)(rnd()%200); float r1=G_lane2_v16f(a), r2=L_lane2_v16f(a); check("lane2_v16f",&r1,&r2,sizeof r1); }
{ v16f a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(float)(int)(rnd()%200); float x=(float)(rnd()%200); v16f r1=G_setlane_v16f(a,x), r2=L_setlane_v16f(a,x); check("setlane_v16f",&r1,&r2,sizeof r1); }
{ float p[4]; fill(p,sizeof p,0); v16f r1=G_ld_v16f(p), r2=L_ld_v16f(p); check("ld_v16f",&r1,&r2,sizeof r1); }
{ v16f v; fill(&v,sizeof v,0); float p1[4],p2[4]; G_st_v16f(p1,v); L_st_v16f(p2,v); check("st_v16f",p1,p2,sizeof p1); }
{ v16f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<4;i++) { a[i]=(float)(int)(rnd()%200); b[i]=(float)(int)(rnd()%200); } for(int c=0;c<2;c++) { v16f r1=G_cond_v16f(a,b,c), r2=L_cond_v16f(a,b,c); check("cond_v16f",&r1,&r2,sizeof r1); } }
{ v16f p[6]; for(int k=0;k<6;k++) for(int i=0;i<4;i++) p[k][i]=(float)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v16f r1=G_acc_v16f(p,n), r2=L_acc_v16f(p,n); check("acc_v16f",&r1,&r2,sizeof r1); } }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32f r1=G_add_v32f(a,b), r2=L_add_v32f(a,b); check("add_v32f",&r1,&r2,sizeof r1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32f r1=G_sub_v32f(a,b), r2=L_sub_v32f(a,b); check("sub_v32f",&r1,&r2,sizeof r1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32f r1=G_mul_v32f(a,b), r2=L_mul_v32f(a,b); check("mul_v32f",&r1,&r2,sizeof r1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) a[i]=(float)((int)rnd()%1000)/7.0;
  v32f r1=G_neg_v32f(a,b), r2=L_neg_v32f(a,b); check("neg_v32f",&r1,&r2,sizeof r1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32i r1=G_eq_v32f(a,b), r2=L_eq_v32f(a,b); check("eq_v32f",&r1,&r2,sizeof r1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32i r1=G_lt_v32f(a,b), r2=L_lt_v32f(a,b); check("lt_v32f",&r1,&r2,sizeof r1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<8;i++) { if(b[i]==0) b[i]=3; if(a[i]==(float)(1ull<<31) && b[i]==(float)-1) b[i]=2; }
  for(int i=0;i<8;i++) { a[i]=(float)((int)rnd()%1000)/7.0; b[i]=(float)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32f r1=G_div_v32f(a,b), r2=L_div_v32f(a,b); check("div_v32f",&r1,&r2,sizeof r1); }
{ v32f r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0); memcpy(r2,r1,sizeof r1);
  for(int k=0;k<9;k++) for(int i=0;i<8;i++) { a[k][i]=(float)((int)rnd()%100); b[k][i]=(float)((int)rnd()%100); r1[k][i]=r2[k][i]=(float)((int)rnd()%100); }
  G_loop_v32f(r1,a,b,9); L_loop_v32f(r2,a,b,9); check("loop_v32f",r1,r2,sizeof r1); }
{ float x=(float)(rnd()%200); v32f r1=G_splat_v32f(x), r2=L_splat_v32f(x); check("splat_v32f",&r1,&r2,sizeof r1); }
{ v32f a; fill(&a,sizeof a,0); for(int i=0;i<8;i++) a[i]=(float)(int)(rnd()%200); float r1=G_lane2_v32f(a), r2=L_lane2_v32f(a); check("lane2_v32f",&r1,&r2,sizeof r1); }
{ v32f a; fill(&a,sizeof a,0); for(int i=0;i<8;i++) a[i]=(float)(int)(rnd()%200); float x=(float)(rnd()%200); v32f r1=G_setlane_v32f(a,x), r2=L_setlane_v32f(a,x); check("setlane_v32f",&r1,&r2,sizeof r1); }
{ float p[8]; fill(p,sizeof p,0); v32f r1=G_ld_v32f(p), r2=L_ld_v32f(p); check("ld_v32f",&r1,&r2,sizeof r1); }
{ v32f v; fill(&v,sizeof v,0); float p1[8],p2[8]; G_st_v32f(p1,v); L_st_v32f(p2,v); check("st_v32f",p1,p2,sizeof p1); }
{ v32f a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<8;i++) { a[i]=(float)(int)(rnd()%200); b[i]=(float)(int)(rnd()%200); } for(int c=0;c<2;c++) { v32f r1=G_cond_v32f(a,b,c), r2=L_cond_v32f(a,b,c); check("cond_v32f",&r1,&r2,sizeof r1); } }
{ v32f p[6]; for(int k=0;k<6;k++) for(int i=0;i<8;i++) p[k][i]=(float)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v32f r1=G_acc_v32f(p,n), r2=L_acc_v32f(p,n); check("acc_v32f",&r1,&r2,sizeof r1); } }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16d r1=G_add_v16d(a,b), r2=L_add_v16d(a,b); check("add_v16d",&r1,&r2,sizeof r1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16d r1=G_sub_v16d(a,b), r2=L_sub_v16d(a,b); check("sub_v16d",&r1,&r2,sizeof r1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16d r1=G_mul_v16d(a,b), r2=L_mul_v16d(a,b); check("mul_v16d",&r1,&r2,sizeof r1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) a[i]=(double)((int)rnd()%1000)/7.0;
  v16d r1=G_neg_v16d(a,b), r2=L_neg_v16d(a,b); check("neg_v16d",&r1,&r2,sizeof r1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16l r1=G_eq_v16d(a,b), r2=L_eq_v16d(a,b); check("eq_v16d",&r1,&r2,sizeof r1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16l r1=G_lt_v16d(a,b), r2=L_lt_v16d(a,b); check("lt_v16d",&r1,&r2,sizeof r1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<2;i++) { if(b[i]==0) b[i]=3; if(a[i]==(double)(1ull<<63) && b[i]==(double)-1) b[i]=2; }
  for(int i=0;i<2;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v16d r1=G_div_v16d(a,b), r2=L_div_v16d(a,b); check("div_v16d",&r1,&r2,sizeof r1); }
{ v16d r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0); memcpy(r2,r1,sizeof r1);
  for(int k=0;k<9;k++) for(int i=0;i<2;i++) { a[k][i]=(double)((int)rnd()%100); b[k][i]=(double)((int)rnd()%100); r1[k][i]=r2[k][i]=(double)((int)rnd()%100); }
  G_loop_v16d(r1,a,b,9); L_loop_v16d(r2,a,b,9); check("loop_v16d",r1,r2,sizeof r1); }
{ double x=(double)(rnd()%200); v16d r1=G_splat_v16d(x), r2=L_splat_v16d(x); check("splat_v16d",&r1,&r2,sizeof r1); }
{ v16d a; fill(&a,sizeof a,0); for(int i=0;i<2;i++) a[i]=(double)(int)(rnd()%200); double r1=G_lane1_v16d(a), r2=L_lane1_v16d(a); check("lane1_v16d",&r1,&r2,sizeof r1); }
{ v16d a; fill(&a,sizeof a,0); for(int i=0;i<2;i++) a[i]=(double)(int)(rnd()%200); double x=(double)(rnd()%200); v16d r1=G_setlane_v16d(a,x), r2=L_setlane_v16d(a,x); check("setlane_v16d",&r1,&r2,sizeof r1); }
{ double p[2]; fill(p,sizeof p,0); v16d r1=G_ld_v16d(p), r2=L_ld_v16d(p); check("ld_v16d",&r1,&r2,sizeof r1); }
{ v16d v; fill(&v,sizeof v,0); double p1[2],p2[2]; G_st_v16d(p1,v); L_st_v16d(p2,v); check("st_v16d",p1,p2,sizeof p1); }
{ v16d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<2;i++) { a[i]=(double)(int)(rnd()%200); b[i]=(double)(int)(rnd()%200); } for(int c=0;c<2;c++) { v16d r1=G_cond_v16d(a,b,c), r2=L_cond_v16d(a,b,c); check("cond_v16d",&r1,&r2,sizeof r1); } }
{ v16d p[6]; for(int k=0;k<6;k++) for(int i=0;i<2;i++) p[k][i]=(double)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v16d r1=G_acc_v16d(p,n), r2=L_acc_v16d(p,n); check("acc_v16d",&r1,&r2,sizeof r1); } }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32d r1=G_add_v32d(a,b), r2=L_add_v32d(a,b); check("add_v32d",&r1,&r2,sizeof r1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32d r1=G_sub_v32d(a,b), r2=L_sub_v32d(a,b); check("sub_v32d",&r1,&r2,sizeof r1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32d r1=G_mul_v32d(a,b), r2=L_mul_v32d(a,b); check("mul_v32d",&r1,&r2,sizeof r1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) a[i]=(double)((int)rnd()%1000)/7.0;
  v32d r1=G_neg_v32d(a,b), r2=L_neg_v32d(a,b); check("neg_v32d",&r1,&r2,sizeof r1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32l r1=G_eq_v32d(a,b), r2=L_eq_v32d(a,b); check("eq_v32d",&r1,&r2,sizeof r1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32l r1=G_lt_v32d(a,b), r2=L_lt_v32d(a,b); check("lt_v32d",&r1,&r2,sizeof r1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);
  for(int i=0;i<4;i++) { if(b[i]==0) b[i]=3; if(a[i]==(double)(1ull<<63) && b[i]==(double)-1) b[i]=2; }
  for(int i=0;i<4;i++) { a[i]=(double)((int)rnd()%1000)/7.0; b[i]=(double)((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }
  v32d r1=G_div_v32d(a,b), r2=L_div_v32d(a,b); check("div_v32d",&r1,&r2,sizeof r1); }
{ v32d r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0); memcpy(r2,r1,sizeof r1);
  for(int k=0;k<9;k++) for(int i=0;i<4;i++) { a[k][i]=(double)((int)rnd()%100); b[k][i]=(double)((int)rnd()%100); r1[k][i]=r2[k][i]=(double)((int)rnd()%100); }
  G_loop_v32d(r1,a,b,9); L_loop_v32d(r2,a,b,9); check("loop_v32d",r1,r2,sizeof r1); }
{ double x=(double)(rnd()%200); v32d r1=G_splat_v32d(x), r2=L_splat_v32d(x); check("splat_v32d",&r1,&r2,sizeof r1); }
{ v32d a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(double)(int)(rnd()%200); double r1=G_lane2_v32d(a), r2=L_lane2_v32d(a); check("lane2_v32d",&r1,&r2,sizeof r1); }
{ v32d a; fill(&a,sizeof a,0); for(int i=0;i<4;i++) a[i]=(double)(int)(rnd()%200); double x=(double)(rnd()%200); v32d r1=G_setlane_v32d(a,x), r2=L_setlane_v32d(a,x); check("setlane_v32d",&r1,&r2,sizeof r1); }
{ double p[4]; fill(p,sizeof p,0); v32d r1=G_ld_v32d(p), r2=L_ld_v32d(p); check("ld_v32d",&r1,&r2,sizeof r1); }
{ v32d v; fill(&v,sizeof v,0); double p1[4],p2[4]; G_st_v32d(p1,v); L_st_v32d(p2,v); check("st_v32d",p1,p2,sizeof p1); }
{ v32d a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<4;i++) { a[i]=(double)(int)(rnd()%200); b[i]=(double)(int)(rnd()%200); } for(int c=0;c<2;c++) { v32d r1=G_cond_v32d(a,b,c), r2=L_cond_v32d(a,b,c); check("cond_v32d",&r1,&r2,sizeof r1); } }
{ v32d p[6]; for(int k=0;k<6;k++) for(int i=0;i<4;i++) p[k][i]=(double)(int)(rnd()%100); for(int n=0;n<=6;n+=3) { v32d r1=G_acc_v32d(p,n), r2=L_acc_v32d(p,n); check("acc_v32d",&r1,&r2,sizeof r1); } }
{ struct S8f a,b; for(int i=0;i<8;i++){a.v[i]=(float)(int)(rnd()%100); b.v[i]=(float)(int)(rnd()%100);} struct S8f r1=G_sadd(a,b), r2=L_sadd(a,b); check("sadd",&r1,&r2,sizeof r1); }
{ v32f a,b; for(int i=0;i<8;i++){a[i]=(float)(int)(rnd()%100); b[i]=(float)(int)(rnd()%100);} v32f r1=G_viacall(a,b), r2=L_viacall(a,b); check("viacall",&r1,&r2,sizeof r1); }
printf("%d/%d checks passed\n", total-fails, total); return fails!=0; }
