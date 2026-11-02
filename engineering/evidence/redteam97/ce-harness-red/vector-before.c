#include <stdio.h>
__asm__(".text\n.globl vector_witness\n.type vector_witness,@function\nvector_witness:\n.cfi_startproc\n    movq %rdi, %xmm0\n    movapd %xmm0, %xmm2\n    .cfi_remember_state; .byte 0x66,0x0f,0xef,0xc0\n    .cfi_restore_state\n    movq %xmm0, %rax\n    ret\n.cfi_endproc\n.size vector_witness, .-vector_witness\n.section .note.GNU-stack,\"\",@progbits\n");
unsigned long long vector_witness(unsigned long long value);
int main(void) {
 printf("%s\n", __VERSION__);
#ifdef __INTEL_LLVM_COMPILER
 printf("INTEL_LLVM=%ld\n", (long)__INTEL_LLVM_COMPILER);
#endif

    const unsigned long long values[] = {0, 1, 0x8000000000000000ULL, ~0ULL};
    for (unsigned i = 0; i < 4; ++i) {
        if (vector_witness(values[i]) != values[i])
            return 1;
    }
    return 0;
}
