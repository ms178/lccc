#include <stdio.h>
#include <unwind.h>
#define MS __attribute__((ms_abi))
#define NI __attribute__((noinline))
typedef long MS (*msfn)(long,double,long,float,long,long);
extern MS long lccc_ms(long,double,long,float,long,long);
extern MS double lccc_fp(double,long,float,double,long);
extern long lccc_driver(long);
extern long shadow_call(void);
extern int preservation_probe(void);
MS NI long gcc_ms(long a,double b,long c,float d,long e,long f) {
 return a+(long)b*2+c*3+(long)d*4+e*5+f*6;
}
NI void clobber_sysv(void) {
    __asm__ volatile(
        "mov $123,%%rdi; mov $456,%%rsi;"
        "pxor %%xmm6,%%xmm6; pxor %%xmm7,%%xmm7;"
        "pxor %%xmm8,%%xmm8; pxor %%xmm9,%%xmm9;"
        "pxor %%xmm10,%%xmm10; pxor %%xmm11,%%xmm11;"
        "pxor %%xmm12,%%xmm12; pxor %%xmm13,%%xmm13;"
        "pxor %%xmm14,%%xmm14; pxor %%xmm15,%%xmm15"
        ::: "rdi", "rsi", "xmm6", "xmm7", "xmm8", "xmm9", "xmm10",
            "xmm11", "xmm12", "xmm13", "xmm14", "xmm15");
}
MS long ms_single(long x) { return 13*x; }
long sys_single(long x) { return 7*x; }
MS long ms_first(long x) { return x+1; }
MS long ms_second(long x) { return x+2; }
MS long ms_string(const char *s) { return s[0]+s[1]+s[2]; }
MS long ms_variadic(int tag, ...) {
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap,tag);
    double a=__builtin_va_arg(ap,double);
    long b=__builtin_va_arg(ap,long);
    double c=__builtin_va_arg(ap,double);
    double d=__builtin_va_arg(ap,double);
    long e=__builtin_va_arg(ap,long);
    __builtin_ms_va_end(ap);
    return tag+(long)(a*10)+b+(long)(c*100)+(long)(d*10)+e;
}
MS long ms_function_argument(msfn f) { return f(1,2.0,3,4.0f,5,6); }
extern MS long inherited(long), large_frame(long);
extern long function_argument_call(void);
extern long mixed_calls(long), sibling_calls(long), variadic_call(void), string_call(void);
struct unwind_state { _Unwind_Ptr expected; int seen, frames; };
static _Unwind_Reason_Code unwind_frame(struct _Unwind_Context *context, void *arg) {
    struct unwind_state *state=arg;
    state->frames++;
    state->seen |= _Unwind_GetIP(context)==state->expected;
    return _URC_NO_REASON;
}
NI int sysv_unwind_helper(void) {
    struct unwind_state state={(_Unwind_Ptr)__builtin_return_address(0),0,0};
    _Unwind_Backtrace(unwind_frame,&state);
    return state.seen && state.frames>=4;
}
extern MS long unwind_probe(long);
int main(void) {
 long (*volatile sysv)(long)=lccc_driver;
 long (MS *volatile ms)(long,double,long,float,long,long)=lccc_ms;
 for(long i=-3;i<=9;i++) {
   long a=ms(i,2.0,3,4.0f,5,6), b=sysv(i);
   if(mixed_calls(i)!=20*i || sibling_calls(i)!=2*i+3 || inherited(i)!=i+17
      || large_frame(i)!=2*i+1) {
       puts("mixed convention/sibling declaration mismatch"); return 5;
   }
   if(a!=i+90 || b!=5*(i+90)) { printf("mismatch %ld %ld %ld\n",i,a,b); return 1; }
 }
 if(lccc_fp(1.5,2,3.25f,4.5,5)!=16.25) { puts("FP mismatch"); return 2; }
 if(variadic_call()!=552 || string_call()!=294 || function_argument_call()!=91) {
     puts("variadic FP duplication/array decay mismatch"); return 6;
 }
 if(shadow_call()!=165) { puts("shadow/overflow/alignment mismatch"); return 3; }
 if(unwind_probe(9)!=10) { puts("MS large-frame unwinder mismatch"); return 7; }
 if(preservation_probe()) { puts("nonvolatile corruption"); return 4; }
 puts("PASS Microsoft x64 typed direct/indirect, GP/FP, overflow, shadow space, nonvolatile preservation");
}
