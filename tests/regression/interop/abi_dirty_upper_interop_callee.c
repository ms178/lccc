/* Callee half of the dirty-upper-bits interop fixture; see
 * abi_dirty_upper_interop.h.  Every function is compiled against its true
 * narrow prototype; the caller passes (on AArch64) unspecified upper bits in
 * each narrow argument. */
#include "abi_dirty_upper_interop.h"

static volatile int du_sink;

/* Out of line on purpose: a value live across it sits in a callee-saved
 * register (or a slot) from the prologue on. */
__attribute__((noinline)) static void du_clobber(int v) { du_sink = v; }

/* x4: a leaf may keep the argument in its arrival register. */
int du_select(int a, int b, int c, int d, int k) { return k ? a + c : b + d; }
int du_branch(int a, int b, int c, int d, int k)
{
    if (k)
        return a - c;
    return b * d;
}
int du_select8(int a, int b, int c, int d, int e, int f, int g, int k)
{
    return k ? e + g : f - a + b - c + d;
}
int du_select_u8(int a, int b, unsigned char k) { return k ? a : b; }
int du_bool(_Bool f, int x) { return f ? x : -x; }
int du_bool_branch(int x, _Bool f)
{
    if (f)
        return x + 7;
    return x - 7;
}

/* Widening reads exactly the type's bits. */
long du_widen_s32(int a, int b, int c, int d, int k) { return k + 0L * (a + b + c + d); }
unsigned long du_widen_u32(unsigned a, unsigned b, unsigned c, unsigned d, unsigned k)
{
    return k + 0UL * (a + b + c + d);
}
long du_widen_s16(short s) { return s; }
unsigned long du_widen_u16(unsigned short s) { return s; }
long du_widen_s8(signed char c) { return c; }
unsigned long du_widen_u8(unsigned char c) { return c; }

/* A jump table indexed by the argument: the bounds check and the index
 * must agree on the width. */
int du_switch(int a, int b, int c, int d, int k)
{
    switch (k) {
    case 0: return 11;
    case 1: return 22 + a;
    case 2: return 33;
    case 3: return 44 + b;
    case 4: return 55;
    case 5: return 66 + c;
    case 6: return 77;
    case 7: return 88 + d;
    default: return -1;
    }
}

/* Addresses: a signed index sign-extends from bit 31, an unsigned one
 * zero-extends. */
int du_index(const int *p, int i) { return p[i]; }
int du_index_u(const int *p, unsigned i) { return p[i]; }
long du_sum(const int *p, int n)
{
    long s = 0;
    for (int i = 0; i < n; i++)
        s += p[i];
    return s;
}

/* Arithmetic and compares at the type's width. */
int du_div(int a, int b) { return a / b; }
unsigned du_divu(unsigned a, unsigned b) { return a / b; }
int du_lt(int a, int b) { return a < b; }
int du_ltu(unsigned a, unsigned b) { return a < b; }
int du_eq8(unsigned char a, unsigned char b) { return a == b; }
int du_lt8(signed char a, signed char b) { return a < b; }
int du_lt16(short a, short b) { return a < b; }
long du_mul_wide(int a, int b) { return (long)a * b; }
long du_add_wide(int a, unsigned b) { return a + (long)b; }
unsigned du_shift(unsigned x, int s) { return x << (s & 31); }

/* Live across a call. */
int du_live(int a, int b, int k)
{
    du_clobber(a);
    return k ? a : b;
}
int du_live_branch(int a, int b, int k)
{
    du_clobber(b);
    if (k)
        return a;
    return b;
}
long du_live_widen(int a, int k)
{
    du_clobber(a);
    return k;
}
unsigned long du_live_widen_u(int a, unsigned k)
{
    du_clobber(a);
    return k;
}

/* The ninth integer argument travels in an 8-byte stack slot whose upper
 * bytes are just as unspecified. */
int du_stack9(int a, int b, int c, int d, int e, int f, int g, int h, int k)
{
    return k ? a + h : b - g + c - f + d - e;
}
long du_stack9_widen(int a, int b, int c, int d, int e, int f, int g, int h, int k)
{
    return k + 0L * (a + b + c + d + e + f + g + h);
}
long du_stack9_short(int a, int b, int c, int d, int e, int f, int g, int h, short k)
{
    return k + 0L * (a + b + c + d + e + f + g + h);
}

/* Results the caller reads through narrower prototypes: on AArch64 the
 * upper bits come back exactly as they arrived. */
long du_ret_s32(long x) { return x; }
long du_ret_u16(long x) { return x; }
long du_ret_s8(long x) { return x; }

/* An int truncated from a 64-bit constant (folded only from -O1 on). The
 * RV64 psABI returns it as the sign-extended word, which the caller reads
 * as a whole register; on AArch64 the caller reads the W register. */
int du_trunc_wide(void)
{
    long c = 0x100000001L;
    return (int)c;
}
int du_trunc_wide_neg(void)
{
    long c = 0x1fffffffeL;
    return (int)c;
}
