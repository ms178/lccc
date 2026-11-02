/* Early byte-load folding must retain exact access footprints and effects.
 * No libc dependency unless GUARD_PAGES is explicitly requested by the host
 * harness. The volatile-byte reference cannot use the widening transform. */
typedef unsigned char U8;
typedef unsigned int U32;
typedef unsigned long long U64;
struct Frame { U8 head[8], body[8]; U32 tail; };

static U64 body_le64(const struct Frame *f) {
    const U8 *p = f->body;
    return (U64)p[0] | (U64)p[1]<<8 | (U64)p[2]<<16 | (U64)p[3]<<24 |
           (U64)p[4]<<32 | (U64)p[5]<<40 | (U64)p[6]<<48 | (U64)p[7]<<56;
}
static U64 load64(const U8 *p) {
    return (U64)p[0] | (U64)p[1]<<8 | (U64)p[2]<<16 | (U64)p[3]<<24 |
           (U64)p[4]<<32 | (U64)p[5]<<40 | (U64)p[6]<<48 | (U64)p[7]<<56;
}
static U32 load32(const U8 *p) {
    return (U32)p[0] | (U32)p[1]<<8 | (U32)p[2]<<16 | (U32)p[3]<<24;
}
static U64 negative64(const U8 *p) {
    return (U64)p[-8] | (U64)p[-7]<<8 | (U64)p[-6]<<16 | (U64)p[-5]<<24 |
           (U64)p[-4]<<32 | (U64)p[-3]<<40 | (U64)p[-2]<<48 | (U64)p[-1]<<56;
}
static U64 repeated_negative(const U8 *p) {
    return (U64)p[-1] | (U64)p[-1]<<8 | (U64)p[-1]<<16 | (U64)p[-1]<<24 |
           (U64)p[-1]<<32 | (U64)p[-1]<<40 | (U64)p[-1]<<48 | (U64)p[-1]<<56;
}
static U64 big64(const U8 *p) {
    return (U64)p[7] | (U64)p[6]<<8 | (U64)p[5]<<16 | (U64)p[4]<<24 |
           (U64)p[3]<<32 | (U64)p[2]<<40 | (U64)p[1]<<48 | (U64)p[0]<<56;
}
__attribute__((noinline)) static U64 ref(const volatile U8 *p, unsigned n, int big) {
    U64 r = 0;
    for (unsigned i=0; i<n; ++i) r |= (U64)p[i] << (8*(big ? n-1-i : i));
    return r;
}
static U32 store_between(U8 *p) {
    U32 low = p[0];
    p[0] ^= 0x80;
    return low | (U32)p[1]<<8 | (U32)p[2]<<16 | (U32)p[3]<<24;
}

#ifdef GUARD_PAGES
#include <sys/mman.h>
#include <unistd.h>
__attribute__((noinline)) static int guard_pages(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page <= 0) return 20;
    U8 *m = mmap(0, (unsigned long)page*3, PROT_NONE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (m == MAP_FAILED) return 21;
    if (mprotect(m+page, (unsigned long)page, PROT_READ|PROT_WRITE)) return 22;
    U8 *p = m+2*page-8;
    for (unsigned i=0; i<8; ++i) p[i]=(U8)(i*37+7);
    int bad = load64(p) != ref(p,8,0) || negative64(p+8) != ref(p,8,0) ||
              load32(p+4) != ref(p+4,4,0) || big64(p) != ref(p,8,1);
    if (munmap(m, (unsigned long)page*3)) return 23;
    return bad ? 24 : 0;
}
#endif

int main(void) {
    U8 bytes[40];
    struct Frame frame;
    U32 state = 0x917bf037U;
    for (unsigned seed=0; seed<257; ++seed) {
        for (unsigned i=0; i<40; ++i) {
            state=state*1664525U+1013904223U; bytes[i]=(U8)(state>>24);
        }
        for (unsigned off=0; off<24; ++off) {
            const U8 *p=bytes+off;
            if (load64(p)!=ref(p,8,0)) return 1;
            if (load32(p)!=ref(p,4,0)) return 2;
            if (negative64(p+8)!=ref(p,8,0)) return 3;
            if (big64(p)!=ref(p,8,1)) return 4;
            if (repeated_negative(p+1)!=(U64)*p*0x0101010101010101ULL) return 5;
        }
        for (unsigned i=0; i<8; ++i) { frame.head[i]=(U8)i; frame.body[i]=bytes[i]; }
        frame.tail=0xaabbccddU;
        if (body_le64(&frame)!=ref(frame.body,8,0)) return 6;
        U32 before=(U32)ref(bytes,4,0);
        if (store_between(bytes)!=before) return 7;
        if (bytes[0]!=(U8)((before&255U)^128U)) return 8;
    }
#ifdef GUARD_PAGES
    return guard_pages();
#else
    return 0;
#endif
}
