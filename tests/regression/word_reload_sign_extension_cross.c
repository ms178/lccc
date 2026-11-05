/* A 32-bit store reloaded as a signed word is a sign extension, whatever the
 * stored register's upper half held.
 *
 * RISC-V -O0: `(void *)(int32_t)u` staged the zero-extended unsigned value
 * through a frame slot (`sw t0` / `lwu t0` / `sext.w t0, t0`).  The peephole
 * folded `lwu; sext.w` into `lw` -- correct by itself -- and the adjacent
 * store/load rule then treated `sw t0; lw t0` as an identity and deleted the
 * load: 0x8000_0000 came back as 0x0000_0000_8000_0000 instead of
 * 0xffff_ffff_8000_0000 (PR824 boundary witness integer_pointer_width.c).
 * The same rule in the global store forwarder rewrote `lw rY` to `mv rY, rX`.
 * A forwarded word reload is now `sext.w rY, rX`, as AArch64 rewrites
 * `str wN; ldrsw xM` to `sxtw xM, wN`.
 *
 * Every shape below moves a 32-bit pattern between the signed and unsigned
 * views through memory (casts, unions, pointer punning, volatile staging)
 * and checks the 64-bit result against constants.  The cross harness runs it
 * on AArch64 and RV64 at every level; the native corpus on x86-64.
 */
typedef unsigned u32;
typedef unsigned long long u64;
typedef long long i64;

__attribute__((noinline)) i64 cast_signed(u32 u) { return (i64)(int)u; }
__attribute__((noinline)) u64 cast_unsigned(int i) { return (u64)(u32)i; }
__attribute__((noinline)) void *cast_pointer(u32 u) { return (void *)(__INTPTR_TYPE__)(int)u; }
__attribute__((noinline)) i64 union_signed(u32 u)
{
    union { u32 u; int i; } v;
    v.u = u;
    return v.i;
}
__attribute__((noinline)) u64 union_unsigned(int i)
{
    union { int i; u32 u; } v;
    v.i = i;
    return v.u;
}
__attribute__((noinline)) i64 punned_signed(u32 u)
{
    u32 slot = u;
    return *(int *)&slot;
}
__attribute__((noinline)) i64 volatile_signed(u32 u)
{
    volatile u32 slot = u;
    int i = (int)slot;
    return i;
}
/* Both views alive at once: the unsigned one feeds a 64-bit add, so its
 * register stays zero-extended while the signed view is reloaded. */
__attribute__((noinline)) i64 both_views(u32 u)
{
    u32 slot = u;
    u64 wide = (u64)slot + 1;
    int i = *(int *)&slot;
    return (i64)wide + i;
}

int main(void)
{
    static volatile u32 in[] = {0x80000000u, 0xffffffffu, 0x7fffffffu, 0u, 1u, 0xfffffffeu};
    static const i64 sext[] = {-2147483647LL - 1, -1, 2147483647, 0, 1, -2};
    static const u64 zext[] = {0x80000000ull, 0xffffffffull, 0x7fffffffull, 0, 1, 0xfffffffeull};
    int bad = 0;
    for (unsigned k = 0; k < sizeof in / sizeof *in; k++) {
        u32 u = in[k];
        if (cast_signed(u) != sext[k])
            bad |= 1;
        if (cast_unsigned((int)u) != zext[k])
            bad |= 2;
        if ((i64)(__INTPTR_TYPE__)cast_pointer(u) != (i64)(__INTPTR_TYPE__)sext[k])
            bad |= 4;
        if (union_signed(u) != sext[k])
            bad |= 8;
        if (union_unsigned((int)u) != zext[k])
            bad |= 16;
        if (punned_signed(u) != sext[k])
            bad |= 32;
        if (volatile_signed(u) != sext[k])
            bad |= 64;
        if (both_views(u) != (i64)(zext[k] + 1) + sext[k])
            bad |= 128;
    }
    return bad;
}
