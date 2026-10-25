/* SDM PCMPESTRI/M effective operand-size table: REX.W selects RAX/RDX
 * lengths, not wider vector operands, and the index is still ECX. */
typedef unsigned char bytes16 __attribute__((vector_size(16)));
typedef union { bytes16 v; unsigned char b[16]; unsigned long long q[2]; } data;
int main(void) {
    data a,b,narrow_mask,wide_mask;
    for(int i=0;i<16;i++) { a.b[i]='a'; b.b[i]='x'; }
    b.b[2]='a';
    for(int which=0;which<2;which++) {
        unsigned long long la=which?1:0x100000000ULL;
        unsigned long long lb=which?0x100000000ULL:3;
        int narrow,wide;
        __asm__("pcmpestri $0,%4,%3" : "=c"(narrow) : "a"(la), "d"(lb), "x"(a.v), "xm"(b.v) : "cc");
        __asm__("rex64 pcmpestri $0,%4,%3" : "=c"(wide) : "a"(la), "d"(lb), "x"(a.v), "xm"(b.v) : "cc");
        if(narrow!=16 || wide!=2) return 1+which;
        __asm__("pcmpestrm $0,%4,%3\n\tmovdqa %%xmm0,%0" : "=x"(narrow_mask.v) : "a"(la), "d"(lb), "x"(a.v), "xm"(b.v) : "xmm0", "cc");
        __asm__("rex64 pcmpestrm $0,%4,%3\n\tmovdqa %%xmm0,%0" : "=x"(wide_mask.v) : "a"(la), "d"(lb), "x"(a.v), "xm"(b.v) : "xmm0", "cc");
        if(narrow_mask.q[0]!=0 || narrow_mask.q[1]!=0 || wide_mask.q[0]!=4 || wide_mask.q[1]!=0) return 3+which;
    }
    return 0;
}
