#include <stdio.h>
__asm__(".text\n.p2align 4\n.globl padding_witness\n.type padding_witness,@function\npadding_witness:\n.cfi_startproc\n.LBB0:\n    testl %edi, %edi\n    je .LBB2\n.LBB1:\n    xorl %eax, %eax\n    .p2align 4,0xf8\n    setc %al\n    jmp .LBB3\n.LBB2:\n    xorl %eax, %eax\n    .p2align 4,0xf9\n    setc %al\n    jmp .LBB3\n.LBB3:\n    ret\n.cfi_endproc\n.size padding_witness, .-padding_witness\n.section .note.GNU-stack,\"\",@progbits\n");
int padding_witness(int value);
int main(void) {
 printf("%s\n", __VERSION__);
#ifdef __INTEL_LLVM_COMPILER
 printf("INTEL_LLVM=%ld\n", (long)__INTEL_LLVM_COMPILER);
#endif

    for (int value = -3; value <= 3; ++value) {
        if (padding_witness(value) != (value == 0))
            return 13 + value;
    }
    return 0;
}
