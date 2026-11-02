/* Address-taken blocks need IBT landing pads, not merely function entries.
 * The regular corpus runs without IBT; check_kernel_codegen_contracts.py
 * additionally executes this with branch/full protection and reads the actual
 * target bytes. Static initializers and Instruction::LabelAddr are distinct
 * IR paths. No library or OS services are required by this fixture. */
#ifndef TEST_IBT
#define TEST_IBT 0
#endif

static int valid_target(const void *address) {
#if TEST_IBT
    const volatile unsigned char *p = address;
# if defined(__i386__)
    const unsigned char last = 0xfb;
# else
    const unsigned char last = 0xfa;
# endif
    return p[0] == 0xf3 && p[1] == 0x0f && p[2] == 0x1e && p[3] == last;
#else
    (void)address;
    return 1;
#endif
}

__attribute__((noinline)) int static_dispatch(unsigned i) {
    static void *table[] = { &&one, &&two, &&three };
    void *target = table[i % 3];
    if (!valid_target(target)) return -100;
    goto *target;
one: return 11;
two: return 17;
three: return 23;
}

__attribute__((noinline)) int local_dispatch(unsigned i) {
    void *target = i ? &&yes : &&no;
    if (!valid_target(target)) return -200;
    goto *target;
yes: return 29;
no: return 31;
}

/* Merely listing a direct asm-goto destination does not make it an indirect
 * target. The focused gate also checks the disassembly for spurious pads. */
__attribute__((noinline)) int asm_only(int n) {
#if defined(__x86_64__) || defined(__i386__)
    __asm__ goto ("test %0, %0\n\tjnz %l[out]" : : "r"(n) : "cc" : out);
    return 37;
out: return 41;
#else
    return n ? 41 : 37;
#endif
}

int main(void) {
    if (!valid_target((void *)static_dispatch) ||
        !valid_target((void *)local_dispatch) ||
        !valid_target((void *)asm_only)) return 4;
    volatile unsigned i;
    for (i = 0; i < 12; ++i) {
        int expected = i % 3 == 0 ? 11 : (i % 3 == 1 ? 17 : 23);
        if (static_dispatch(i) != expected) return 1;
        if (local_dispatch(i & 1) != ((i & 1) ? 29 : 31)) return 2;
        if (asm_only(i != 0) != (i ? 41 : 37)) return 3;
    }
    return 0;
}
