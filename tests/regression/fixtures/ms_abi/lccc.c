#define MS __attribute__((ms_abi))
#define NI __attribute__((noinline))
extern MS long gcc_ms(long,double,long,float,long,long);
extern void clobber_sysv(void);
extern MS long shadow_probe(long,long,long,long,long);
typedef long MS (*msfn)(long,double,long,float,long,long);
typedef long (*svfn)(long,double,long,float,long,long);
_Static_assert(!__builtin_types_compatible_p(msfn, svfn), "ABI belongs to function type");
struct ops { long (MS *f)(long,double,long,float,long,long); };
MS NI long lccc_ms(long a,double b,long c,float d,long e,long f) {
    long result=a+(long)b*2+c*3+(long)d*4+e*5+f*6;
    clobber_sysv();
    return result;
}
MS NI void lccc_ms_clobber(void) { clobber_sysv(); }
NI long apply(msfn f, long x) { return f(x,2.0,3,4.0f,5,6); }
NI long lccc_driver(long x) {
    msfn volatile p=gcc_ms;
    struct ops v={gcc_ms};
    msfn array[2]={gcc_ms,gcc_ms};
    return gcc_ms(x,2.0,3,4.0f,5,6)+p(x,2.0,3,4.0f,5,6)
       +v.f(x,2.0,3,4.0f,5,6)+array[x&1](x,2.0,3,4.0f,5,6)+apply(gcc_ms,x);
}
NI long shadow_call(void) { return shadow_probe(11,22,33,44,55); }
MS NI double lccc_fp(double a,long b,float c,double d,long e) { return a+b+c+d+e; }

// A SysV-only typed-call optimization must never rewrite these MS sites.
extern MS long ms_single(long);
extern long sys_single(long);
NI long mixed_calls(long x) { return ms_single(x) + sys_single(x); }

// Specifier attributes apply to both comma-separated declarations.
MS long ms_first(long), ms_second(long);
_Static_assert(__builtin_types_compatible_p(__typeof__(ms_first), __typeof__(ms_second)),
               "specifier ABI applies to every declarator");
NI long sibling_calls(long x) { return ms_first(x) + ms_second(x); }

extern MS long ms_variadic(int, ...);
NI long variadic_call(void) { return ms_variadic(7, 2.5, 11L, 4.25, 6.5, 19L); }
extern MS long ms_string(const char *);
NI long string_call(void) { return ms_string("abc"); }

// A declaration without an ABI inherits the established ABI, not SysV.
MS long inherited(long);
long inherited(long);
long inherited(long x) { return x+17; }

extern MS long ms_function_argument(msfn);
NI long function_argument_call(void) { return ms_function_argument(lccc_ms); }
MS NI long large_frame(long x) {
    volatile char data[32768];
    data[0]=(char)x;
    data[32767]=(char)(x+1);
    clobber_sysv();
    return data[0]+data[32767];
}

extern int sysv_unwind_helper(void);
MS NI long unwind_probe(long x) {
    volatile char frame[32768];
    frame[0]=(char)x;
    int success=sysv_unwind_helper();
    return frame[0]+success;
}
