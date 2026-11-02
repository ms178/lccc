.text
.globl copy_q4
.p2align 4
.type copy_q4, @function
copy_q4:
.cfi_startproc
    subq $56, %rsp
    .cfi_def_cfa_offset 64
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovdqu (%rsi), %ymm0
    vmovdqu %ymm0, (%rdi)
    vzeroupper
    addq $56, %rsp
    ret
.cfi_endproc
.size copy_q4, .-copy_q4

.globl copy_d2
.p2align 4
.type copy_d2, @function
copy_d2:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movupd (%rsi), %xmm2
    movupd %xmm2, (%rdi)
    ret
.cfi_endproc
.size copy_d2, .-copy_d2

.globl copy_w4
.p2align 4
.type copy_w4, @function
copy_w4:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movdqu (%rsi), %xmm2
    movdqu %xmm2, (%rdi)
    ret
.cfi_endproc
.size copy_w4, .-copy_w4

.globl copy_b16
.p2align 4
.type copy_b16, @function
copy_b16:
.cfi_startproc
    subq $40, %rsp
    .cfi_def_cfa_offset 48
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movdqu (%rsi), %xmm0
    movdqu %xmm0, (%rdi)
    addq $40, %rsp
    ret
.cfi_endproc
.size copy_b16, .-copy_b16

.globl xor_q4
.p2align 4
.type xor_q4, @function
xor_q4:
.cfi_startproc
    subq $152, %rsp
    .cfi_def_cfa_offset 160
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovdqu (%rdx), %ymm0
    vpxor (%rsi), %ymm0, %ymm0
    vmovdqu %ymm0, (%rdi)
    vzeroupper
    addq $152, %rsp
    ret
.cfi_endproc
.size xor_q4, .-xor_q4

.globl add_q4
.p2align 4
.type add_q4, @function
add_q4:
.cfi_startproc
    subq $152, %rsp
    .cfi_def_cfa_offset 160
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovdqu (%rdx), %ymm0
    vpaddq (%rsi), %ymm0, %ymm0
    vmovdqu %ymm0, (%rdi)
    vzeroupper
    addq $152, %rsp
    ret
.cfi_endproc
.size add_q4, .-add_q4

.globl sub_d2
.p2align 4
.type sub_d2, @function
sub_d2:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movupd (%rsi), %xmm15
    movupd (%rdx), %xmm14
    subpd %xmm14, %xmm15
    movupd %xmm15, (%rdi)
    ret
.cfi_endproc
.size sub_d2, .-sub_d2

.globl mul_h8
.p2align 4
.type mul_h8, @function
mul_h8:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movdqu (%rsi), %xmm15
    movdqu (%rdx), %xmm14
    pmullw %xmm14, %xmm15
    movdqu %xmm15, (%rdi)
    ret
.cfi_endproc
.size mul_h8, .-mul_h8

.globl add_b16
.p2align 4
.type add_b16, @function
add_b16:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movdqu (%rsi), %xmm15
    movdqu (%rdx), %xmm14
    paddb %xmm14, %xmm15
    movdqu %xmm15, (%rdi)
    ret
.cfi_endproc
.size add_b16, .-add_b16

.globl ksub_w4
.p2align 4
.type ksub_w4, @function
ksub_w4:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movdqu (%rsi), %xmm15
    movl $1, %eax
    movd %eax, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqa %xmm0, %xmm14
    psubd %xmm14, %xmm15
    movdqu %xmm15, (%rdi)
    ret
.cfi_endproc
.size ksub_w4, .-ksub_w4

.globl madd_q4
.p2align 4
.type madd_q4, @function
madd_q4:
.cfi_startproc
    subq $248, %rsp
    .cfi_def_cfa_offset 256
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovdqu (%rdx), %ymm0
    vpaddq (%rsi), %ymm0, %ymm0
    vpaddq (%rcx), %ymm0, %ymm0
    vmovdqu %ymm0, (%rdi)
    vzeroupper
    addq $248, %rsp
    ret
.cfi_endproc
.size madd_q4, .-madd_q4

.globl mixed
.p2align 4
.type mixed, @function
mixed:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movupd (%rsi), %xmm2
    movupd %xmm2, (%rdi)
    movl (%rcx), %eax
    movl %eax, (%rdx)
    movl 4(%rcx), %eax
    movl %eax, 4(%rdx)
    ret
.cfi_endproc
.size mixed, .-mixed


.section .note.GNU-stack,"",@progbits
