
#include <stdio.h>
#include <string.h>
#include <setjmp.h>
#include <stdlib.h>

struct S3  { unsigned char b[3]; };
struct S12 { unsigned int a, b, c; };
struct S24 { unsigned long long x, y, z; };

static volatile int g_vol;                 /* anti-folding, forces reloads */
static unsigned long long g_sink;          /* consumes every computed value */

/* Noinline external barrier: clobbers caller-saved registers, so every value
   that is live across it must be spilled to a stack slot. */
__attribute__((noinline)) unsigned long long ext(unsigned long long x)
{
    g_vol++;
    return x ^ 0x9e3779b97f4a7c15ULL ^ (unsigned long long)g_vol;
}

__attribute__((noinline)) void barrier(void) { g_vol += 3; }

static unsigned long long mix(unsigned long long acc, unsigned long long v)
{
    acc ^= v + 0x9e3779b97f4a7c15ULL + (acc << 6u) + (acc >> 2u);
    return acc;
}

int main(void)
{
    g_sink = 0x123456789abcdefULL;
    {
        unsigned n = 16;
        unsigned long long a[n];
        unsigned int b[n];
        for (unsigned i = 0; i < n; i++) { a[i] = ((unsigned long long)(1ull * 0x9e3779b97f4a7c15ULL + 7ULL)) + i; b[i] = (unsigned)(((unsigned int)(2u * 2654435761u + 12345u))) + i; }
        barrier();
        for (unsigned i = 0; i < n; i++) g_sink = mix(g_sink, a[i] + (unsigned long long)b[i]);
    }
    g_sink = mix(g_sink, 0ull);
    unsigned long long __attribute__((vector_size(16))) v0_0;
    v0_0 = ((unsigned long long __attribute__((vector_size(16)))){ 3ull * 3ull, 3ull * 5ull + 1ull });
    struct S3 v1_1;
    v1_1 = ((struct S3){ { (unsigned char)(4u * 5u), (unsigned char)(5u * 9u), (unsigned char)(6u * 13u) } });
    unsigned char v2_2;
    v2_2 = ((unsigned char)(5u * 7u + 3u));
    unsigned char v3_3;
    v3_3 = ((unsigned char)(6u * 7u + 3u));
    long double v4_4;
    v4_4 = ((long double)(7) * 1.25L + 0.5L);
    struct S24 v5_5;
    v5_5 = ((struct S24){ 8ull * 5ull, 8ull * 7ull + 1ull, 8ull * 9ull + 2ull });
    struct S3 v6_6;
    v6_6 = ((struct S3){ { (unsigned char)(9u * 5u), (unsigned char)(10u * 9u), (unsigned char)(11u * 13u) } });
    unsigned char v7_7;
    v7_7 = ((unsigned char)(10u * 7u + 3u));
    unsigned char v8_8;
    v8_8 = ((unsigned char)(11u * 7u + 3u));
    struct S24 v9_9;
    v9_9 = ((struct S24){ 12ull * 5ull, 12ull * 7ull + 1ull, 12ull * 9ull + 2ull });
    unsigned short v10_10;
    v10_10 = ((unsigned short)(13u * 3121u + 17u));
    unsigned int v11_11;
    v11_11 = ((unsigned int)(14u * 2654435761u + 12345u));
    unsigned long long __attribute__((vector_size(16))) v12_12;
    v12_12 = ((unsigned long long __attribute__((vector_size(16)))){ 15ull * 3ull, 15ull * 5ull + 1ull });
    struct S12 v13_13;
    v13_13 = ((struct S12){ 16u * 3u, 16u * 11u + 1u, 16u * 13u + 2u });
    unsigned short v14_14;
    v14_14 = ((unsigned short)(17u * 3121u + 17u));
    struct S3 v15_15;
    v15_15 = ((struct S3){ { (unsigned char)(18u * 5u), (unsigned char)(19u * 9u), (unsigned char)(20u * 13u) } });
    struct S12 v16_16;
    v16_16 = ((struct S12){ 19u * 3u, 19u * 11u + 1u, 19u * 13u + 2u });
    unsigned short v17_17;
    v17_17 = ((unsigned short)(20u * 3121u + 17u));
    struct S3 v18_18;
    v18_18 = ((struct S3){ { (unsigned char)(21u * 5u), (unsigned char)(22u * 9u), (unsigned char)(23u * 13u) } });
    ext(g_sink); barrier(); ext(g_sink);
    g_sink = mix(g_sink, (unsigned long long)((unsigned long long __attribute__((vector_size(16))))v0_0)[0] ^ ((unsigned long long __attribute__((vector_size(16))))v0_0)[1]);
    g_sink = mix(g_sink, (unsigned long long)((unsigned)v1_1.b[0] * 3u + v1_1.b[1] * 5u + v1_1.b[2] * 7u));
    g_sink = mix(g_sink, (unsigned long long)v2_2);
    g_sink = mix(g_sink, (unsigned long long)v3_3);
    g_sink = mix(g_sink, (unsigned long long)((long long)v4_4));
    g_sink = mix(g_sink, v5_5.x * 3ull + v5_5.y * 5ull + v5_5.z * 7ull);
    g_sink = mix(g_sink, (unsigned long long)((unsigned)v6_6.b[0] * 3u + v6_6.b[1] * 5u + v6_6.b[2] * 7u));
    g_sink = mix(g_sink, (unsigned long long)v7_7);
    g_sink = mix(g_sink, (unsigned long long)v8_8);
    g_sink = mix(g_sink, v9_9.x * 3ull + v9_9.y * 5ull + v9_9.z * 7ull);
    g_sink = mix(g_sink, (unsigned long long)v10_10);
    g_sink = mix(g_sink, (unsigned long long)v11_11);
    g_sink = mix(g_sink, (unsigned long long)((unsigned long long __attribute__((vector_size(16))))v12_12)[0] ^ ((unsigned long long __attribute__((vector_size(16))))v12_12)[1]);
    g_sink = mix(g_sink, (unsigned long long)((unsigned long long)v13_13.a * 3ull + (unsigned long long)v13_13.b * 5ull + (unsigned long long)v13_13.c * 7ull));
    g_sink = mix(g_sink, (unsigned long long)v14_14);
    g_sink = mix(g_sink, (unsigned long long)((unsigned)v15_15.b[0] * 3u + v15_15.b[1] * 5u + v15_15.b[2] * 7u));
    g_sink = mix(g_sink, (unsigned long long)((unsigned long long)v16_16.a * 3ull + (unsigned long long)v16_16.b * 5ull + (unsigned long long)v16_16.c * 7ull));
    g_sink = mix(g_sink, (unsigned long long)v17_17);
    g_sink = mix(g_sink, (unsigned long long)((unsigned)v18_18.b[0] * 3u + v18_18.b[1] * 5u + v18_18.b[2] * 7u));
    g_sink = mix(g_sink, 1ull);
    unsigned char v0_19;
    v0_19 = ((unsigned char)(22u * 7u + 3u));
    unsigned long long __attribute__((vector_size(16))) v1_20;
    v1_20 = ((unsigned long long __attribute__((vector_size(16)))){ 23ull * 3ull, 23ull * 5ull + 1ull });
    long double v2_21;
    v2_21 = ((long double)(24) * 1.25L + 0.5L);
    if (((g_sink >> 7u) & 1u) ^ 0) {
        v0_19 = ((unsigned char)(25u * 7u + 3u));
        g_sink = mix(g_sink, (unsigned long long)v0_19);
        g_sink = mix(g_sink, (unsigned long long)((unsigned long long __attribute__((vector_size(16))))v1_20)[0] ^ ((unsigned long long __attribute__((vector_size(16))))v1_20)[1]);
        g_sink = mix(g_sink, (unsigned long long)((long long)v2_21));
        ext(g_sink);
    } else {
        v0_19 = ((unsigned char)(26u * 7u + 3u));
        g_sink = mix(g_sink, (unsigned long long)v0_19);
        v1_20 = ((unsigned long long __attribute__((vector_size(16)))){ 27ull * 3ull, 27ull * 5ull + 1ull });
        g_sink = mix(g_sink, (unsigned long long)((unsigned long long __attribute__((vector_size(16))))v1_20)[0] ^ ((unsigned long long __attribute__((vector_size(16))))v1_20)[1]);
        v2_21 = ((long double)(28) * 1.25L + 0.5L);
        g_sink = mix(g_sink, (unsigned long long)((long long)v2_21));
        barrier();
    }
    ext(g_sink);
    g_sink = mix(g_sink, (unsigned long long)v0_19);
    g_sink = mix(g_sink, (unsigned long long)((unsigned long long __attribute__((vector_size(16))))v1_20)[0] ^ ((unsigned long long __attribute__((vector_size(16))))v1_20)[1]);
    g_sink = mix(g_sink, (unsigned long long)((long long)v2_21));
    g_sink = mix(g_sink, 2ull);
    {
        volatile unsigned long long bt = ((unsigned long long)(29ull * 0x9e3779b97f4a7c15ULL + 7ULL));
        unsigned long long x = bt;
        g_sink = mix(g_sink, (unsigned long long)((x >> 0u) & 1u) * 1ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 1u) & 1u) * 2ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 2u) & 1u) * 3ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 3u) & 1u) * 4ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 4u) & 1u) * 5ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 5u) & 1u) * 6ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 6u) & 1u) * 7ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 7u) & 1u) * 8ull);
        barrier();
        if (((x >> 0u) & 1u) ^ 1u) { g_sink = mix(g_sink, 101ull); barrier(); }
        else { g_sink = mix(g_sink, 202ull); ext(g_sink); }
        if (((x >> 3u) & 1u) ^ 0u) { g_sink = mix(g_sink, 303ull); ext(g_sink); }
        else { g_sink = mix(g_sink, 404ull); barrier(); }
        g_sink = mix(g_sink, (unsigned long long)((x >> 0u) & 1u));
    }
    g_sink = mix(g_sink, 3ull);
    {
        static jmp_buf jb;
        volatile unsigned long long s1 = ((unsigned long long)(30ull * 0x9e3779b97f4a7c15ULL + 7ULL));
        volatile unsigned int s2 = ((unsigned int)(31u * 2654435761u + 12345u));
        if (setjmp(jb) == 0) {
            s1 = ((unsigned long long)(32ull * 0x9e3779b97f4a7c15ULL + 7ULL));
            s2 = ((unsigned int)(33u * 2654435761u + 12345u));
            barrier();
            g_sink = mix(g_sink, s1 + (unsigned long long)s2);
            longjmp(jb, 1);
        }
        g_sink = mix(g_sink, s1 * 3ull + (unsigned long long)s2 * 5ull);
    }
    g_sink = mix(g_sink, 4ull);
    {
        volatile unsigned long long bt = ((unsigned long long)(34ull * 0x9e3779b97f4a7c15ULL + 7ULL));
        unsigned long long x = bt;
        g_sink = mix(g_sink, (unsigned long long)((x >> 0u) & 1u) * 1ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 1u) & 1u) * 2ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 2u) & 1u) * 3ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 3u) & 1u) * 4ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 4u) & 1u) * 5ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 5u) & 1u) * 6ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 6u) & 1u) * 7ull);
        g_sink = mix(g_sink, (unsigned long long)((x >> 7u) & 1u) * 8ull);
        barrier();
        if (((x >> 0u) & 1u) ^ 1u) { g_sink = mix(g_sink, 101ull); barrier(); }
        else { g_sink = mix(g_sink, 202ull); ext(g_sink); }
        if (((x >> 3u) & 1u) ^ 0u) { g_sink = mix(g_sink, 303ull); ext(g_sink); }
        else { g_sink = mix(g_sink, 404ull); barrier(); }
        g_sink = mix(g_sink, (unsigned long long)((x >> 0u) & 1u));
    }
    g_sink = mix(g_sink, 5ull);
    printf("%llu\n", g_sink);
    return 0;
}
