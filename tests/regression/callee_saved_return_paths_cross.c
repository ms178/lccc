/* Every return path hands the callee-saved registers back intact.
 *
 * AArch64 at -O1 and -O2: the loop-invariant hoist mistook the shared
 * epilogue for a loop top -- a block below it branches back to the
 * epilogue's label -- and moved the epilogue's `ldr x21, [sp, #32]` restore
 * above that label as if it were an invariant reload.  A return through the
 * branch below then skipped the restore and handed the caller x21 as the
 * function had left it (the scan pointer).  Nothing in the function's own
 * result shows it; only the caller's register does.
 *
 * `call_checked` loads every callee-saved register with a known value, calls
 * the function and counts the registers that came back different.
 */

static char *words[9];
char **split_words(char *s);
long call_checked(char *arg, char **(*fn)(char *));

#if defined(__aarch64__)
__asm__(
    ".text\n"
    ".globl call_checked\n"
    ".type call_checked, %function\n"
    "call_checked:\n"
    "    stp x29, x30, [sp, #-96]!\n"
    "    mov x29, sp\n"
    "    stp x19, x20, [sp, #16]\n"
    "    stp x21, x22, [sp, #32]\n"
    "    stp x23, x24, [sp, #48]\n"
    "    stp x25, x26, [sp, #64]\n"
    "    stp x27, x28, [sp, #80]\n"
    "    mov x9, x1\n"
    "    movz x19, #0x1019\n"
    "    movz x20, #0x1020\n"
    "    movz x21, #0x1021\n"
    "    movz x22, #0x1022\n"
    "    movz x23, #0x1023\n"
    "    movz x24, #0x1024\n"
    "    movz x25, #0x1025\n"
    "    movz x26, #0x1026\n"
    "    movz x27, #0x1027\n"
    "    movz x28, #0x1028\n"
    "    blr x9\n"
    "    mov x0, #0\n"
    "    movz x10, #0x1019\n"
    "    cmp x19, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1020\n"
    "    cmp x20, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1021\n"
    "    cmp x21, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1022\n"
    "    cmp x22, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1023\n"
    "    cmp x23, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1024\n"
    "    cmp x24, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1025\n"
    "    cmp x25, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1026\n"
    "    cmp x26, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1027\n"
    "    cmp x27, x10\n"
    "    cinc x0, x0, ne\n"
    "    movz x10, #0x1028\n"
    "    cmp x28, x10\n"
    "    cinc x0, x0, ne\n"
    "    ldp x19, x20, [sp, #16]\n"
    "    ldp x21, x22, [sp, #32]\n"
    "    ldp x23, x24, [sp, #48]\n"
    "    ldp x25, x26, [sp, #64]\n"
    "    ldp x27, x28, [sp, #80]\n"
    "    ldp x29, x30, [sp], #96\n"
    "    ret\n"
    ".size call_checked, .-call_checked\n");
#elif defined(__riscv) && __riscv_xlen == 64
__asm__(
    ".text\n"
    ".globl call_checked\n"
    ".type call_checked, @function\n"
    "call_checked:\n"
    "    addi sp, sp, -112\n"
    "    sd ra, 104(sp)\n"
    "    sd s0, 96(sp)\n"
    "    sd s1, 88(sp)\n"
    "    sd s2, 80(sp)\n"
    "    sd s3, 72(sp)\n"
    "    sd s4, 64(sp)\n"
    "    sd s5, 56(sp)\n"
    "    sd s6, 48(sp)\n"
    "    sd s7, 40(sp)\n"
    "    sd s8, 32(sp)\n"
    "    sd s9, 24(sp)\n"
    "    sd s10, 16(sp)\n"
    "    sd s11, 8(sp)\n"
    "    mv t0, a1\n"
    "    li s0, 0x1008\n"
    "    li s1, 0x1009\n"
    "    li s2, 0x1018\n"
    "    li s3, 0x1019\n"
    "    li s4, 0x1020\n"
    "    li s5, 0x1021\n"
    "    li s6, 0x1022\n"
    "    li s7, 0x1023\n"
    "    li s8, 0x1024\n"
    "    li s9, 0x1025\n"
    "    li s10, 0x1026\n"
    "    li s11, 0x1027\n"
    "    jalr t0\n"
    "    li a0, 0\n"
    "    li t1, 0x1008\n"
    "    xor t2, s0, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1009\n"
    "    xor t2, s1, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1018\n"
    "    xor t2, s2, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1019\n"
    "    xor t2, s3, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1020\n"
    "    xor t2, s4, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1021\n"
    "    xor t2, s5, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1022\n"
    "    xor t2, s6, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1023\n"
    "    xor t2, s7, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1024\n"
    "    xor t2, s8, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1025\n"
    "    xor t2, s9, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1026\n"
    "    xor t2, s10, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    li t1, 0x1027\n"
    "    xor t2, s11, t1\n"
    "    snez t2, t2\n"
    "    add a0, a0, t2\n"
    "    ld ra, 104(sp)\n"
    "    ld s0, 96(sp)\n"
    "    ld s1, 88(sp)\n"
    "    ld s2, 80(sp)\n"
    "    ld s3, 72(sp)\n"
    "    ld s4, 64(sp)\n"
    "    ld s5, 56(sp)\n"
    "    ld s6, 48(sp)\n"
    "    ld s7, 40(sp)\n"
    "    ld s8, 32(sp)\n"
    "    ld s9, 24(sp)\n"
    "    ld s10, 16(sp)\n"
    "    ld s11, 8(sp)\n"
    "    addi sp, sp, 112\n"
    "    ret\n"
    ".size call_checked, .-call_checked\n");
#else
#error "callee-saved checker: AArch64 and RV64 only"
#endif

int main(void)
{
    static char empty[] = "", blanks[] = "   ", three[] = " x yy  zzz ", one[] = "word";
    char *input[] = {empty, blanks, three, one};
    static const int expect[] = {0, 0, 3, 1};
    for (int i = 0; i < 4; i++) {
        if (call_checked(input[i], split_words) != 0)
            return 10 + i;
        int n = 0;
        while (words[n])
            n++;
        if (n != expect[i])
            return 20 + i;
    }
    return 0;
}

/* Last in the file: the old hoist scanned the rest of the MODULE for readers
   after the loop, so a later function naming x21 hid the defect. */
__attribute__((noinline)) char **split_words(char *s)
{
    int n = 0;
    for (;;) {
        while (*s == ' ')
            s++;
        if (*s == 0)
            break;
        words[n++] = s;
        while (*s != ' ' && *s != 0)
            s++;
        if (*s == 0)
            break;
        *s++ = 0;
    }
    words[n] = 0;
    return words;
}

