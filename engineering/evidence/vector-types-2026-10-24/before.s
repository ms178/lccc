.globl fma_chain
fma_chain:
.cfi_startproc
    # LCCC_RET_XMM 1
    # LCCC_RET_XMM_BITS 256
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovaps %ymm1, %ymm14
    vfmadd132ps %ymm0, %ymm2, %ymm14
    vfmadd231ps %ymm3, %ymm14, %ymm0
    ret
.cfi_endproc
.size fma_chain, .-fma_chain
