typedef char v16c __attribute__((vector_size(16)));
v16c add_v16c(v16c a,v16c b){return a+b;}
v16c sub_v16c(v16c a,v16c b){return a-b;}
v16c mul_v16c(v16c a,v16c b){return a*b;}
v16c and_v16c(v16c a,v16c b){return a&b;}
v16c or_v16c(v16c a,v16c b){return a|b;}
v16c xor_v16c(v16c a,v16c b){return a^b;}
v16c shl_v16c(v16c a,v16c b){return a<<b;}
v16c shr_v16c(v16c a,v16c b){return a>>b;}
v16c neg_v16c(v16c a,v16c b){return -a;}
v16c not_v16c(v16c a,v16c b){return ~a;}
v16c eq_v16c(v16c a,v16c b){return a==b;}
v16c lt_v16c(v16c a,v16c b){return a<b;}
void loop_v16c(v16c*r,const v16c*a,const v16c*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v16c splat_v16c(char x){return (v16c){}+x;}
char lane2_v16c(v16c a){return a[2];}
v16c setlane_v16c(v16c a,char x){a[1]=x;return a;}
v16c ld_v16c(const char*p){v16c v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v16c(char*p,v16c v){__builtin_memcpy(p,&v,sizeof v);}
v16c cond_v16c(v16c a,v16c b,int c){return c?a+b:a-b;}
v16c acc_v16c(const v16c*p,int n){v16c s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef char v32c __attribute__((vector_size(32)));
v32c add_v32c(v32c a,v32c b){return a+b;}
v32c sub_v32c(v32c a,v32c b){return a-b;}
v32c mul_v32c(v32c a,v32c b){return a*b;}
v32c and_v32c(v32c a,v32c b){return a&b;}
v32c or_v32c(v32c a,v32c b){return a|b;}
v32c xor_v32c(v32c a,v32c b){return a^b;}
v32c shl_v32c(v32c a,v32c b){return a<<b;}
v32c shr_v32c(v32c a,v32c b){return a>>b;}
v32c neg_v32c(v32c a,v32c b){return -a;}
v32c not_v32c(v32c a,v32c b){return ~a;}
v32c eq_v32c(v32c a,v32c b){return a==b;}
v32c lt_v32c(v32c a,v32c b){return a<b;}
void loop_v32c(v32c*r,const v32c*a,const v32c*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v32c splat_v32c(char x){return (v32c){}+x;}
char lane2_v32c(v32c a){return a[2];}
v32c setlane_v32c(v32c a,char x){a[1]=x;return a;}
v32c ld_v32c(const char*p){v32c v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v32c(char*p,v32c v){__builtin_memcpy(p,&v,sizeof v);}
v32c cond_v32c(v32c a,v32c b,int c){return c?a+b:a-b;}
v32c acc_v32c(const v32c*p,int n){v32c s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef short v16s __attribute__((vector_size(16)));
v16s add_v16s(v16s a,v16s b){return a+b;}
v16s sub_v16s(v16s a,v16s b){return a-b;}
v16s mul_v16s(v16s a,v16s b){return a*b;}
v16s and_v16s(v16s a,v16s b){return a&b;}
v16s or_v16s(v16s a,v16s b){return a|b;}
v16s xor_v16s(v16s a,v16s b){return a^b;}
v16s shl_v16s(v16s a,v16s b){return a<<b;}
v16s shr_v16s(v16s a,v16s b){return a>>b;}
v16s neg_v16s(v16s a,v16s b){return -a;}
v16s not_v16s(v16s a,v16s b){return ~a;}
v16s eq_v16s(v16s a,v16s b){return a==b;}
v16s lt_v16s(v16s a,v16s b){return a<b;}
void loop_v16s(v16s*r,const v16s*a,const v16s*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v16s splat_v16s(short x){return (v16s){}+x;}
short lane2_v16s(v16s a){return a[2];}
v16s setlane_v16s(v16s a,short x){a[1]=x;return a;}
v16s ld_v16s(const short*p){v16s v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v16s(short*p,v16s v){__builtin_memcpy(p,&v,sizeof v);}
v16s cond_v16s(v16s a,v16s b,int c){return c?a+b:a-b;}
v16s acc_v16s(const v16s*p,int n){v16s s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef short v32s __attribute__((vector_size(32)));
v32s add_v32s(v32s a,v32s b){return a+b;}
v32s sub_v32s(v32s a,v32s b){return a-b;}
v32s mul_v32s(v32s a,v32s b){return a*b;}
v32s and_v32s(v32s a,v32s b){return a&b;}
v32s or_v32s(v32s a,v32s b){return a|b;}
v32s xor_v32s(v32s a,v32s b){return a^b;}
v32s shl_v32s(v32s a,v32s b){return a<<b;}
v32s shr_v32s(v32s a,v32s b){return a>>b;}
v32s neg_v32s(v32s a,v32s b){return -a;}
v32s not_v32s(v32s a,v32s b){return ~a;}
v32s eq_v32s(v32s a,v32s b){return a==b;}
v32s lt_v32s(v32s a,v32s b){return a<b;}
void loop_v32s(v32s*r,const v32s*a,const v32s*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v32s splat_v32s(short x){return (v32s){}+x;}
short lane2_v32s(v32s a){return a[2];}
v32s setlane_v32s(v32s a,short x){a[1]=x;return a;}
v32s ld_v32s(const short*p){v32s v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v32s(short*p,v32s v){__builtin_memcpy(p,&v,sizeof v);}
v32s cond_v32s(v32s a,v32s b,int c){return c?a+b:a-b;}
v32s acc_v32s(const v32s*p,int n){v32s s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef int v16i __attribute__((vector_size(16)));
v16i add_v16i(v16i a,v16i b){return a+b;}
v16i sub_v16i(v16i a,v16i b){return a-b;}
v16i mul_v16i(v16i a,v16i b){return a*b;}
v16i and_v16i(v16i a,v16i b){return a&b;}
v16i or_v16i(v16i a,v16i b){return a|b;}
v16i xor_v16i(v16i a,v16i b){return a^b;}
v16i shl_v16i(v16i a,v16i b){return a<<b;}
v16i shr_v16i(v16i a,v16i b){return a>>b;}
v16i neg_v16i(v16i a,v16i b){return -a;}
v16i not_v16i(v16i a,v16i b){return ~a;}
v16i eq_v16i(v16i a,v16i b){return a==b;}
v16i lt_v16i(v16i a,v16i b){return a<b;}
void loop_v16i(v16i*r,const v16i*a,const v16i*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v16i splat_v16i(int x){return (v16i){}+x;}
int lane2_v16i(v16i a){return a[2];}
v16i setlane_v16i(v16i a,int x){a[1]=x;return a;}
v16i ld_v16i(const int*p){v16i v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v16i(int*p,v16i v){__builtin_memcpy(p,&v,sizeof v);}
v16i cond_v16i(v16i a,v16i b,int c){return c?a+b:a-b;}
v16i acc_v16i(const v16i*p,int n){v16i s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef int v32i __attribute__((vector_size(32)));
v32i add_v32i(v32i a,v32i b){return a+b;}
v32i sub_v32i(v32i a,v32i b){return a-b;}
v32i mul_v32i(v32i a,v32i b){return a*b;}
v32i and_v32i(v32i a,v32i b){return a&b;}
v32i or_v32i(v32i a,v32i b){return a|b;}
v32i xor_v32i(v32i a,v32i b){return a^b;}
v32i shl_v32i(v32i a,v32i b){return a<<b;}
v32i shr_v32i(v32i a,v32i b){return a>>b;}
v32i neg_v32i(v32i a,v32i b){return -a;}
v32i not_v32i(v32i a,v32i b){return ~a;}
v32i eq_v32i(v32i a,v32i b){return a==b;}
v32i lt_v32i(v32i a,v32i b){return a<b;}
void loop_v32i(v32i*r,const v32i*a,const v32i*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v32i splat_v32i(int x){return (v32i){}+x;}
int lane2_v32i(v32i a){return a[2];}
v32i setlane_v32i(v32i a,int x){a[1]=x;return a;}
v32i ld_v32i(const int*p){v32i v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v32i(int*p,v32i v){__builtin_memcpy(p,&v,sizeof v);}
v32i cond_v32i(v32i a,v32i b,int c){return c?a+b:a-b;}
v32i acc_v32i(const v32i*p,int n){v32i s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef long v16l __attribute__((vector_size(16)));
v16l add_v16l(v16l a,v16l b){return a+b;}
v16l sub_v16l(v16l a,v16l b){return a-b;}
v16l mul_v16l(v16l a,v16l b){return a*b;}
v16l and_v16l(v16l a,v16l b){return a&b;}
v16l or_v16l(v16l a,v16l b){return a|b;}
v16l xor_v16l(v16l a,v16l b){return a^b;}
v16l shl_v16l(v16l a,v16l b){return a<<b;}
v16l shr_v16l(v16l a,v16l b){return a>>b;}
v16l neg_v16l(v16l a,v16l b){return -a;}
v16l not_v16l(v16l a,v16l b){return ~a;}
v16l eq_v16l(v16l a,v16l b){return a==b;}
v16l lt_v16l(v16l a,v16l b){return a<b;}
void loop_v16l(v16l*r,const v16l*a,const v16l*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v16l splat_v16l(long x){return (v16l){}+x;}
long lane2_v16l(v16l a){return a[2];}
v16l setlane_v16l(v16l a,long x){a[1]=x;return a;}
v16l ld_v16l(const long*p){v16l v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v16l(long*p,v16l v){__builtin_memcpy(p,&v,sizeof v);}
v16l cond_v16l(v16l a,v16l b,int c){return c?a+b:a-b;}
v16l acc_v16l(const v16l*p,int n){v16l s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef long v32l __attribute__((vector_size(32)));
v32l add_v32l(v32l a,v32l b){return a+b;}
v32l sub_v32l(v32l a,v32l b){return a-b;}
v32l mul_v32l(v32l a,v32l b){return a*b;}
v32l and_v32l(v32l a,v32l b){return a&b;}
v32l or_v32l(v32l a,v32l b){return a|b;}
v32l xor_v32l(v32l a,v32l b){return a^b;}
v32l shl_v32l(v32l a,v32l b){return a<<b;}
v32l shr_v32l(v32l a,v32l b){return a>>b;}
v32l neg_v32l(v32l a,v32l b){return -a;}
v32l not_v32l(v32l a,v32l b){return ~a;}
v32l eq_v32l(v32l a,v32l b){return a==b;}
v32l lt_v32l(v32l a,v32l b){return a<b;}
void loop_v32l(v32l*r,const v32l*a,const v32l*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v32l splat_v32l(long x){return (v32l){}+x;}
long lane2_v32l(v32l a){return a[2];}
v32l setlane_v32l(v32l a,long x){a[1]=x;return a;}
v32l ld_v32l(const long*p){v32l v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v32l(long*p,v32l v){__builtin_memcpy(p,&v,sizeof v);}
v32l cond_v32l(v32l a,v32l b,int c){return c?a+b:a-b;}
v32l acc_v32l(const v32l*p,int n){v32l s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef float v16f __attribute__((vector_size(16)));
v16f add_v16f(v16f a,v16f b){return a+b;}
v16f sub_v16f(v16f a,v16f b){return a-b;}
v16f mul_v16f(v16f a,v16f b){return a*b;}
v16f neg_v16f(v16f a,v16f b){return -a;}
v16i eq_v16f(v16f a,v16f b){return a==b;}
v16i lt_v16f(v16f a,v16f b){return a<b;}
v16f div_v16f(v16f a,v16f b){return a/b;}
void loop_v16f(v16f*r,const v16f*a,const v16f*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v16f splat_v16f(float x){return (v16f){}+x;}
float lane2_v16f(v16f a){return a[2];}
v16f setlane_v16f(v16f a,float x){a[1]=x;return a;}
v16f ld_v16f(const float*p){v16f v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v16f(float*p,v16f v){__builtin_memcpy(p,&v,sizeof v);}
v16f cond_v16f(v16f a,v16f b,int c){return c?a+b:a-b;}
v16f acc_v16f(const v16f*p,int n){v16f s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef float v32f __attribute__((vector_size(32)));
v32f add_v32f(v32f a,v32f b){return a+b;}
v32f sub_v32f(v32f a,v32f b){return a-b;}
v32f mul_v32f(v32f a,v32f b){return a*b;}
v32f neg_v32f(v32f a,v32f b){return -a;}
v32i eq_v32f(v32f a,v32f b){return a==b;}
v32i lt_v32f(v32f a,v32f b){return a<b;}
v32f div_v32f(v32f a,v32f b){return a/b;}
void loop_v32f(v32f*r,const v32f*a,const v32f*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v32f splat_v32f(float x){return (v32f){}+x;}
float lane2_v32f(v32f a){return a[2];}
v32f setlane_v32f(v32f a,float x){a[1]=x;return a;}
v32f ld_v32f(const float*p){v32f v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v32f(float*p,v32f v){__builtin_memcpy(p,&v,sizeof v);}
v32f cond_v32f(v32f a,v32f b,int c){return c?a+b:a-b;}
v32f acc_v32f(const v32f*p,int n){v32f s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef double v16d __attribute__((vector_size(16)));
v16d add_v16d(v16d a,v16d b){return a+b;}
v16d sub_v16d(v16d a,v16d b){return a-b;}
v16d mul_v16d(v16d a,v16d b){return a*b;}
v16d neg_v16d(v16d a,v16d b){return -a;}
v16l eq_v16d(v16d a,v16d b){return a==b;}
v16l lt_v16d(v16d a,v16d b){return a<b;}
v16d div_v16d(v16d a,v16d b){return a/b;}
void loop_v16d(v16d*r,const v16d*a,const v16d*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v16d splat_v16d(double x){return (v16d){}+x;}
double lane2_v16d(v16d a){return a[2];}
v16d setlane_v16d(v16d a,double x){a[1]=x;return a;}
v16d ld_v16d(const double*p){v16d v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v16d(double*p,v16d v){__builtin_memcpy(p,&v,sizeof v);}
v16d cond_v16d(v16d a,v16d b,int c){return c?a+b:a-b;}
v16d acc_v16d(const v16d*p,int n){v16d s={};for(int i=0;i<n;i++)s+=p[i];return s;}
typedef double v32d __attribute__((vector_size(32)));
v32d add_v32d(v32d a,v32d b){return a+b;}
v32d sub_v32d(v32d a,v32d b){return a-b;}
v32d mul_v32d(v32d a,v32d b){return a*b;}
v32d neg_v32d(v32d a,v32d b){return -a;}
v32l eq_v32d(v32d a,v32d b){return a==b;}
v32l lt_v32d(v32d a,v32d b){return a<b;}
v32d div_v32d(v32d a,v32d b){return a/b;}
void loop_v32d(v32d*r,const v32d*a,const v32d*b,int n){for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}
v32d splat_v32d(double x){return (v32d){}+x;}
double lane2_v32d(v32d a){return a[2];}
v32d setlane_v32d(v32d a,double x){a[1]=x;return a;}
v32d ld_v32d(const double*p){v32d v; __builtin_memcpy(&v,p,sizeof v); return v;}
void st_v32d(double*p,v32d v){__builtin_memcpy(p,&v,sizeof v);}
v32d cond_v32d(v32d a,v32d b,int c){return c?a+b:a-b;}
v32d acc_v32d(const v32d*p,int n){v32d s={};for(int i=0;i<n;i++)s+=p[i];return s;}
struct S8f { v32f v; }; struct S8f sadd(struct S8f a, struct S8f b){ a.v += b.v; return a; }
v32f call8(v32f a); v32f viacall(v32f a, v32f b){ return call8(a+b) * b; }

/* scalar + vector register interleave: s in %xmm0, a in %ymm1, b in %ymm2 */
static v32d c_dmix_helper(v32d t){ return t*t-t; }
static void c_touch(v32f*p){ (*p)[1]=(*p)[0]+1.0f; }
v32f mix_sab(float s, v32f a, v32f b){ return a*s + b; }
v16f mix_sab4(float s, v16f a, v16f b){ return a*s + b; }
/* scalar splat on the right / left */
v32i splat_r(v32i a){ return a + 3; }
v32f splat_l(v32f a){ return 2.0f * a; }
/* nine vector args: the ninth has no XMM register and arrives in memory */
v32f nine(v32f a,v32f b,v32f c,v32f d,v32f e,v32f f,v32f g,v32f h,v32f i){ return a+b+c+d+e+f+g+h+i; }
v16i nine4(v16i a,v16i b,v16i c,v16i d,v16i e,v16i f,v16i g,v16i h,v16i i){ return a+b+c+d+e+f+g+h+i; }
/* identity / parameter passthrough */
v32f ident(v32f a){ return a; }
v16d ident2(v16d a){ return a; }
/* whole-object assignment through pointers, including self-assignment */
void asg(v32f*r,const v32f*a){ *r=*a; }
void asg_self(v32f*r){ *r=*r; }
void asg4(v16i*r,const v16i*a){ *r=*a; }
/* overlapping union members: the load must precede the store */
union U { v32f v; v16f h[2]; };
void asg_union(union U*u){ u->v = u->v + u->v; }
/* ternary with lvalue arms, with side-effecting arms (one arm evaluated),
   nested, and with a call arm */
v32f tern_lv(v32f a, v32f b, int c){ return c ? a : b; }
int tern_count;
static v32f bump(v32f a){ tern_count++; return a + 1.0f; }
v32f tern_se(v32f a, v32f b, int c){ return c ? bump(a) : bump(b); }
v32f tern_nest(v32f a, v32f b, int c){ return c > 1 ? a + b : c ? a - b : a * b; }
v32f tern_assign(v32f a, v32f b, int c){ v32f r; r = c ? a : b; return r + 1.0f; }
/* sign of zero through negate / subtract */
v32f negz(v32f a){ return -a; }
v16d negz2(v16d a){ return -a; }
/* struct with a vector member passed/returned by value */
struct S4i { v16i v; int tag; };
struct S4i s4i_add(struct S4i a, v16i b){ a.v += b; a.tag++; return a; }
/* compound assign on a dereferenced pointer and on an array element */
void cas_ptr(v32f*p, v32f b){ *p *= b; }
void cas_arr(v16i*p, v16i b, int i){ p[i] ^= b; }
/* comparison mask feeding a blend by hand */
v32i sel_mask(v32f a, v32f b, v32i x, v32i y){ v32i m = a < b; return (m & x) | (~m & y); }
/* FP contraction shapes (vecfma): fused under FMA3 exactly where GCC fuses
   (every use of the product an absorbing add/sub in the same block), so the
   single-rounding results stay bit-identical across the differential */
v32f fma_f8(v32f a, v32f b, v32f c){ return a*b + c; }
v32f fma_f8r(v32f a, v32f b, v32f c){ return c + a*b; }
v32f fnma_f8(v32f a, v32f b, v32f c){ return c - a*b; }
v32f fms_f8(v32f a, v32f b, v32f c){ return a*b - c; }
v32d fma_d4(v32d a, v32d b, v32d c){ return a*b + c; }
v32d fnma_d4(v32d a, v32d b, v32d c){ return c - a*b; }
v32d fms_d4(v32d a, v32d b, v32d c){ return a*b - c; }
v16f fma_f4(v16f a, v16f b, v16f c){ return a*b + c; }
v16f fnma_f4(v16f a, v16f b, v16f c){ return c - a*b; }
v16f fms_f4(v16f a, v16f b, v16f c){ return a*b - c; }
v16d fma_d2(v16d a, v16d b, v16d c){ return a*b + c; }
v16d fnma_d2(v16d a, v16d b, v16d c){ return c - a*b; }
v32f fma_sq(v32f a, v32f c){ return a*a + c; }
v16f fma_sq4(v16f a, v16f c){ return a*a + c; }
v32f fma_self(v32f a, v32f b){ return a*b + a; }
v32f fma_chain(v32f a, v32f b, v32f c, v32f d){ return (a*b + c)*d + a; }
v32f fma_two(v32f a, v32f b, v32f c, v32f d){ v32f m = a*b; return (m + c) - (m - d); }
v32f fma_nouse(v32f a, v32f b, v32f c){ v32f m = a*b; return (m + c) * m; }
void fma_loop(v32f*r,const v32f*a,const v32f*b,const v32f*c,int n){ for(int i=0;i<n;i++) r[i]=a[i]*b[i]+c[i]; }
void fma_acc(v32d*r,const v32d*a,const v32d*b,int n){ v32d s={}; for(int i=0;i<n;i++) s = s + a[i]*b[i]; *r=s; }
/* FMA3 destructive handoff in EVERY operand position (regalloc
   `allocate_vector_registers_destructive`): the dying operand that becomes
   the destination may be the multiplicand (p0/p1), the addend (p2 → 231
   form), aliased (sq/al/al2: the 132 form reads both sources before the
   write), chained, used twice, or still live afterwards (live: no handoff
   allowed, the temporary must survive the FMA) — at all four widths */
#define HOFF(T,N) \
T N##_p0(T a,T b,T c){ T t=a+a; return t*b+c; } \
T N##_p1(T a,T b,T c){ T t=b+b; return a*t+c; } \
T N##_p2(T a,T b,T c){ T t=c+c; return a*b+t; } \
T N##_p1s(T a,T b,T c){ T t=b+b; return a*t-c; } \
T N##_p2s(T a,T b,T c){ T t=c+c; return t-a*b; } \
T N##_p2n(T a,T b,T c){ T t=c+c; return -(a*b)-t; } \
T N##_sq(T a,T b,T c){ T t=a+b; return t*t+c; } \
T N##_al(T a,T b,T c){ T t=a+b; return t*b+t; } \
T N##_al2(T a,T b,T c){ T t=a+b; return b*t+t; } \
T N##_ch(T a,T b,T c,T d){ T t=a*b+c; return t*d+t; } \
T N##_ch2(T a,T b,T c,T d){ T t=a*b+c; T u=t*d+a; return u*t+b; } \
T N##_two(T a,T b,T c,T d){ T t=a+b; return (t*c+d)+(t*d-c); } \
T N##_ret0(T a,T b,T c){ return c*b+a; } \
T N##_ret1(T a,T b,T c){ return b*c+a; } \
T N##_live(T a,T b,T c,T*o){ T t=a+a; *o=t*b+c; return t; }
HOFF(v16f,ho_f4) HOFF(v16d,ho_d2) HOFF(v32f,ho_f8) HOFF(v32d,ho_d4)
/* Control-flow / lifetime stress for the register-promotion pass (`vecssa`)
   and the param-preload prologue: phis across calls, loop-carried
   accumulators with and without calls in the body, nested ternaries,
   switch, goto loops, two live vectors across a call, long-double-free
   mixes, lane access after packed ops, conditional assignment, struct
   wrapping, address escape, volatile temporaries, compare chains,
   self-assignment, dead stores, per-lane rewrites and a zero-initialised
   temporary read through every lane. */
void sink(void);
v32f cf_phi_call(v32f a, v32f b, int c){ v32f r = c ? a+b : a-b; sink(); return r*r; }
v32f cf_loop_acc(const v32f*a, int n){ v32f s={}; for(int i=0;i<n;i++) s+=a[i]; return s; }
v32f cf_loop_acc_call(const v32f*a, int n){ v32f s={}; for(int i=0;i<n;i++){ s+=a[i]; if(i&1) sink(); } return s; }
v32f cf_nested_tern(v32f a, v32f b, int c){ return c>1 ? a*b : (c ? a+b : a-b); }
v32f cf_sw(v32f a, v32f b, int c){ v32f r=a; switch(c&3){ case 0: r=a+b; break; case 1: r=a-b; break; case 2: r=a*b; break; default: r=b; } return r+a; }
v32f cf_goto_loop(const v32f*a, int n){ v32f s=a[0]; int i=1; top: if(i<n){ s=s*a[i]+a[0]; i++; goto top; } return s; }
v32f cf_two_lives(v32f a, v32f b){ v32f x=a+b, y=a-b; sink(); return x*y; }
v32l cf_l_phi(v32l a, v32l b, int c){ v32l r = c ? a&b : a|b; return r^a; }
v32d cf_d_mix(v32d a, v32d b, double s){ v32d t=a*s; return c_dmix_helper(t)+b; }
float cf_lane_after(v32f a, v32f b){ v32f t=a*b+a; return t[0]+t[7]+t[3]; }
v32f cf_cond_assign(v32f a, v32f b, int c){ v32f r=a; if(c) r=b*a; return r+b; }
struct W8 { v32f v; int tag; };
v32f cf_wrap_phi(struct W8 w, v32f b, int c){ v32f r = c ? w.v : b; return r-w.v; }
v32f cf_addr_escape(v32f a, v32f b){ v32f t=a+b; c_touch(&t); return t*b; }
v32f cf_volatile_t(v32f a, v32f b){ volatile v32f t=a*b; v32f u=t; return u+a; }
v32i cf_cmp_chain(v32f a, v32f b, v32f c){ return (a<b)&(b<c); }
v32f cf_self_assign(v32f a){ a=a; a=a*a; a=a; return a; }
v32f cf_dead_store(v32f a, v32f b){ v32f t=a+b; t=a-b; t=t*b; return t; }
v32f cf_all_lanes(v32f a){ v32f t=a*a; for(int i=0;i<8;i++) t[i]=t[i]+(float)i; return t; }
v32f cf_uninit(v32f a){ v32f t={}; return t+a; }
/* Per-lane variable shifts (`vpsllv*`/`vpsrlv*`/`vpsravd`): unsigned lanes
   (logical right), non-constant scalar counts (splat), a count vector of
   the other signedness, 64-bit lanes (no `vpsravq` below AVX-512VL: the
   signed 64-bit right shift stays scalar), and 16-byte shapes. */
typedef unsigned v32u __attribute__((vector_size(32)));
typedef unsigned long v32ul __attribute__((vector_size(32)));
typedef unsigned v16u __attribute__((vector_size(16)));
v32u sh_vv_u(v32u a, v32u b){ return a >> b; }
v32ul sh_vv_ul(v32ul a, v32ul b){ return a >> b; }
v16u sh_vv_u4(v16u a, v16u b){ return a >> b; }
v32i sh_vs_shl(v32i a, int n){ return a << n; }
v32i sh_vs_sar(v32i a, int n){ return a >> n; }
v32u sh_vs_shr(v32u a, unsigned n){ return a >> n; }
v32l sh_vs_shl_l(v32l a, long n){ return a << n; }
v32ul sh_vs_shr_ul(v32ul a, int n){ return a >> n; }
v32i sh_mixsign(v32i a, v32u b){ return a << b; }
v32u sh_mixsign2(v32u a, v32i b){ return a >> b; }
v32i sh_chain(v32i a, v32i b, v32i c){ return (a << b) >> c; }
v32i sh_then_add(v32i a, v32i b){ return (a << b) + a; }
v16i sh4_vs_sar(v16i a, int n){ return a >> n; }
v16l sh4_vv_shl_l(v16l a, v16l b){ return a << b; }
v16i sh4_chain(v16i a, v16i b, v16i c){ return (a << b) + (a >> c); }

