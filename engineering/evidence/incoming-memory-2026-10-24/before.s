.text
cf_wrap_phi:
.cfi_startproc
    subq $152, %rsp
    .cfi_def_cfa_offset 160
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    leaq 79(%rsp), %r11
    andq $-32, %r11
    movq 160(%rsp), %rax
    movq %rax, 0(%r11)
    movq 168(%rsp), %rax
    movq %rax, 8(%r11)
    movq 176(%rsp), %rax
    movq %rax, 16(%r11)
    movq 184(%rsp), %rax
    movq %rax, 24(%r11)
    movq 192(%rsp), %rax
    movq %rax, 32(%r11)
    movq 200(%rsp), %rax
    movq %rax, 40(%r11)
    movq 208(%rsp), %rax
    movq %rax, 48(%r11)
    movq 216(%rsp), %rax
    movq %rax, 56(%r11)
    vmovaps %ymm0, %ymm2
    testl %edi, %edi
jne .LBB497
.LBB496:
    jmp .LBB498
.LBB497:
    leaq 79(%rsp), %rax
    andq $-32, %rax
    vmovups (%rax), %ymm2
.LBB498:
    leaq 79(%rsp), %rax
    andq $-32, %rax
    vmovups (%rax), %ymm4
    vsubps %ymm4, %ymm2, %ymm0
    addq $152, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size cf_wrap_phi, .-cf_wrap_phi

nine:
.cfi_startproc
    subq $72, %rsp
    .cfi_def_cfa_offset 80
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movq 80(%rsp), %rax
    movq %rax, 40(%rsp)
    movq 88(%rsp), %rax
    movq %rax, 48(%rsp)
    movq 96(%rsp), %rax
    movq %rax, 56(%rsp)
    movq 104(%rsp), %rax
    movq %rax, 64(%rsp)
    vmovups 40(%rsp), %ymm10
    vaddps %ymm1, %ymm0, %ymm11
    vaddps %ymm2, %ymm11, %ymm12
    vaddps %ymm3, %ymm12, %ymm13
    vaddps %ymm4, %ymm13, %ymm14
    vaddps %ymm5, %ymm14, %ymm15
    vaddps %ymm6, %ymm15, %ymm2
    vaddps %ymm7, %ymm2, %ymm3
    vaddps %ymm10, %ymm3, %ymm0
    addq $72, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size nine, .-nine

nine4:
.cfi_startproc
    subq $24, %rsp
    .cfi_def_cfa_offset 32
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movq 32(%rsp), %rax
    movq %rax, 0(%rsp)
    movq 40(%rsp), %rax
    movq %rax, 8(%rsp)
    movaps %xmm7, %xmm8
    movdqu 0(%rsp), %xmm7
    vpaddd %xmm1, %xmm0, %xmm15
    vpaddd %xmm2, %xmm15, %xmm15
    vpaddd %xmm3, %xmm15, %xmm15
    vpaddd %xmm4, %xmm15, %xmm15
    vpaddd %xmm5, %xmm15, %xmm15
    vpaddd %xmm6, %xmm15, %xmm15
    vpaddd %xmm8, %xmm15, %xmm15
    vpaddd %xmm7, %xmm15, %xmm0
    addq $24, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size nine4, .-nine4

s4i_add:
.cfi_startproc
    subq $104, %rsp
    .cfi_def_cfa_offset 112
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, 96(%rsp)
    movq 112(%rsp), %rax
    movq %rax, 48(%rsp)
    movq 120(%rsp), %rax
    movq %rax, 56(%rsp)
    movq 128(%rsp), %rax
    movq %rax, 64(%rsp)
    movq 136(%rsp), %rax
    movq %rax, 72(%rsp)
    movdqu 48(%rsp), %xmm14
    vpaddd %xmm0, %xmm14, %xmm14
    movdqu %xmm14, 48(%rsp)
    movslq 64(%rsp), %rax
    addl $1, %eax
    movl %eax, 64(%rsp)
    movq %rdi, 0(%rsp)
    leaq 48(%rsp), %rsi
    movdqu (%rsi), %xmm0
    movdqu %xmm0, (%rdi)
    movdqu 16(%rsi), %xmm0
    movdqu %xmm0, 16(%rdi)
    movq 0(%rsp), %rax
    addq $104, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size s4i_add, .-s4i_add

sum_big:
.cfi_startproc
    subq $24, %rsp
    .cfi_def_cfa_offset 32
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq 32(%rsp), %rax
    movq %rax, 0(%rsp)
    movq 40(%rsp), %rax
    movq %rax, 8(%rsp)
    movq 48(%rsp), %rax
    movq %rax, 16(%rsp)
    movq 0(%rsp), %rax
    addq 8(%rsp), %rax
    addq 16(%rsp), %rax
    addq $24, %rsp
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size sum_big, .-sum_big
