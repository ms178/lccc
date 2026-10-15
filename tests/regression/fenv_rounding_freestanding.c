/* Dynamic rounding without libc: run on every supported backend with
   -frounding-math. The four indices mean nearest/up/down/toward-zero. */
__attribute__((noinline)) static void set_mode(unsigned mode) {
#if defined(__aarch64__)
    unsigned long control;
    __asm__ volatile("mrs %0, fpcr" : "=r"(control));
    control=(control & ~(3UL<<22)) | ((unsigned long)mode<<22);
    __asm__ volatile("msr fpcr, %0" :: "r"(control) : "memory");
#elif defined(__riscv)
    const unsigned encodings[]={0,3,2,1};
    unsigned long control=encodings[mode];
    __asm__ volatile("csrw frm, %0" :: "r"(control) : "memory");
#else
    const unsigned encodings[]={0,2,1,3};
    unsigned control;
    unsigned short x87;
    __asm__ volatile("stmxcsr %0" : "=m"(control));
    control=(control & ~(3U<<13)) | (encodings[mode]<<13);
    __asm__ volatile("ldmxcsr %0" :: "m"(control) : "memory");
    __asm__ volatile("fnstcw %0" : "=m"(x87));
    x87=(unsigned short)((x87 & ~(3U<<10)) | (encodings[mode]<<10));
    __asm__ volatile("fldcw %0" :: "m"(x87) : "memory");
#endif
}
__attribute__((noinline)) double positive(void) { return 1.0+0x1p-53; }
__attribute__((noinline)) double negative(void) { return -1.0-0x1p-53; }
__attribute__((noinline)) float converted(void) { return (float)16777217; }
int main(void) {
    for(unsigned i=0;i<4;i++) {
        set_mode(i);
        union { double d; unsigned long long u; } p,n;
        union { float f; unsigned u; } c;
        p.d=positive(); n.d=negative(); c.f=converted();
        if(p.u!=(i==1?0x3ff0000000000001ULL:0x3ff0000000000000ULL))return 20+(int)i;
        if(n.u!=(i==2?0xbff0000000000001ULL:0xbff0000000000000ULL))return 30+(int)i;
        if(c.u!=(i==1?0x4b800001U:0x4b800000U))return 40+(int)i;
    }
    set_mode(0);
    return 0;
}
