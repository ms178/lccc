.text
.globl lccc_d4sl
.p2align 4
.type lccc_d4sl, @function
lccc_d4sl:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovd %edi, %xmm14
    vpslld %xmm14, %xmm0, %xmm0
    ret
.cfi_endproc
.size lccc_d4sl, .-lccc_d4sl

.globl lccc_d4sr
.p2align 4
.type lccc_d4sr, @function
lccc_d4sr:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovd %edi, %xmm14
    vpsrld %xmm14, %xmm0, %xmm0
    ret
.cfi_endproc
.size lccc_d4sr, .-lccc_d4sr

.globl lccc_d4sa
.p2align 4
.type lccc_d4sa, @function
lccc_d4sa:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovd %edi, %xmm14
    vpsrad %xmm14, %xmm0, %xmm0
    ret
.cfi_endproc
.size lccc_d4sa, .-lccc_d4sa

.globl lccc_d8sl
.p2align 4
.type lccc_d8sl, @function
lccc_d8sl:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovd %edi, %xmm15
    vpslld %xmm15, %ymm0, %ymm0
    ret
.cfi_endproc
.size lccc_d8sl, .-lccc_d8sl

.globl lccc_d8sr
.p2align 4
.type lccc_d8sr, @function
lccc_d8sr:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovd %edi, %xmm15
    vpsrld %xmm15, %ymm0, %ymm0
    ret
.cfi_endproc
.size lccc_d8sr, .-lccc_d8sr

.globl lccc_d8sa
.p2align 4
.type lccc_d8sa, @function
lccc_d8sa:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovd %edi, %xmm15
    vpsrad %xmm15, %ymm0, %ymm0
    ret
.cfi_endproc
.size lccc_d8sa, .-lccc_d8sa

.globl lccc_q2sl
.p2align 4
.type lccc_q2sl, @function
lccc_q2sl:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movaps %xmm0, %xmm15
    movl %edi, %esi
    movq %rsi, %xmm0
    vpsllq %xmm0, %xmm15, %xmm0
    ret
.cfi_endproc
.size lccc_q2sl, .-lccc_q2sl

.globl lccc_q2sr
.p2align 4
.type lccc_q2sr, @function
lccc_q2sr:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 128
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movaps %xmm0, %xmm15
    movl %edi, %esi
    movq %rsi, %xmm0
    vpsrlq %xmm0, %xmm15, %xmm0
    ret
.cfi_endproc
.size lccc_q2sr, .-lccc_q2sr

.globl lccc_q4sl
.p2align 4
.type lccc_q4sl, @function
lccc_q4sl:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl %edi, %esi
    vmovq %rsi, %xmm15
    vpsllq %xmm15, %ymm0, %ymm0
    ret
.cfi_endproc
.size lccc_q4sl, .-lccc_q4sl

.globl lccc_q4sr
.p2align 4
.type lccc_q4sr, @function
lccc_q4sr:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl %edi, %esi
    vmovq %rsi, %xmm15
    vpsrlq %xmm15, %ymm0, %ymm0
    ret
.cfi_endproc
.size lccc_q4sr, .-lccc_q4sr


.section .note.GNU-stack,"",@progbits
