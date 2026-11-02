/* Frame-addressing cases for stack frames past 2 GiB and past 4 GiB.
 *
 * x86-64 memory operands carry a signed 32-bit displacement, so a frame larger
 * than 2 GiB has slots that no single anchor can name:
 *
 *   |<---------------- frame_size ---------------->|
 *   %rbp ------------------------------------------ %rsp   (frame-pointer mode)
 *
 * From %rbp only the top 2 GiB of the frame is directly addressable; from %rsp
 * only the bottom 2 GiB. For a frame past 4 GiB there is a middle band no
 * anchor reaches at all, and those addresses have to be materialised in a
 * register. Three code paths do that, and each is exercised below:
 *
 *   1. Slot operands: `movq $disp, %rcx; leaq (%rsp,%rcx), %rcx` for a slot
 *      whose displacement neither anchor can encode.
 *   2. Slot *addresses*: `leaq slot` splits into `movq $disp, %reg; leaq
 *      (%base,%reg), %reg` -- flag-neutral, no scratch register, so it is legal
 *      in the middle of a pending fused compare.
 *   3. MachInst windows: a MachInst embeds its displacement, so a window that
 *      names an unreachable slot is replayed through the accumulator path
 *      instead (see `machinst_needs_text_fallback`).
 *
 * The functions are externally visible and NEVER CALLED: the frame cannot be
 * optimised away, so the prologue and every access are emitted, but nothing
 * executes. `main` therefore returns 0 without touching a giant frame -- this
 * is an assembly-production check that cannot destabilise a runner, matching
 * `large_stack_frame_codegen.c`. GCC compiles the same source, so the A/B
 * oracle compares two zero exits.
 *
 * Sizes are chosen around the encoding boundaries, not arbitrarily:
 * 0x7ffffff8 (imm32 top), 0x80000008 (> 2^31), 0x100000000 (> 2^32, the band no
 * anchor reaches), 0x180000000 (6 GiB, a wide middle band).
 */

#define HF_G ((unsigned long)0x40000000UL)          /* 1 GiB  */
#define HF_6G (6 * HF_G)
#define HF_JUST_PAST_I32 0x80000008UL

/* 1. Interior probes of one 6 GiB object.
 *
 * `big[2.5 G]` is 3.5 GiB above %rsp and 3.5 GiB below %rbp: unencodable from
 * both, so only a materialised address can reach it. `big[0]` sits at the base
 * of the frame (2 GiB+ from %rbp), `big[6 G - 1]` at the far end (2 GiB+ from
 * %rsp); between them the stores and loads cover every anchor.
 */
__attribute__((noinline)) int hf_interior_probes(void)
{
    volatile unsigned char big[HF_6G];
    big[0] = 0x11;
    big[HF_G] = 0x22;
    big[2 * HF_G] = 0x33;
    big[2 * HF_G + HF_G / 2] = 0x44;
    big[3 * HF_G + HF_G / 2] = 0x55;
    big[5 * HF_G] = 0x66;
    big[HF_6G - 1] = 0x77;
    return big[0] + big[HF_G] + big[2 * HF_G] + big[2 * HF_G + HF_G / 2]
         + big[3 * HF_G + HF_G / 2] + big[5 * HF_G] + big[HF_6G - 1];
}

/* 2. Every scalar width at a displaced interior slot.
 *
 * The displacement 0x80000008 leaves the object base far from %rbp, so the
 * loads and stores take the materialised path regardless of their width. The
 * narrow forms (`movb`/`movw`) share the typed-slot emitters, which stage
 * through %rcx when the displacement is out of range.
 */
__attribute__((noinline)) unsigned long hf_widths(void)
{
    volatile unsigned char big[3 * HF_G];
    volatile unsigned int mid_u32[4];
    volatile unsigned short mid_u16[4];
    volatile unsigned long mid_u64[2];
    unsigned long sum = 0;

    big[HF_JUST_PAST_I32] = 0x5a;
    sum += big[HF_JUST_PAST_I32];

    mid_u32[0] = 0xdeadbeefu;                    /* just past the imm32 band */
    mid_u32[1] = 0x0badf00du;
    mid_u16[0] = 0xbeefu;
    mid_u16[1] = 0xfeedu;
    mid_u64[0] = 0x0123456789abcdefUL;
    mid_u64[1] = 0xfedcba9876543210UL;

    sum += mid_u32[0] + mid_u32[1];
    sum += mid_u16[0] + mid_u16[1];
    sum += mid_u64[0] + mid_u64[1];
    sum += big[2 * HF_G + HF_JUST_PAST_I32] = (unsigned char)sum;
    return sum;
}

/* 3. Float domains in a giant frame.
 *
 * F32/F64/F128 live in the same frame as everything else; their stores go
 * through the SSE/x87 slot emitters rather than the integer ones, and those
 * take a different route to a far slot.
 */
__attribute__((noinline)) double hf_float_domains(void)
{
    volatile unsigned char big[5 * HF_G / 2];
    volatile float f[2];
    volatile double d[2];
    volatile long double ld[1];
    double sum;

    f[0] = 1.5f;
    f[1] = -2.25f;
    d[0] = 3.75;
    d[1] = -4.5;
    ld[0] = 6.25L;
    big[HF_G] = 0x7f;
    sum = (double)f[0] + (double)f[1] + d[0] + d[1] + (double)ld[0] + (double)big[HF_G];
    return sum;
}

