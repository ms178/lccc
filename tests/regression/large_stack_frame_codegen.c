/* A large legal automatic object makes the x86-64 frame adjustment exceed
   the sign-extended imm32 range. Keep the function externally visible so
   -O2 must emit its prologue, but never call it at runtime. */
#define LARGE_FRAME_SIZE 0x10000000000UL

__attribute__((noinline)) int
large_frame_codegen(void)
{
    volatile unsigned char frame[LARGE_FRAME_SIZE];
    frame[0] = 0x2a;
    return frame[0];
}

int
main(void)
{
    return 0;
}
