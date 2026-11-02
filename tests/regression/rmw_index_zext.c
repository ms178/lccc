/* Counter increments indexed by a 32-bit unsigned value: `C[idx]++`.
 *
 * The x86-64 emitter re-zero-extends a 32-bit index before each memory
 * access, so the load and the store of `C[idx]++` are separated by a
 * redundant `movl %r, %r`, and a 32-bit increment is spelled
 * `leal K(%r64), %dst` (address-width base).  The memory read-modify-write
 * fold must still produce one `addl $K, (base,idx,4)` / `subl $K, (...)` for
 * every index shape below, as GCC 16.2 and Clang 23.1 do.
 *
 * check_rmw_index_zext_codegen.sh pins the emitted code; the runtime half of
 * this fixture (compared against GCC by run_regression.py and by the gate)
 * pins the semantics of every shape, including:
 *   - inc_and_return: the new value is used, so the fold must NOT apply;
 *   - inc_u16 / the narrow counters: the operation is exact only modulo the
 *     counter width, and an out-of-range increment (`+= 300` on a byte) must
 *     wrap exactly as the C conversion does;
 *   - inc_alias: two counters that may be the same object.
 */
#include <stdio.h>
#include <string.h>

typedef unsigned U32;
typedef unsigned char BYTE;

__attribute__((noinline)) void inc_shr24(U32 *C, U32 c) { C[c >> 24]++; }
__attribute__((noinline)) void inc_byte(U32 *C, U32 c) { C[(BYTE)c]++; }
__attribute__((noinline)) void inc_shr8(U32 *C, U32 c) { C[(c >> 8) & 0xffff]++; }
__attribute__((noinline)) void inc_mask(U32 *C, U32 c) { C[c & 0xff]++; }
__attribute__((noinline)) void inc_param(U32 *C, U32 c) { C[c]++; }
__attribute__((noinline)) void add_param(U32 *C, U32 c) { C[c] += 7; }
__attribute__((noinline)) void sub_param(U32 *C, U32 c) { C[c] -= 3; }

/* Negative controls (the fold must not delete a live value). */
__attribute__((noinline)) U32 inc_and_return(U32 *C, U32 c) { return ++C[c]; }
__attribute__((noinline)) void inc_u16(unsigned short *C, U32 c) { C[c >> 24]++; }
__attribute__((noinline)) void inc_alias(U32 *C, U32 *D, U32 c) { C[c]++; D[c]++; }

/* Narrow counters with increments wider than the counter. */
__attribute__((noinline)) void add_byte_300(BYTE *C, U32 c) { C[c >> 24] += 300; }
__attribute__((noinline)) void sub_byte_200(BYTE *C, U32 c) { C[c >> 24] -= 200; }
__attribute__((noinline)) void add_word_70000(unsigned short *C, U32 c) { C[c >> 24] += 70000; }
__attribute__((noinline)) void sub_schar_3(signed char *C, U32 c) { C[c >> 24] -= 3; }

/* zstd HIST_count_parallel_wksp shape: four tables, four bytes per word. */
__attribute__((noinline)) void hist4(U32 *C, const BYTE *ip, const BYTE *iend) {
    U32 *C1 = C, *C2 = C + 256, *C3 = C + 512, *C4 = C + 768;
    while (ip + 4 <= iend) {
        U32 c;
        memcpy(&c, ip, 4);
        ip += 4;
        C1[(BYTE)c]++;
        C2[(BYTE)(c >> 8)]++;
        C3[(BYTE)(c >> 16)]++;
        C4[c >> 24]++;
    }
}

static U32 table[1 << 16];
static unsigned short small[256];
static unsigned short words[256];
static BYTE bytes[256];
static signed char schars[256];
static BYTE buf[4096];

static U32 fold(const U32 *t, unsigned n) {
    U32 h = 2166136261u;
    for (unsigned i = 0; i < n; i++) h = (h ^ t[i]) * 16777619u;
    return h;
}

int main(void) {
    U32 x = 0x9e3779b9u;
    for (unsigned i = 0; i < sizeof buf; i++) {
        x = x * 1664525u + 1013904223u;
        buf[i] = (BYTE)(x >> 24);
    }
    U32 seed = 12345u, live = 0;
    for (unsigned i = 0; i < 20000; i++) {
        seed = seed * 1103515245u + 12345u;
        inc_shr24(table, seed);
        inc_byte(table, seed);
        inc_shr8(table, seed);
        inc_mask(table, seed);
        inc_param(table, seed & 0xffff);
        add_param(table, (seed >> 3) & 0xffff);
        sub_param(table, (seed >> 5) & 0xffff);
        live += inc_and_return(table, (seed >> 7) & 0xffff);
        inc_u16(small, seed);
        inc_alias(table, table, (seed >> 11) & 0xffff);
        add_byte_300(bytes, seed);
        sub_byte_200(bytes, seed * 7u);
        add_word_70000(words, seed);
        sub_schar_3(schars, seed);
    }
    printf("table %08x live %08x\n", fold(table, 1 << 16), live);
    U32 h = 0;
    for (unsigned i = 0; i < 256; i++) h = h * 31u + small[i];
    printf("small %08x\n", h);
    h = 0;
    for (unsigned i = 0; i < 256; i++) h = (h * 31u + bytes[i]) * 31u + words[i];
    for (unsigned i = 0; i < 256; i++) h = h * 31u + (U32)(int)schars[i];
    printf("narrow %08x\n", h);
    memset(table, 0, 1024 * sizeof table[0]);
    hist4(table, buf, buf + sizeof buf);
    printf("hist4 %08x\n", fold(table, 1024));
    return 0;
}
