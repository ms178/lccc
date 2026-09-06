/* i686 over-aligned struct stack arguments — the GCC -m32 cdecl ABI.
 *
 * GCC's i386 cdecl NEVER aligns stack arguments beyond the 4-byte slot
 * granularity (PARM_BOUNDARY): a 3-int prefix places an aligned(32)/24-byte
 * struct at arg offset 12 (== 4 mod 8), sizeof rounds 24 -> 32, and the
 * callee reads x via %ebp at that same 4-granular offset. The `aligned`
 * attribute rounds the TYPE SIZE only — no argument-area padding, no
 * dynamic realignment, for fixed and variadic calls alike (GCC 14.2 -m32
 * oracle, empirically pinned in CallAbiConfig::stack_arg_align_cap).
 *
 * Two lccc bugs violated that contract and are pinned here:
 *  1. classify_params_full aligned struct stack params to min(align, 8):
 *     with a 3-int prefix the callee read x 4 bytes past where the caller
 *     (and GCC) store it. Also shifted va_start's overflow offset for any
 *     variadic function with over-aligned struct named params.
 *  2. The callee's param-copy wrote the RAW alloca slot while every access
 *     recomputed align_up(slot, 32) — a guaranteed desync (the slot offset
 *     is never 32-aligned in absolute terms), corrupting the bytes under
 *     &x even when the incoming offset was right.
 *
 * Probes: named-arg byte identity, &x self-consistency, address-taken
 * writes through the aligned pointer, variadic reading with an over-aligned
 * struct named-param prefix (va_start offset), and an aligned(16) struct at
 * the same 4-mod-8 natural offset (GCC does not 16-align it either).
 * libc-free; exits through the i386 ABI. */

typedef struct {
    __attribute__((aligned(32))) unsigned char p[24];
} A32;
typedef struct {
    __attribute__((aligned(16))) unsigned char p[12];
} A16;

/* Keep this ABI probe defined under -O2/-Os: assigning an unsigned-word
 * lvalue through a char-array member then reading it as char relies on an
 * incompatible effective type, and GCC legitimately exploits that aliasing
 * assumption.  The test wants the actual object representation instead. */
static void put32le(unsigned char *p, unsigned v)
{
    p[0] = (unsigned char)v;
    p[1] = (unsigned char)(v >> 8);
    p[2] = (unsigned char)(v >> 16);
    p[3] = (unsigned char)(v >> 24);
}

static unsigned get32le(const unsigned char *p)
{
    return (unsigned)p[0]
        | ((unsigned)p[1] << 8)
        | ((unsigned)p[2] << 16)
        | ((unsigned)p[3] << 24);
}

static A32 mk32(unsigned v)
{
    A32 a;
    for (int i = 0; i < 6; i++)
        put32le(&a.p[i * 4], v + (unsigned)i);
    return a;
}

/* Named over-aligned struct at natural offset 12 (4 mod 8): the caller
 * stores it 4-granular; the callee must read the identical bytes. */
int callee32(int a, int b, int c, A32 x)
{
    (void)a; (void)b; (void)c;
    for (int i = 0; i < 6; i++)
        if (get32le(&x.p[i * 4]) != 0x11000u + (unsigned)i)
            return 0;
    /* &x must alias the storage the bytes were copied into: write through
     * the address, read through the value. */
    put32le(&x.p[8], 0xdeadbeefu);
    if (x.p[8] != 0xefu || x.p[11] != 0xdeu)
        return 0;
    return 1;
}

int callee16(int a, int b, int c, A16 x)
{
    (void)a; (void)b; (void)c;
    for (int i = 0; i < 3; i++)
        if (get32le(&x.p[i * 4]) != 0x22000u + (unsigned)i)
            return 0;
    return 1;
}

/* Variadic with over-aligned struct NAMED params: the named-arg area is
 * 4+4+4+32 (A32) = 44 bytes, so the first vararg sits at overflow offset
 * 44 — the va_start offset classify_params_full derives. An 8-cap layout
 * would compute 48 and skip the first vararg dword. */
int callee_v(int a, int b, int c, A32 x, ...)
{
    (void)a; (void)b; (void)c;
    for (int i = 0; i < 6; i++)
        if (get32le(&x.p[i * 4]) != 3000u + (unsigned)i)
            return 0;
    __builtin_va_list ap;
    __builtin_va_start(ap, x);
    unsigned v1 = __builtin_va_arg(ap, unsigned);
    unsigned v2 = __builtin_va_arg(ap, unsigned);
    __builtin_va_end(ap);
    return v1 == 0x5a5a5a5au && v2 == 0xc3c3c3c3u;
}

static int probe_named(void)
{
    /* Local aligned storage: reads through the value must see the writes
     * (the alloca-address/copy desync check). */
    A32 x = mk32(1000);
    if (get32le(&x.p[0]) != 1000u || get32le(&x.p[20]) != 1005u)
        return 0;
    /* Param side: pass a fresh copy and verify every dword. */
    A32 y;
    for (int i = 0; i < 6; i++)
        put32le(&y.p[i * 4], 0x11000u + (unsigned)i);
    if (!callee32(7, 8, 9, y))
        return 0;
    A16 s;
    for (int i = 0; i < 3; i++)
        put32le(&s.p[i * 4], 0x22000u + (unsigned)i);
    if (!callee16(7, 8, 9, s))
        return 0;
    return 1;
}

static int probe_varargs(void)
{
    A32 x = mk32(3000);
    if (!callee_v(1, 2, 3, x, 0x5a5a5a5au, 0xc3c3c3c3u))
        return 0;
    return 1;
}

__attribute__((noreturn))
void _start(void)
{
    int status = (probe_named() && probe_varargs()) ? 0 : 1;
    __asm__ volatile ("int $0x80" : : "a"(1), "b"(status) : "memory");
    __builtin_unreachable();
}
