.section .rodata
.Lstr0:
    .byte 37, 103, 32, 37, 103, 10, 0

.text
.globl f
.p2align 4
.type f, @function
f:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 1
    movsd (%rdi), %xmm2
    movl $0, (%rsi)
    movl $1079558144, 4(%rsi)
    vmulsd .LCFP_0(%rip), %xmm2, %xmm2
    movsd %xmm2, %xmm0
    ret
.cfi_endproc
.size f, .-f

.globl g
.p2align 4
.type g, @function
g:
.cfi_startproc
    subq $24, %rsp
    .cfi_def_cfa_offset 32
    # LCCC_RET_XMM 1
    movq (%rdi), %rax
    movq %rax, 16(%rsp)
    movq $0, (%rsi)
    movq 16(%rsp), %rax
    movq %rax, 8(%rsp)
    movsd 8(%rsp), %xmm2
    vaddsd .LCFP_1(%rip), %xmm2, %xmm2
    movsd %xmm2, %xmm0
    addq $24, %rsp
    ret
.cfi_endproc
.size g, .-g

.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    subq $40, %rsp
    .cfi_def_cfa_offset 48
    # LCCC_RET_XMM 0
    movl $0, 32(%rsp)
    movl $1073217536, 36(%rsp)
    movabsq $4611686018427387904, %rax
    movq %rax, 24(%rsp)
    leaq 32(%rsp), %rax
    movq %rax, %rdi
    movq %rax, %rsi
    call f@PLT
    movsd %xmm0, 16(%rsp)
    leaq 24(%rsp), %rax
    movq %rax, %rdi
    movq %rax, %rsi
    call g@PLT
    movapd %xmm0, %xmm2
    leaq .Lstr0(%rip), %rdi
    movsd 16(%rsp), %xmm0
    movsd %xmm2, %xmm1
    movb $2, %al
    call printf@PLT
    xorl %eax, %eax
    addq $40, %rsp
    ret
.cfi_endproc
.size main, .-main

.section .rodata
.p2align 4
.LCFP_0:
    .quad 4611686018427387904
    .quad 0
.p2align 4
.LCFP_1:
    .quad 4607182418800017408
    .quad 0

.section .note.GNU-stack,"",@progbits
