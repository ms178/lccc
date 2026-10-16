.text
.globl vector_witness
.type vector_witness,@function
vector_witness:
.cfi_startproc
    movq %rdi, %xmm0
    movapd %xmm0, %xmm2
    .cfi_remember_state; .byte 0x66,0x0f,0xef,0xc0
    .cfi_restore_state
    movq %xmm0, %rax
    ret
.cfi_endproc
.size vector_witness, .-vector_witness
.section .note.GNU-stack,"",@progbits
