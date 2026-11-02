/* The exit-add can be removed only with a no-wrap proof, including the
 * first failing test. Fixed-width types keep this reference portable. */
typedef unsigned char U8;
typedef unsigned int U32;
typedef unsigned long long U64;
typedef long long I64;

__attribute__((noinline)) U64 hot_guard_64(const U8 *p) {
    U64 sum=0;
    for (U64 i=0; i+8<=256; i+=8) sum += (U64)p[i] ^ i;
    return sum;
}
__attribute__((noinline)) U32 hot_guard_32(const U8 *p) {
    U32 sum=0;
    for (U32 i=0; i+8<256; i+=8) sum += (U32)p[i] ^ i;
    return sum;
}
__attribute__((noinline)) U64 negative_start(const U8 *p) {
    U64 sum=0;
    for (I64 i=-8; i+8<=64; i+=8) sum += (U64)p[i+8] ^ (U64)i;
    return sum;
}
__attribute__((noinline)) U64 wrapping32(void) {
    U32 count=0, acc=0;
    for (U32 i=0xfffffff0U; i+8<=0xfffffffdU; i+=8) {
        acc ^= i;
        if (++count==20) break;
    }
    return ((U64)count<<32)|acc;
}
__attribute__((noinline)) U64 wrapping64(void) {
    U32 count=0;
    U64 acc=0;
    for (U64 i=0xfffffffffffffff0ULL; i+8<=0xfffffffffffffffdULL; i+=8) {
        acc ^= i;
        if (++count==20) break;
    }
    return acc ^ ((U64)count<<32);
}
__attribute__((noinline)) U64 empty_guard(const U8 *p) {
    U64 sum=17;
    for (U64 i=16; i+8<=8; i+=8) sum+=p[i];
    return sum;
}

int main(void) {
    U8 p[256];
    U32 state=19;
    for (unsigned seed=0; seed<129; ++seed) {
        for (unsigned k=0; k<256; ++k) { state=state*1664525U+1013904223U; p[k]=(U8)(state>>24); }
        U64 r64=0, neg=0;
        U32 r32=0;
        const volatile U8 *v=p;
        for (unsigned k=0; k<32; ++k) r64+=(U64)v[k*8]^(U64)(k*8);
        for (unsigned k=0; k<31; ++k) r32+=(U32)v[k*8]^(U32)(k*8);
        for (unsigned k=0; k<9; ++k) neg+=(U64)v[k*8]^(U64)((I64)k*8-8);
        if (hot_guard_64(p)!=r64) return 1;
        if (hot_guard_32(p)!=r32) return 2;
        if (negative_start(p)!=neg) return 3;
        if (empty_guard(p)!=17) return 4;
    }
    /* Independent modular sequence: no biased comparison in this reference. */
    U32 r32=0;
    U64 r64=0;
    for (unsigned k=0; k<20; ++k) {
        r32 ^= (U32)(0xfffffff0ULL + (U64)k*8);
        r64 ^= 0xfffffffffffffff0ULL + (U64)k*8;
    }
    if (wrapping32()!=((20ULL<<32)|r32)) return 5;
    if (wrapping64()!=(r64^(20ULL<<32))) return 6;
    return 0;
}
