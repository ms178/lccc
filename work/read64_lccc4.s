.text
.globl f
.p2align 4
.type f, @function
f:
.cfi_startproc
    pushq %rbx
    .cfi_def_cfa_offset 16
    # LCCC_RET_XMM 0
    xorl %r8d, %r8d
    xorl %r9d, %r9d
.LBB1:
    cmpq %rdx, %r9
jae .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    leaq 0(,%r9,8), %r11
    movq (%rdi, %r11, 1), %rbx
    leaq (%rsi, %r11, 1), %r10
    movq (%r10), %r11
    xorq %rbx, %r11
    addq %r11, %r8
    addq $1, %r9
    cmpq %rdx, %r9
jb .LBB2
.LBB3:
    movq %r8, %rax
    popq %rbx
    ret
.cfi_endproc
.size f, .-f


.section .note.GNU-stack,"",@progbits
