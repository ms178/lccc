main:
.cfi_startproc
    subq $88, %rsp
    .cfi_def_cfa_offset 96
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    leaq guard(%rip), %r11
    xorl %r10d, %r10d
.LBB1:
    cmpq $256, %r10
jge .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    movb %r10b, (%r11, %r10)
    addq $1, %r10
    cmpq $256, %r10
jl .LBB2
.LBB3:
    vpxor %ymm0, %ymm0, %ymm0
    vmovdqu %ymm0, 56(%rsp)
    leaq tree(%rip), %rdi
    xorl %edx, %edx
.LBB4:
    cmpq $144, %rdx
jge .LBB6
.p2align 4,,10
.p2align 3
.LBB5:
    movw $8, (%rdi, %rdx, 2)
    movzwl 72(%rsp), %eax
    leal 1(%rax), %r8d
    movw %r8w, 72(%rsp)
    addq $1, %rdx
    cmpq $144, %rdx
jl .LBB5
.LBB6:
    leaq 56(%rsp), %rdi
    leaq guard(%rip), %rsi
    leaq tree(%rip), %rdx
    call check@PLT
    # LCCC_CALL_ARGS 3
    # LCCC_CALL_FP 0
    movl %eax, %eax
    vzeroupper
    addq $88, %rsp
    .cfi_def_cfa_offset 8
    ret