/* 4. Register pressure: spill slots next to a giant object.
 *
 * The spill slots sit below the object, i.e. far from %rbp, and are live across
 * a call, so the call barrier forces the MachInst window to flush with slots
 * it cannot name directly.
 */
extern unsigned long hf_sink(unsigned long, unsigned long);

__attribute__((noinline)) unsigned long hf_pressure(unsigned long seed)
{
    volatile unsigned char big[3 * HF_G];
    unsigned long a0 = seed + 1, a1 = seed + 2, a2 = seed + 3, a3 = seed + 4;
    unsigned long a4 = seed + 5, a5 = seed + 6, a6 = seed + 7, a7 = seed + 8;
    unsigned long a8 = seed + 9, a9 = seed + 10, a10 = seed + 11, a11 = seed + 12;

    big[HF_G] = (unsigned char)seed;
    a0 = hf_sink(a0, a1) + a2;
    a3 = hf_sink(a3, a4) + a5;
    a6 = hf_sink(a6, a7) + a8;
    a9 = hf_sink(a9, a10) + a11;
    return a0 + a3 + a6 + a9 + big[HF_G];
}

/* 5. Addresses of far slots as call arguments.
 *
 * The address of the object base and of an interior point are both far from
 * %rbp; the call setup materialises them into the argument registers, and the
 * by-value aggregate copy is a second, independent user of the same addresses.
 */
struct hf_pair { unsigned long a, b; };

extern void hf_take_ptr(const void *);
extern void hf_take_pair(struct hf_pair);

__attribute__((noinline)) void hf_addresses(void)
{
    volatile unsigned char big[HF_6G];
    struct hf_pair p;

    big[0] = 0x01;
    big[2 * HF_G + HF_G / 2] = 0x02;
    p.a = (unsigned long)(const void *)big;
    p.b = (unsigned long)(const void *)&big[2 * HF_G + HF_G / 2];
    hf_take_ptr((const void *)big);
    hf_take_ptr((const void *)&big[2 * HF_G + HF_G / 2]);
    hf_take_pair(p);
}

/* 6. (reserved)
 *
 * A dynamic alloca in the same function as a giant object is the one shape the
 * frame-pointer anchor cannot serve: `__builtin_alloca` moves `%rsp` behind the
 * allocator's back, so the bottom-of-frame anchor is only valid before the
 * first alloca and the object's own base has to be materialised for every
 * access. The slot writers that reach a home register directly still spell
 * `(%rbp)` (see `tests/regression/check_giant_frame_dyn_alloca.sh`, which gates
 * the loud failure rather than a silently wrong encoding).
 */

/* 7. Pointer arithmetic just past 2^31 and 2^32.
 *
 * A constant byte offset in (i32::MAX, u32::MAX] must NOT be narrowed to its
 * signed 32-bit value on a 64-bit target: `base + 0xFFFFFFF0` is a genuine
 * +4294967280, four gibibytes away from `base - 16`. The pre-fix backend
 * narrowed it, so `big[0xFFFFFFF0]` and `big[0x80000000]` addressed the wrong
 * half of the object -- and did so self-consistently, which is why a store/
 * load pair alone cannot catch it.
 */
__attribute__((noinline)) unsigned long hf_gep_u32_offsets(void)
{
    volatile unsigned char big[HF_6G];
    volatile unsigned char *p = big;

    p[0x7ffffff0UL] = 0x31;                      /* below 2^31: plain disp32 */
    p[0x80000000UL] = 0x32;                      /* exactly 2^31 */
    p[0xffffffF0UL] = 0x33;                      /* in (2^31, 2^32) */
    p[HF_6G - 1] = 0x34;
    return p[0x7ffffff0UL] + p[0x80000000UL] + p[0xffffffF0UL] + p[HF_6G - 1];
}

/* 8. Block operations over a far object.
 *
 * `memcpy`/`memset` of a stack object take their destination as an address
 * computed from the frame, not as a slot operand, so they exercise a third
 * way of naming a far slot.
 */
__attribute__((noinline)) unsigned long hf_block_ops(unsigned long n)
{
    volatile unsigned char big[3 * HF_G];
    unsigned char tmp[16];
    unsigned long i;

    for (i = 0; i < 16; i++) {
        tmp[i] = (unsigned char)(i + 1);
    }
    __builtin_memcpy((void *)&big[2 * HF_G], tmp, 16);
    __builtin_memset((void *)&big[HF_G], 0x5c, 16);
    return big[2 * HF_G] + big[2 * HF_G + 15] + big[HF_G] + big[HF_G + 15] + n;
}

/* Definitions for the address-taking helpers used above. No giant-frame
 * function is ever called, so these exist only to make the translation unit
 * linkable; they are ordinary functions so the calls above are real call
 * sites with a real ABI (stack arguments, by-value aggregate copies). */
unsigned long hf_sink(unsigned long a, unsigned long b)
{
    return a + b;
}

void hf_take_ptr(const void *p)
{
    (void)p;
}

void hf_take_pair(struct hf_pair p)
{
    (void)p;
}

int main(void)
{
    return 0;
}
