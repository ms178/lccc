typedef unsigned char v16u8 __attribute__((vector_size(16)));
typedef unsigned short v8u16 __attribute__((vector_size(16)));
typedef unsigned long long v2u64 __attribute__((vector_size(16)));
typedef unsigned v4u __attribute__((vector_size(16)));
typedef double v2d __attribute__((vector_size(16)));
extern v16u8 id8(v16u8);
extern v8u16 id16(v8u16);
extern v2u64 id64(v2u64);
extern v2d iddouble(v2d);
extern v4u id32(v4u);
extern v4u relay32(v4u,v4u (*)(v4u));
extern v4u pick5(v4u,v4u,v4u,v4u,v4u);
#ifndef LCCC_OMIT_VARIADIC_ABI_PROBE
extern v4u pick_variadic(v4u,...);
#endif
static int same(const void *a,const void *b) {
    const unsigned char *x=a,*y=b;
    for(unsigned k=0;k<16;++k) if(x[k]!=y[k])return 0;
    return 1;
}
int main(void) {
    unsigned state=17;
    for(unsigned n=0;n<2048;++n) {
        v4u a,b,c,d,e;
        for(unsigned k=0;k<4;++k) {
            state=state*1664525u+1013904223u;a[k]=state;
            state=state*1664525u+1013904223u;b[k]=state;
            state=state*1664525u+1013904223u;c[k]=state;
            state=state*1664525u+1013904223u;d[k]=state;
            state=state*1664525u+1013904223u;e[k]=state;
        }
        v16u8 r8=id8((v16u8)a); if(!same(&r8,&a))return 1;
        v8u16 r16=id16((v8u16)a); if(!same(&r16,&a))return 2;
        v2u64 r64=id64((v2u64)a); if(!same(&r64,&a))return 3;
        v2d rd=iddouble((v2d)a); if(!same(&rd,&a))return 4;
        v4u r32=id32(a); if(!same(&r32,&a))return 5;
        v4u ri=relay32(a,id32); if(!same(&ri,&a))return 6;
        v4u expected={b[0],e[1],b[2],e[3]};
        v4u r5=pick5(a,b,c,d,e); if(!same(&r5,&expected))return 7;
#ifndef LCCC_OMIT_VARIADIC_ABI_PROBE
        expected=(v4u){a[0],b[1],a[2],b[3]};
        v4u rv=pick_variadic(a,b); if(!same(&rv,&expected))return 8;
#endif
    }
    return 0;
}
