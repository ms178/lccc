/* NON-KERNEL i386 ABI contract. Keep volatile cells: calls must stay indirect.
 * Assembly callees independently check the entry alignment and ECX/EDX/stack
 * assignment, rather than letting matching compiler bugs cancel each other. */
#define FC __attribute__((fastcall))
#define NI __attribute__((noinline))
typedef int FC F0(void);
typedef int FC F1(int);
typedef int FC F4(int, int, int, int);
typedef int Cdecl4(int, int, int, int);
_Static_assert(!__builtin_types_compatible_p(F4 *, Cdecl4 *),
               "fastcall is part of the function type");
extern int FC probe0(void);
extern int FC probe1(int);
extern int FC probe4(int, int, int, int);
__asm__(".text\n"
        ".globl probe0\nprobe0:\n"
        "leal 4(%esp),%eax\n andl $15,%eax\n jnz 9f\n movl $17,%eax\n ret\n"
        ".globl probe1\nprobe1:\n"
        "leal 4(%esp),%eax\n andl $15,%eax\n jnz 9f\n movl %ecx,%eax\n ret\n"
        ".globl probe4\nprobe4:\n"
        "leal 4(%esp),%eax\n andl $15,%eax\n jnz 8f\n"
        "leal (%ecx,%edx,2),%eax\n movl 4(%esp),%ecx\n"
        "leal (%ecx,%ecx,2),%ecx\n addl %ecx,%eax\n"
        "movl 8(%esp),%ecx\n leal (%eax,%ecx,4),%eax\n ret $8\n"
        "8: movl $-999,%eax\n ret $8\n9: movl $-999,%eax\n ret\n");
F0 *volatile p0 = probe0;
F1 *volatile p1 = probe1;
F4 *volatile p4 = probe4;
static NI F4 *choose(int n) { return n ? p4 : probe4; }
static NI int invoke(int (__attribute__((fastcall)) *p)(int,int,int,int))
{ return p(1,2,3,4); }

/* Inherit the convention from a previous declaration, including a typedef. */
int FC inherited(int, int, int, int);
NI int inherited(int a, int b, int c, int d) { return a+2*b+3*c+4*d; }
static F4 *volatile pi = inherited;
struct Hooks { F4 *call; };
static volatile struct Hooks hooks = {probe4};
static F4 *volatile table[2] = {probe4, inherited};
static NI long long FC wide(int a, int b, int c, int d)
{ return ((long long)(unsigned)(a+b)<<32) | (unsigned)(c+d); }
typedef long long FC FW(int,int,int,int);
static FW *volatile pw = wide;
struct R { int a,b,c,d,e; };
static NI struct R FC record(int a,int b,int c,int d)
{ return (struct R){a,b,c,d,a+b+c+d}; }
typedef struct R FC FR(int,int,int,int);
static FR *volatile pr = record;
static NI double FC mixed(int a, double b, int c, long long d)
{ return a+b+c+d; }
typedef double FC FM(int,double,int,long long);
static FM *volatile pm = mixed;
/* GNU fastcall variadics use the cdecl stack convention and caller cleanup. */
static NI int FC variable(int a, ...)
{
    __builtin_va_list ap;
    __builtin_va_start(ap,a);
    int b=__builtin_va_arg(ap,int), c=__builtin_va_arg(ap,int);
    __builtin_va_end(ap);
    return a+2*b+3*c;
}
typedef int FC FV(int,...);
static FV *volatile pv = variable;
static NI struct R FC variable_record(int a, ...)
{
    __builtin_va_list ap;
    __builtin_va_start(ap,a);
    int b=__builtin_va_arg(ap,int);
    __builtin_va_end(ap);
    return (struct R){a,b,3,4,a+b+7};
}
typedef struct R FC FVR(int,...);
static FVR *volatile pvr = variable_record;
int main(void)
{
    for (int i=0; i<257; ++i) {
        F4 *local = p4;
        struct R r = pr(1,2,3,4);
        struct R vr = pvr(1,2);
        if (p0()!=17 || p1(13)!=13 || p4(1,2,3,4)!=30 ||
            local(1,2,3,4)!=30 || choose(i)(1,2,3,4)!=30 ||
            (i ? p4 : local)(1,2,3,4)!=30 || pi(1,2,3,4)!=30 ||
            hooks.call(1,2,3,4)!=30 || table[i&1](1,2,3,4)!=30 || invoke(p4)!=30 ||
            pw(1,2,3,4)!=(((long long)3<<32)|7) ||
            r.a!=1 || r.b!=2 || r.c!=3 || r.d!=4 || r.e!=10 ||
            pm(1,2.5,3,4)!=10.5 || pv(1,2,3)!=14 ||
            vr.a!=1 || vr.b!=2 || vr.c!=3 || vr.d!=4 || vr.e!=10)
            return 1;
    }
    return 0;
}
