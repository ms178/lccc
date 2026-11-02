/* Explicit lane extraction, simultaneous narrow values and a live wide canary.
 * The XMM clobber forces spill/reload even when FP homes are enabled. */
#include <xmmintrin.h>
#include <stdint.h>
#include <stdio.h>
__attribute__((noinline)) static double lanes(float *out, const float *in, const volatile double *wide) {
    __m128 v=_mm_loadu_ps(in);
    float a=_mm_cvtss_f32(v);
    float b=_mm_cvtss_f32(_mm_shuffle_ps(v,v,0x55));
    float c=_mm_cvtss_f32(_mm_shuffle_ps(v,v,0xaa));
    float d=_mm_cvtss_f32(_mm_shuffle_ps(v,v,0xff));
    double guard=*wide;
    __asm__ volatile("" ::: "xmm0","xmm1","xmm2","xmm3","xmm4","xmm5","xmm6","xmm7",
                            "xmm8","xmm9","xmm10","xmm11","xmm12","xmm13","xmm14","xmm15","memory");
    out[0]=a; out[1]=b; out[2]=c; out[3]=d;
    return guard;
}
int main(void) {
    union { uint32_t u; float f; } in[4], got;
    union { uint64_t u; double d; } wide, result;
    float values[4], out[4]; uint32_t state=123456789;
    for(unsigned k=0;k<257;++k) {
        for(unsigned j=0;j<4;++j) {
            state=state*1664525u+1013904223u;
            in[j].u=(state&0x807fffffu)|(((state%250)+1)<<23);
            if(k==0) in[j].u=(j&1)?0x80000000u:0;
            values[j]=in[j].f;
        }
        wide.u=UINT64_C(0x400921fb54442d18) ^ ((uint64_t)k<<7);
        result.d=lanes(out,values,&wide.d);
        if(result.u!=wide.u) return 1;
        for(unsigned j=0;j<4;++j) { got.f=out[j]; if(got.u!=in[j].u) return 2; }
    }
    puts("fp_extract_slot_boundary: OK");
    return 0;
}
