/* No libc headers: the compile-only ILP32 control needs no multilib headers.
 * All pointers and accesses remain within storage, including negative offsets.
 */
typedef unsigned long long u64;
typedef signed long long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef signed short i16;
typedef unsigned char u8;
typedef signed char i8;
extern int printf(const char *, ...);
_Static_assert(sizeof(u64) == 8 && sizeof(u32) == 4 && sizeof(u16) == 2, "integer widths");
static u64 storage[1024];

__attribute__((noinline)) static u64 unsigned_index(const u64 *p, u64 i) {
    return p[(i64)(i + 1)];
}
__attribute__((noinline)) static u64 signed_index(const u64 *p, i64 i) {
    return p[(i64)((u64)i - 1)];
}
__attribute__((noinline)) static u64 narrow_word(const u64 *p, u32 i) {
    return p[(i64)(int)i + 32];
}
__attribute__((noinline)) static u64 narrow_half(const u64 *p, u16 i) {
    return p[(i64)(i16)i + 1];
}
__attribute__((noinline)) static u64 narrow_signed_byte(const u64 *p, u8 i) {
    return p[(i64)(i8)i + 1];
}
__attribute__((noinline)) static u64 narrow_unsigned_byte(const u64 *p, u8 i) {
    return p[(u64)i + 1];
}

int main(void) {
    u64 digest = 0;
    const u64 *p = storage + 512;
    for (unsigned i = 0; i < 1024; ++i)
        storage[i] = ((u64)i * 0x9e3779b97f4a7c15ULL) ^ (i >> 3);
    for (int i = -128; i <= 128; ++i) {
        digest = digest * 33 + unsigned_index(p, (u64)(i64)i);
        digest ^= signed_index(p, (i64)i);
    }
    for (unsigned i = 0; i < 32; ++i) {
        digest += narrow_word(p, 0xfffffff0U + i);
        digest ^= narrow_half(p, (u16)(0xfff0U + i));
    }
    for (unsigned i = 0; i < 256; ++i) {
        digest += narrow_signed_byte(p, (u8)i);
        digest ^= narrow_unsigned_byte(p, (u8)i);
    }
    printf("pointer-ring-indexing %016llx\n", digest);
    return 0;
}
