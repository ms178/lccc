/* i686 label-reference safety.
 *
 * Two defects in the text threader:
 *
 * 1. Static `&&label` initializers are emitted in the data section before
 *    every function body. A later function's peephole window does not see
 *    that `.long .LBBn`. Deleting a pure trampoline left the initializer
 *    unresolved (GAS address 0). Rewriting the same `.long` as if it were a
 *    jump table collapses `&&mid` and `&&end`.
 *
 * 2. `jecxz`/`jcxz`/`loop` are rel8-only. Retargeting one across a relay
 *    whose final target is more than 127 bytes away fails in the assembler.
 *    The relay is in range; the ultimate target is not.
 *
 * decoy() is first so hop()'s data reference is outside hop's window.
 */
#if !defined(__i386__)
#error this regression is the i686 label-reference contract
#endif

__attribute__((noinline)) int decoy(int x) { return x + 3; }

__attribute__((noinline)) int hop(int which) {
    static const void *table[] = {&&mid, &&end};
    if (which < 0)
        return table[0] == table[1] ? 9 : 8;
    if (which == 2)
        goto filler;
    goto *table[which];
mid:
    goto end;
filler:
    return 4;
end:
    return 1;
}

/* Hand-written bodies, not #APP regions: the peephole must see the
 * short-only branch as a compiler line. The relay is within rel8 of the
 * branch; `.space 200` puts the ultimate target out of rel8 range. */
__asm__(
    ".globl jecxz_fwd\n"
    ".type jecxz_fwd, @function\n"
    "jecxz_fwd:\n"
    ".cfi_startproc\n"
    "    movl 4(%esp), %ecx\n"
    "    jecxz .Ljecxz_relay_fwd\n"
    "    movl $2, %eax\n"
    "    ret\n"
    ".Ljecxz_relay_fwd:\n"
    "    jmp .Ljecxz_far_fwd\n"
    ".Ljecxz_pad_fwd:\n"
    "    .space 200\n"
    ".Ljecxz_far_fwd:\n"
    "    movl $1, %eax\n"
    "    ret\n"
    ".cfi_endproc\n"
    ".size jecxz_fwd, .-jecxz_fwd\n"
    "\n"
    ".globl jecxz_back\n"
    ".type jecxz_back, @function\n"
    "jecxz_back:\n"
    ".cfi_startproc\n"
    "    jmp .Ljecxz_entry_back\n"
    ".Ljecxz_far_back:\n"
    "    movl $1, %eax\n"
    "    ret\n"
    ".Ljecxz_pad_back:\n"
    "    .space 200\n"
    ".Ljecxz_entry_back:\n"
    "    movl 4(%esp), %ecx\n"
    "    jecxz .Ljecxz_relay_back\n"
    "    movl $2, %eax\n"
    "    ret\n"
    ".Ljecxz_relay_back:\n"
    "    jmp .Ljecxz_far_back\n"
    ".cfi_endproc\n"
    ".size jecxz_back, .-jecxz_back\n"
    "\n"
    ".globl jcxz_fwd\n"
    ".type jcxz_fwd, @function\n"
    "jcxz_fwd:\n"
    ".cfi_startproc\n"
    "    movl 4(%esp), %ecx\n"
    "    jcxz .Ljcxz_relay_fwd\n"
    "    movl $2, %eax\n"
    "    ret\n"
    ".Ljcxz_relay_fwd:\n"
    "    jmp .Ljcxz_far_fwd\n"
    ".Ljcxz_pad_fwd:\n"
    "    .space 200\n"
    ".Ljcxz_far_fwd:\n"
    "    movl $1, %eax\n"
    "    ret\n"
    ".cfi_endproc\n"
    ".size jcxz_fwd, .-jcxz_fwd\n"
    "\n"
    ".globl loop_fwd\n"
    ".type loop_fwd, @function\n"
    "loop_fwd:\n"
    ".cfi_startproc\n"
    "    movl 4(%esp), %ecx\n"
    "    loop .Lloop_relay_fwd\n"
    "    movl $2, %eax\n"
    "    ret\n"
    ".Lloop_relay_fwd:\n"
    "    jmp .Lloop_far_fwd\n"
    ".Lloop_pad_fwd:\n"
    "    .space 200\n"
    ".Lloop_far_fwd:\n"
    "    movl $1, %eax\n"
    "    ret\n"
    ".cfi_endproc\n"
    ".size loop_fwd, .-loop_fwd\n");

int jecxz_fwd(int n);
int jecxz_back(int n);
int jcxz_fwd(int n);
int loop_fwd(int n);

int main(void) {
    if (decoy(4) != 7)
        return 10;
    if (hop(0) != 1)
        return 11;
    if (hop(1) != 1)
        return 12;
    if (hop(2) != 4)
        return 13;
    if (hop(-1) != 8)
        return 14;
    if (jecxz_fwd(0) != 1)
        return 21;
    if (jecxz_fwd(3) != 2)
        return 22;
    if (jecxz_back(0) != 1)
        return 23;
    if (jecxz_back(5) != 2)
        return 24;
    if (jcxz_fwd(0) != 1)
        return 25;
    if (jcxz_fwd(4) != 2)
        return 26;
    /* loop decrements ecx, then jumps if the result is nonzero.
     * ecx==1 falls through (2); any other entry count takes the relay (1). */
    if (loop_fwd(1) != 2)
        return 27;
    if (loop_fwd(0) != 1)
        return 28;
    if (loop_fwd(2) != 1)
        return 29;
    return 0;
}
