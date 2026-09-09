.section .rodata
.Lstr0:
    .byte 37, 103, 32, 37, 103, 10, 0

.text
.globl f
.type f, @function
f:
.cfi_startproc
    pushq %rbp
    .cfi_def_cfa_offset 16
    .cfi_offset %rbp, -16
    movq %rsp, %rbp
    .cfi_def_cfa_register %rbp
    subq $48, %rsp
    # LCCC_RET_XMM 1
    movq %rdi, -8(%rbp)
    movq %rsi, -16(%rbp)
    movq %rdi, %rax
    movq %rdi, -24(%rbp)
    movsd (%rax), %xmm0
    movsd %xmm0, -32(%rbp)
    movq -16(%rbp), %rax
    movq %rax, -40(%rbp)
    movsd .LCFP_0(%rip), %xmm0
    movsd %xmm0, (%rax)
    movsd -32(%rbp), %xmm0
    vmulsd .LCFP_1(%rip), %xmm0, %xmm0
    movsd %xmm0, -48(%rbp)
    movsd -48(%rbp), %xmm0
    movq %rbp, %rsp
    popq %rbp
    ret
.cfi_endproc
.size f, .-f

.globl g
.type g, @function
g:
.cfi_startproc
    pushq %rbp
    .cfi_def_cfa_offset 16
    .cfi_offset %rbp, -16
    movq %rsp, %rbp
    .cfi_def_cfa_register %rbp
    subq $64, %rsp
    # LCCC_RET_XMM 1
    movq %rdi, -8(%rbp)
    movq %rsi, -16(%rbp)
    movq %rdi, %rax
    movq %rdi, -40(%rbp)
    movq (%rax), %rax
    movq %rax, -24(%rbp)
    movq -16(%rbp), %rax
    movq %rax, -48(%rbp)
    movq $0, (%rax)
    movq -24(%rbp), %rax
    movq %rax, -32(%rbp)
    movsd -32(%rbp), %xmm0
    movsd %xmm0, -56(%rbp)
    movsd -56(%rbp), %xmm0
    vaddsd .LCFP_2(%rip), %xmm0, %xmm0
    movsd %xmm0, -64(%rbp)
    movsd -64(%rbp), %xmm0
    movq %rbp, %rsp
    popq %rbp
    ret
.cfi_endproc
.size g, .-g

.globl main
.type main, @function
main:
.cfi_startproc
    pushq %rbp
    .cfi_def_cfa_offset 16
    .cfi_offset %rbp, -16
    movq %rsp, %rbp
    .cfi_def_cfa_register %rbp
    subq $48, %rsp
    # LCCC_RET_XMM 0
    movl $0, -8(%rbp)
    movl $1073217536, -4(%rbp)
    movabsq $4611686018427387904, %rax
    movq %rax, -16(%rbp)
    leaq -8(%rbp), %rax
    movq %rax, %rdi
    movq %rax, %rsi
    call f@PLT
    movsd %xmm0, -24(%rbp)
    leaq -16(%rbp), %rax
    movq %rax, %rdi
    movq %rax, %rsi
    call g@PLT
    movsd %xmm0, -32(%rbp)
    leaq .Lstr0(%rip), %rdi
    movsd -24(%rbp), %xmm0
    movq -32(%rbp), %rax
    movq %rax, %xmm1
    movb $2, %al
    call printf@PLT
    xorl %eax, %eax
    cltq
    movl %eax, -36(%rbp)
    movq %rbp, %rsp
    popq %rbp
    ret
.cfi_endproc
.size main, .-main

.section .rodata
.p2align 4
.LCFP_0:
    .quad 4636666922610458624
    .quad 0
.p2align 4
.LCFP_1:
    .quad 4611686018427387904
    .quad 0
.p2align 4
.LCFP_2:
    .quad 4607182418800017408
    .quad 0

.section .note.GNU-stack,"",@progbits
