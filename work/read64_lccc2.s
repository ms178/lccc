.text
.globl f
.p2align 4
.type f, @function
f:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    .cfi_def_cfa_offset 96
    # LCCC_RET_XMM 0
    movq %rdi, %rbx
    movq %rsi, %r12
    movq %rdx, %r13
    xorl %r14d, %r14d
    xorl %r15d, %r15d
.LBB1:
    cmpq %r13, %r15
jae .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    leaq 0(,%r15,8), %rbp
    movq (%rbx, %rbp, 1), %rax
    movq %rax, 8(%rsp)
    movq (%r12, %rbp, 1), %rax
    movq 8(%rsp), %r9
    xorq %rax, %r9
    addq %r9, %r14
    addq $1, %r15
    cmpq %r13, %r15
jb .LBB2
.LBB3:
    movq %r14, %rax
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.cfi_endproc
.size f, .-f


.section .note.GNU-stack,"",@progbits
