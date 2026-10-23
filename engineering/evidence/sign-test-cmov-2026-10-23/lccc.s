nonnegative_difference:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    subl %esi, %edi
    movl $0, %ecx
    movl %ecx, %eax
    cmovnsl %edi, %eax
    ret
