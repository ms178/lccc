.text
cf_wrap_phi:
.cfi_startproc
    subq $40, %rsp
    .cfi_def_cfa_offset 48
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovaps %ymm0, %ymm2
    testl %edi, %edi
jne .LBB497
.LBB496:
    jmp .LBB498
.LBB497:
    vmovups 48(%rsp), %ymm2
.LBB498:
    vmovups 48(%rsp), %ymm4
    vsubps %ymm4, %ymm2, %ymm0
    addq $40, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size cf_wrap_phi, .-cf_wrap_phi

nine:
.cfi_startproc
    subq $40, %rsp
    .cfi_def_cfa_offset 48
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovups 48(%rsp), %ymm10
    vaddps %ymm1, %ymm0, %ymm11
    vaddps %ymm2, %ymm11, %ymm12
    vaddps %ymm3, %ymm12, %ymm13
    vaddps %ymm4, %ymm13, %ymm14
    vaddps %ymm5, %ymm14, %ymm15
    vaddps %ymm6, %ymm15, %ymm2
    vaddps %ymm7, %ymm2, %ymm3
    vaddps %ymm10, %ymm3, %ymm0
    addq $40, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size nine, .-nine

nine4:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movaps %xmm7, %xmm8
    movdqu 8(%rsp), %xmm7
    vpaddd %xmm1, %xmm0, %xmm15
    vpaddd %xmm2, %xmm15, %xmm15
    vpaddd %xmm3, %xmm15, %xmm15
    vpaddd %xmm4, %xmm15, %xmm15
    vpaddd %xmm5, %xmm15, %xmm15
    vpaddd %xmm6, %xmm15, %xmm15
    vpaddd %xmm8, %xmm15, %xmm15
    vpaddd %xmm7, %xmm15, %xmm0
    ret
.cfi_endproc
.size nine4, .-nine4

s4i_add:
.cfi_startproc
    subq $56, %rsp
    .cfi_def_cfa_offset 64
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movdqu 64(%rsp), %xmm14
    vpaddd %xmm0, %xmm14, %xmm14
    movdqu %xmm14, 64(%rsp)
    movslq 80(%rsp), %rax
    addl $1, %eax
    movl %eax, 80(%rsp)
    movq %rdi, 0(%rsp)
    leaq 64(%rsp), %rsi
    movdqu (%rsi), %xmm0
    movdqu %xmm0, (%rdi)
    movdqu 16(%rsi), %xmm0
    movdqu %xmm0, 16(%rdi)
    movq 0(%rsp), %rax
    addq $56, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size s4i_add, .-s4i_add

sum_big:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq 8(%rsp), %rax
    addq 16(%rsp), %rax
    addq 24(%rsp), %rax
    ret
.cfi_endproc
.size sum_big, .-sum_big
