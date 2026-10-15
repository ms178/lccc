.text
.p2align 4
.globl padding_witness
.type padding_witness,@function
padding_witness:
.cfi_startproc
.LBB0:
    testl %edi, %edi
    je .LBB2
.LBB1:
    xorl %eax, %eax
    .p2align 4,0xf8
    setc %al
    jmp .LBB3
.LBB2:
    xorl %eax, %eax
    .p2align 4,0xf9
    setc %al
    jmp .LBB3
.LBB3:
    ret
.cfi_endproc
.size padding_witness, .-padding_witness
.section .note.GNU-stack,"",@progbits
