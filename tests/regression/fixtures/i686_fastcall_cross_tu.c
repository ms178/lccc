/* Standalone, non-kernel ABI differential: compile caller and callee with
 * different compilers so incorrect matching conventions cannot cancel. */
#define FC __attribute__((fastcall,noinline))
struct R { int a,b,c,d,e; };
int FC scalar(int,int,int,int,int,int);
long long FC wide(int,int,int,int);
double FC mixed(int,double,int,long long);
struct R FC record(int,int,int,int);
int FC aggregate(int,struct R,int,int);
#ifdef ABI_CALLEE
int FC scalar(int a,int b,int c,int d,int e,int f)
{ return a+2*b+3*c+4*d+5*e+6*f; }
long long FC wide(int a,int b,int c,int d)
{ return ((long long)(unsigned)(a+b)<<32)|(unsigned)(c+d); }
double FC mixed(int a,double b,int c,long long d) { return a+b+c+d; }
struct R FC record(int a,int b,int c,int d)
{ return (struct R){a,b,c,d,a+b+c+d}; }
int FC aggregate(int a,struct R r,int b,int c)
{ return a+r.a+2*r.b+3*r.c+4*r.d+5*r.e+6*b+7*c; }
#else
/* noinline is a function property, not needed on pointer typedefs. */
typedef int __attribute__((fastcall)) FS(int,int,int,int,int,int);
typedef long long __attribute__((fastcall)) FW(int,int,int,int);
typedef double __attribute__((fastcall)) FM(int,double,int,long long);
typedef struct R __attribute__((fastcall)) FR(int,int,int,int);
typedef int __attribute__((fastcall)) FA(int,struct R,int,int);
FS *volatile ps=scalar;
FW *volatile pw=wide;
FM *volatile pm=mixed;
FR *volatile pr=record;
FA *volatile pa=aggregate;
int main(void) {
    for(int i=0;i<31;++i) {
        /* Clang's fastcall record-result pointer lives on the stack;
         * GNU's lives in ECX. Only the GCC pairs exercise that GNU ABI. */
#ifdef CLANG_FASTCALL_VARIANT
        struct R r;
        r.a=1; r.b=2; r.c=3; r.d=4; r.e=10;
#else
        struct R r=pr(1,2,3,4);
#endif
        if(ps(1,2,3,4,5,6)!=91 || pw(1,2,3,4)!=(((long long)3<<32)|7) ||
           pm(1,2.5,3,4)!=10.5 || r.a!=1 || r.b!=2 || r.c!=3 || r.d!=4 || r.e!=10 ||
           pa(1,r,6,7)!=166) return 1;
    }
    return 0;
}
#endif
