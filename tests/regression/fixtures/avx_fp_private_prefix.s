.section .rodata
.Lstr0:
    .ascii "avx\137fp\137private checksum\075\045llu\012\000"

.text
.globl avx_private_frame
.p2align 4
.type avx_private_frame, @function
avx_private_frame:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vxorps %ymm2, %ymm2, %ymm2
    vbroadcastss .LCFP_0(%rip), %ymm15
    vmovdqu (%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, (%rdi)
    vmovdqu 32(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 32(%rdi)
    vmovdqu 64(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 64(%rdi)
    vmovdqu 96(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 96(%rdi)
    vmovdqu 128(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 128(%rdi)
    vmovdqu 160(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 160(%rdi)
    vmovdqu 192(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 192(%rdi)
    vmovdqu 224(%rsi), %ymm0
    vminps %ymm15, %ymm0, %ymm0
    vmaxps %ymm2, %ymm0, %ymm0
    vmovdqu %ymm0, 224(%rdi)
    vzeroupper
    ret
.cfi_endproc
.size avx_private_frame, .-avx_private_frame

.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    pushq %rbx
    .cfi_def_cfa_offset 16
    .cfi_offset %rbx, -16
    pushq %r12
    .cfi_def_cfa_offset 24
    .cfi_offset %r12, -24
    pushq %r13
    .cfi_def_cfa_offset 32
    .cfi_offset %r13, -32
    subq $800, %rsp
    .cfi_def_cfa_offset 832
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    vmovdqa .LCVEC_1(%rip), %ymm15
    xorl %r12d, %r12d
    leaq 431(%rsp), %rbx
    andq $-32, %rbx
.LBB2:
    cmpq $8, %r12
    jge .LBB4
.p2align 5,,15
.p2align 4
.LBB3:
    vmovups %ymm15, (%rbx)
    addq $1, %r12
    leaq 32(%rbx), %rbx
    cmpq $8, %r12
    jl .LBB3
.LBB4:
    xorl %ebx, %ebx
    movl $1, %r12d
    xorl %r13d, %r13d
.LBB5:
    cmpl $2000000, %ebx
    jge .LBB7
.p2align 5,,15
.p2align 4
.LBB6:
    imull $1664525, %r12d, %r10d
    leal 1013904223(%r10), %r12d
    leaq 431(%rsp), %rax
    andq $-32, %rax
    vmovups (%rax), %ymm0
    vmovdqu %ymm0, 0(%rsp)
    movl %r12d, %r8d
    andl $8388607, %r8d
    orl $1040187392, %r8d
    vmovd %r8d, %xmm0
    vpbroadcastd %xmm0, %ymm15
    vmovdqu 0(%rsp), %ymm0
    vblendps $1, %ymm15, %ymm0, %ymm0
    leaq 751(%rsp), %rdi
    andq $-32, %rdi
    vmovdqu %ymm0, (%rdi)
    leaq 431(%rsp), %rdi
    andq $-32, %rdi
    vmovups %ymm0, (%rdi)
    leaq 111(%rsp), %rdi
    andq $-32, %rdi
    leaq 431(%rsp), %rsi
    andq $-32, %rsi
    vzeroupper
    call avx_private_frame@PLT
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    leaq 111(%rsp), %rax
    andq $-32, %rax
    vmovups (%rax), %ymm2
    vmovups %ymm2, 48(%rsp)
    vextractps $0, %xmm2, %r9d
    addq %r9, %r13
    addl $1, %ebx
    cmpl $2000000, %ebx
    jl .LBB6
.LBB7:
    leaq .Lstr0(%rip), %rdi
    movq %r13, %rsi
    xorl %eax, %eax
    vzeroupper
    call printf@PLT
    # LCCC_VA_CALL
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    xorl %eax, %eax
    vzeroupper
    addq $800, %rsp
    .cfi_def_cfa_offset 32
    popq %r13
    .cfi_def_cfa_offset 24
    popq %r12
    .cfi_def_cfa_offset 16
    popq %rbx
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size main, .-main

.section .rodata
.p2align 4
.LCFP_0:
    .quad 1065353216
    .quad 0
.p2align 5
.LCVEC_1:
    .quad 4539628425446424576
    .quad 4539628425446424576
    .quad 4539628425446424576
    .quad 4539628425446424576

.section .note.GNU-stack,"",@progbits
