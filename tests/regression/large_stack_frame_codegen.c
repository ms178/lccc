/* Frame adjustments that a sign-extended imm32 cannot encode.
 *
 * x86's `add/sub r64, imm` carries only a sign-extended 32-bit immediate, so a
 * legal large automatic object has no immediate encoding for its frame delta.
 * The backend materializes such a delta into caller-saved %r11 and uses the
 * register form; cfi_synth models that exact adjacency so the synthesized CFA
 * stays exact for these functions.
 *
 * Three ranges are covered, because the encoder resolves them differently:
 *   0x10000000000  > 2^32   -> movabs (B8+rd io, 10 bytes)
 *   0x80000008     > 2^31   -> zero-extending 32-bit mov (B8+rd id, 6 bytes)
 *   0x7fffffff     <= imm32 -> the immediate form, which must stay unchanged
 *
 * Every function is externally visible and never called: the prologue must be
 * generated (the frame cannot be optimized away), and nothing executes, so this
 * is a pure assembly-production check that cannot destabilize a runner.
 */
#define PAST_U32 ((unsigned long)0x10000000000UL)
#define PAST_I32 ((unsigned long)0x80000008UL)
#define AT_I32 ((unsigned long)0x7ffffff8UL)

__attribute__((noinline)) int large_frame_past_u32(void)
{
    volatile unsigned char frame[PAST_U32];
    frame[0] = 0x2a;
    return frame[0];
}

__attribute__((noinline)) int large_frame_past_i32(void)
{
    volatile unsigned char frame[PAST_I32];
    frame[0] = 0x2b;
    return frame[0];
}

/* Exactly at the imm32 boundary (0x7ffffff8 fits): the short immediate form
 * must survive, so this function guards against a "fix" that routes every
 * frame through %r11. */
__attribute__((noinline)) int large_frame_at_i32(void)
{
    volatile unsigned char frame[AT_I32];
    frame[0] = 0x2c;
    return frame[0];
}

int main(void)
{
    return 0;
}
