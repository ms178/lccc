#include <stdint.h>
#include <stdio.h>
/* GNU may_alias vector views of aligned scalar arrays; unsigned lane arithmetic. */
typedef uint8_t v8_16 __attribute__((vector_size(16), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (A + B) - A;
}
static void reference_add_cancel_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (A - B) + B;
}
static void reference_sub_cancel_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = A & (A | B);
}
static void reference_and_absorb_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = A | (A & B);
}
static void reference_or_absorb_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (A & B) | (A & C);
}
static void reference_and_factor_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (A | B) & (A | C);
}
static void reference_or_factor_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = A & A;
}
static void reference_same_and_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = A | A;
}
static void reference_same_or_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = A - A;
}
static void reference_zero_sub_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = A ^ A;
}
static void reference_zero_xor_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u8_16(void *o, const void *a, const void *b, const void *c) {
    v8_16 A=*(const v8_16*)a, B=*(const v8_16*)b, C=*(const v8_16*)c; (void)B; (void)C;
    *(v8_16*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u8_16(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint8_t)(a[i]<8?a[i]:c[i]);
}
static int check_u8_16(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint8_t a[16],b[16],c[16],out[16],ref[16];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<16;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint8_t)(round<16?round:*seed); b[i]=(uint8_t)(*seed>>1); c[i]=(uint8_t)(~*seed);
        }
        packed_xor_cancel_u8_16(out,a,b,c); reference_xor_cancel_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u8_16(out,a,b,c); reference_add_cancel_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u8_16(out,a,b,c); reference_sub_cancel_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u8_16(out,a,b,c); reference_and_absorb_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u8_16(out,a,b,c); reference_or_absorb_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u8_16(out,a,b,c); reference_and_factor_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u8_16(out,a,b,c); reference_or_factor_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u8_16(out,a,b,c); reference_xor_pair_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u8_16(out,a,b,c); reference_same_and_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u8_16(out,a,b,c); reference_same_or_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u8_16(out,a,b,c); reference_zero_sub_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u8_16(out,a,b,c); reference_zero_xor_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u8_16(out,a,b,c); reference_nested_blend_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u8_16(out,a,b,c); reference_opposite_blend_add_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u8_16(out,a,b,c); reference_bounded_mask_u8_16(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u8_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#ifndef ONLY128
typedef uint8_t v8_32 __attribute__((vector_size(32), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (A + B) - A;
}
static void reference_add_cancel_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (A - B) + B;
}
static void reference_sub_cancel_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = A & (A | B);
}
static void reference_and_absorb_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = A | (A & B);
}
static void reference_or_absorb_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (A & B) | (A & C);
}
static void reference_and_factor_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (A | B) & (A | C);
}
static void reference_or_factor_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = A & A;
}
static void reference_same_and_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = A | A;
}
static void reference_same_or_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = A - A;
}
static void reference_zero_sub_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = A ^ A;
}
static void reference_zero_xor_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u8_32(void *o, const void *a, const void *b, const void *c) {
    v8_32 A=*(const v8_32*)a, B=*(const v8_32*)b, C=*(const v8_32*)c; (void)B; (void)C;
    *(v8_32*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u8_32(uint8_t *o,const uint8_t *a,const uint8_t *b,const uint8_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<32;++i) o[i]=(uint8_t)(a[i]<8?a[i]:c[i]);
}
static int check_u8_32(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint8_t a[32],b[32],c[32],out[32],ref[32];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<32;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint8_t)(round<16?round:*seed); b[i]=(uint8_t)(*seed>>1); c[i]=(uint8_t)(~*seed);
        }
        packed_xor_cancel_u8_32(out,a,b,c); reference_xor_cancel_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u8_32(out,a,b,c); reference_add_cancel_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u8_32(out,a,b,c); reference_sub_cancel_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u8_32(out,a,b,c); reference_and_absorb_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u8_32(out,a,b,c); reference_or_absorb_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u8_32(out,a,b,c); reference_and_factor_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u8_32(out,a,b,c); reference_or_factor_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u8_32(out,a,b,c); reference_xor_pair_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u8_32(out,a,b,c); reference_same_and_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u8_32(out,a,b,c); reference_same_or_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u8_32(out,a,b,c); reference_zero_sub_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u8_32(out,a,b,c); reference_zero_xor_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u8_32(out,a,b,c); reference_nested_blend_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u8_32(out,a,b,c); reference_opposite_blend_add_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u8_32(out,a,b,c); reference_bounded_mask_u8_32(ref,a,b,c);
        for(unsigned i=0;i<32;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u8_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#endif
typedef uint16_t v16_16 __attribute__((vector_size(16), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (A + B) - A;
}
static void reference_add_cancel_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (A - B) + B;
}
static void reference_sub_cancel_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = A & (A | B);
}
static void reference_and_absorb_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = A | (A & B);
}
static void reference_or_absorb_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (A & B) | (A & C);
}
static void reference_and_factor_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (A | B) & (A | C);
}
static void reference_or_factor_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = A & A;
}
static void reference_same_and_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = A | A;
}
static void reference_same_or_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = A - A;
}
static void reference_zero_sub_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = A ^ A;
}
static void reference_zero_xor_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u16_16(void *o, const void *a, const void *b, const void *c) {
    v16_16 A=*(const v16_16*)a, B=*(const v16_16*)b, C=*(const v16_16*)c; (void)B; (void)C;
    *(v16_16*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u16_16(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint16_t)(a[i]<8?a[i]:c[i]);
}
static int check_u16_16(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint16_t a[8],b[8],c[8],out[8],ref[8];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<8;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint16_t)(round<16?round:*seed); b[i]=(uint16_t)(*seed>>1); c[i]=(uint16_t)(~*seed);
        }
        packed_xor_cancel_u16_16(out,a,b,c); reference_xor_cancel_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u16_16(out,a,b,c); reference_add_cancel_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u16_16(out,a,b,c); reference_sub_cancel_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u16_16(out,a,b,c); reference_and_absorb_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u16_16(out,a,b,c); reference_or_absorb_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u16_16(out,a,b,c); reference_and_factor_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u16_16(out,a,b,c); reference_or_factor_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u16_16(out,a,b,c); reference_xor_pair_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u16_16(out,a,b,c); reference_same_and_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u16_16(out,a,b,c); reference_same_or_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u16_16(out,a,b,c); reference_zero_sub_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u16_16(out,a,b,c); reference_zero_xor_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u16_16(out,a,b,c); reference_nested_blend_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u16_16(out,a,b,c); reference_opposite_blend_add_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u16_16(out,a,b,c); reference_bounded_mask_u16_16(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u16_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#ifndef ONLY128
typedef uint16_t v16_32 __attribute__((vector_size(32), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (A + B) - A;
}
static void reference_add_cancel_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (A - B) + B;
}
static void reference_sub_cancel_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = A & (A | B);
}
static void reference_and_absorb_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = A | (A & B);
}
static void reference_or_absorb_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (A & B) | (A & C);
}
static void reference_and_factor_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (A | B) & (A | C);
}
static void reference_or_factor_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = A & A;
}
static void reference_same_and_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = A | A;
}
static void reference_same_or_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = A - A;
}
static void reference_zero_sub_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = A ^ A;
}
static void reference_zero_xor_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u16_32(void *o, const void *a, const void *b, const void *c) {
    v16_32 A=*(const v16_32*)a, B=*(const v16_32*)b, C=*(const v16_32*)c; (void)B; (void)C;
    *(v16_32*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u16_32(uint16_t *o,const uint16_t *a,const uint16_t *b,const uint16_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<16;++i) o[i]=(uint16_t)(a[i]<8?a[i]:c[i]);
}
static int check_u16_32(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint16_t a[16],b[16],c[16],out[16],ref[16];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<16;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint16_t)(round<16?round:*seed); b[i]=(uint16_t)(*seed>>1); c[i]=(uint16_t)(~*seed);
        }
        packed_xor_cancel_u16_32(out,a,b,c); reference_xor_cancel_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u16_32(out,a,b,c); reference_add_cancel_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u16_32(out,a,b,c); reference_sub_cancel_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u16_32(out,a,b,c); reference_and_absorb_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u16_32(out,a,b,c); reference_or_absorb_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u16_32(out,a,b,c); reference_and_factor_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u16_32(out,a,b,c); reference_or_factor_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u16_32(out,a,b,c); reference_xor_pair_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u16_32(out,a,b,c); reference_same_and_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u16_32(out,a,b,c); reference_same_or_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u16_32(out,a,b,c); reference_zero_sub_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u16_32(out,a,b,c); reference_zero_xor_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u16_32(out,a,b,c); reference_nested_blend_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u16_32(out,a,b,c); reference_opposite_blend_add_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u16_32(out,a,b,c); reference_bounded_mask_u16_32(ref,a,b,c);
        for(unsigned i=0;i<16;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u16_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#endif
typedef uint32_t v32_16 __attribute__((vector_size(16), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (A + B) - A;
}
static void reference_add_cancel_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (A - B) + B;
}
static void reference_sub_cancel_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = A & (A | B);
}
static void reference_and_absorb_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = A | (A & B);
}
static void reference_or_absorb_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (A & B) | (A & C);
}
static void reference_and_factor_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (A | B) & (A | C);
}
static void reference_or_factor_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = A & A;
}
static void reference_same_and_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = A | A;
}
static void reference_same_or_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = A - A;
}
static void reference_zero_sub_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = A ^ A;
}
static void reference_zero_xor_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u32_16(void *o, const void *a, const void *b, const void *c) {
    v32_16 A=*(const v32_16*)a, B=*(const v32_16*)b, C=*(const v32_16*)c; (void)B; (void)C;
    *(v32_16*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u32_16(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint32_t)(a[i]<8?a[i]:c[i]);
}
static int check_u32_16(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint32_t a[4],b[4],c[4],out[4],ref[4];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<4;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint32_t)(round<16?round:*seed); b[i]=(uint32_t)(*seed>>1); c[i]=(uint32_t)(~*seed);
        }
        packed_xor_cancel_u32_16(out,a,b,c); reference_xor_cancel_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u32_16(out,a,b,c); reference_add_cancel_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u32_16(out,a,b,c); reference_sub_cancel_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u32_16(out,a,b,c); reference_and_absorb_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u32_16(out,a,b,c); reference_or_absorb_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u32_16(out,a,b,c); reference_and_factor_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u32_16(out,a,b,c); reference_or_factor_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u32_16(out,a,b,c); reference_xor_pair_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u32_16(out,a,b,c); reference_same_and_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u32_16(out,a,b,c); reference_same_or_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u32_16(out,a,b,c); reference_zero_sub_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u32_16(out,a,b,c); reference_zero_xor_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u32_16(out,a,b,c); reference_nested_blend_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u32_16(out,a,b,c); reference_opposite_blend_add_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u32_16(out,a,b,c); reference_bounded_mask_u32_16(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u32_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#ifndef ONLY128
typedef uint32_t v32_32 __attribute__((vector_size(32), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (A + B) - A;
}
static void reference_add_cancel_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (A - B) + B;
}
static void reference_sub_cancel_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = A & (A | B);
}
static void reference_and_absorb_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = A | (A & B);
}
static void reference_or_absorb_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (A & B) | (A & C);
}
static void reference_and_factor_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (A | B) & (A | C);
}
static void reference_or_factor_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = A & A;
}
static void reference_same_and_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = A | A;
}
static void reference_same_or_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = A - A;
}
static void reference_zero_sub_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = A ^ A;
}
static void reference_zero_xor_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u32_32(void *o, const void *a, const void *b, const void *c) {
    v32_32 A=*(const v32_32*)a, B=*(const v32_32*)b, C=*(const v32_32*)c; (void)B; (void)C;
    *(v32_32*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u32_32(uint32_t *o,const uint32_t *a,const uint32_t *b,const uint32_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<8;++i) o[i]=(uint32_t)(a[i]<8?a[i]:c[i]);
}
static int check_u32_32(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint32_t a[8],b[8],c[8],out[8],ref[8];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<8;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint32_t)(round<16?round:*seed); b[i]=(uint32_t)(*seed>>1); c[i]=(uint32_t)(~*seed);
        }
        packed_xor_cancel_u32_32(out,a,b,c); reference_xor_cancel_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u32_32(out,a,b,c); reference_add_cancel_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u32_32(out,a,b,c); reference_sub_cancel_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u32_32(out,a,b,c); reference_and_absorb_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u32_32(out,a,b,c); reference_or_absorb_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u32_32(out,a,b,c); reference_and_factor_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u32_32(out,a,b,c); reference_or_factor_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u32_32(out,a,b,c); reference_xor_pair_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u32_32(out,a,b,c); reference_same_and_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u32_32(out,a,b,c); reference_same_or_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u32_32(out,a,b,c); reference_zero_sub_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u32_32(out,a,b,c); reference_zero_xor_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u32_32(out,a,b,c); reference_nested_blend_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u32_32(out,a,b,c); reference_opposite_blend_add_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u32_32(out,a,b,c); reference_bounded_mask_u32_32(ref,a,b,c);
        for(unsigned i=0;i<8;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u32_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#endif
typedef uint64_t v64_16 __attribute__((vector_size(16), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (A + B) - A;
}
static void reference_add_cancel_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (A - B) + B;
}
static void reference_sub_cancel_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = A & (A | B);
}
static void reference_and_absorb_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = A | (A & B);
}
static void reference_or_absorb_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (A & B) | (A & C);
}
static void reference_and_factor_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (A | B) & (A | C);
}
static void reference_or_factor_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = A & A;
}
static void reference_same_and_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = A | A;
}
static void reference_same_or_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = A - A;
}
static void reference_zero_sub_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = A ^ A;
}
static void reference_zero_xor_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u64_16(void *o, const void *a, const void *b, const void *c) {
    v64_16 A=*(const v64_16*)a, B=*(const v64_16*)b, C=*(const v64_16*)c; (void)B; (void)C;
    *(v64_16*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u64_16(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<2;++i) o[i]=(uint64_t)(a[i]<8?a[i]:c[i]);
}
static int check_u64_16(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint64_t a[2],b[2],c[2],out[2],ref[2];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<2;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint64_t)(round<16?round:*seed); b[i]=(uint64_t)(*seed>>1); c[i]=(uint64_t)(~*seed);
        }
        packed_xor_cancel_u64_16(out,a,b,c); reference_xor_cancel_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u64_16(out,a,b,c); reference_add_cancel_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u64_16(out,a,b,c); reference_sub_cancel_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u64_16(out,a,b,c); reference_and_absorb_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u64_16(out,a,b,c); reference_or_absorb_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u64_16(out,a,b,c); reference_and_factor_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u64_16(out,a,b,c); reference_or_factor_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u64_16(out,a,b,c); reference_xor_pair_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u64_16(out,a,b,c); reference_same_and_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u64_16(out,a,b,c); reference_same_or_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u64_16(out,a,b,c); reference_zero_sub_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u64_16(out,a,b,c); reference_zero_xor_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u64_16(out,a,b,c); reference_nested_blend_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u64_16(out,a,b,c); reference_opposite_blend_add_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u64_16(out,a,b,c); reference_bounded_mask_u64_16(ref,a,b,c);
        for(unsigned i=0;i<2;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u64_16"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#ifndef ONLY128
typedef uint64_t v64_32 __attribute__((vector_size(32), may_alias));
__attribute__((noinline)) void packed_xor_cancel_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (A ^ B) ^ A;
}
static void reference_xor_cancel_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(b[i]);
}
__attribute__((noinline)) void packed_add_cancel_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (A + B) - A;
}
static void reference_add_cancel_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(b[i]);
}
__attribute__((noinline)) void packed_sub_cancel_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (A - B) + B;
}
static void reference_sub_cancel_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_and_absorb_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = A & (A | B);
}
static void reference_and_absorb_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_or_absorb_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = A | (A & B);
}
static void reference_or_absorb_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_and_factor_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (A & B) | (A & C);
}
static void reference_and_factor_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)((a[i]&b[i])|(a[i]&c[i]));
}
__attribute__((noinline)) void packed_or_factor_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (A | B) & (A | C);
}
static void reference_or_factor_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)((a[i]|b[i])&(a[i]|c[i]));
}
__attribute__((noinline)) void packed_xor_pair_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (A ^ B) ^ (A ^ C);
}
static void reference_xor_pair_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(b[i]^c[i]);
}
__attribute__((noinline)) void packed_same_and_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = A & A;
}
static void reference_same_and_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_same_or_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = A | A;
}
static void reference_same_or_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]);
}
__attribute__((noinline)) void packed_zero_sub_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = A - A;
}
static void reference_zero_sub_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(0);
}
__attribute__((noinline)) void packed_zero_xor_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = A ^ A;
}
static void reference_zero_xor_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(0);
}
__attribute__((noinline)) void packed_nested_blend_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = ((((A>B)&A)|(~(A>B)&B))&(A>B))|(~(A>B)&C);
}
static void reference_nested_blend_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]>b[i]?a[i]:c[i]);
}
__attribute__((noinline)) void packed_opposite_blend_add_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = (((A>B)&A)|(~(A>B)&B)) + (((A>B)&B)|(~(A>B)&A));
}
static void reference_opposite_blend_add_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]+b[i]);
}
__attribute__((noinline)) void packed_bounded_mask_u64_32(void *o, const void *a, const void *b, const void *c) {
    v64_32 A=*(const v64_32*)a, B=*(const v64_32*)b, C=*(const v64_32*)c; (void)B; (void)C;
    *(v64_32*)o = ((A<8)&(A&7))|(~(A<8)&C);
}
static void reference_bounded_mask_u64_32(uint64_t *o,const uint64_t *a,const uint64_t *b,const uint64_t *c) {
    (void)a; (void)b; (void)c; for(unsigned i=0;i<4;++i) o[i]=(uint64_t)(a[i]<8?a[i]:c[i]);
}
static int check_u64_32(uint64_t *seed,uint64_t *digest) {
    _Alignas(32) uint64_t a[4],b[4],c[4],out[4],ref[4];
    for(unsigned round=0;round<256;++round) {
        for(unsigned i=0;i<4;++i) {
            *seed^=*seed<<13; *seed^=*seed>>7; *seed^=*seed<<17;
            a[i]=(uint64_t)(round<16?round:*seed); b[i]=(uint64_t)(*seed>>1); c[i]=(uint64_t)(~*seed);
        }
        packed_xor_cancel_u64_32(out,a,b,c); reference_xor_cancel_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_cancel_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_add_cancel_u64_32(out,a,b,c); reference_add_cancel_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: add_cancel_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_sub_cancel_u64_32(out,a,b,c); reference_sub_cancel_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: sub_cancel_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_absorb_u64_32(out,a,b,c); reference_and_absorb_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_absorb_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_absorb_u64_32(out,a,b,c); reference_or_absorb_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_absorb_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_and_factor_u64_32(out,a,b,c); reference_and_factor_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: and_factor_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_or_factor_u64_32(out,a,b,c); reference_or_factor_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: or_factor_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_xor_pair_u64_32(out,a,b,c); reference_xor_pair_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: xor_pair_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_and_u64_32(out,a,b,c); reference_same_and_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_and_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_same_or_u64_32(out,a,b,c); reference_same_or_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: same_or_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_sub_u64_32(out,a,b,c); reference_zero_sub_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_sub_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_zero_xor_u64_32(out,a,b,c); reference_zero_xor_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: zero_xor_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_nested_blend_u64_32(out,a,b,c); reference_nested_blend_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: nested_blend_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_opposite_blend_add_u64_32(out,a,b,c); reference_opposite_blend_add_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: opposite_blend_add_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
        packed_bounded_mask_u64_32(out,a,b,c); reference_bounded_mask_u64_32(ref,a,b,c);
        for(unsigned i=0;i<4;++i) { if(out[i]!=ref[i]) {puts("mismatch: bounded_mask_u64_32"); return 1;} *digest=(*digest*UINT64_C(0x100000001b3))^out[i]; }
    }
    return 0;
}
#endif
int main(void) {
    uint64_t seed=UINT64_C(0x243f6a8885a308d3),digest=0;
    if(check_u8_16(&seed,&digest)) return 1;
#ifndef ONLY128
    if(check_u8_32(&seed,&digest)) return 1;
#endif
    if(check_u16_16(&seed,&digest)) return 1;
#ifndef ONLY128
    if(check_u16_32(&seed,&digest)) return 1;
#endif
    if(check_u32_16(&seed,&digest)) return 1;
#ifndef ONLY128
    if(check_u32_32(&seed,&digest)) return 1;
#endif
    if(check_u64_16(&seed,&digest)) return 1;
#ifndef ONLY128
    if(check_u64_32(&seed,&digest)) return 1;
#endif
    printf("%llu\n",(unsigned long long)digest);
    return 0;
}
