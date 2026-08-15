# pextrw/pextrb/pextrd/pextrq + pinsrw encoding parity with GNU as.
#
# The legacy pextrw form (66 0F C5) is REGISTER-ONLY per the Intel SDM;
# memory destinations require the SSE4.1 form 66 0F 3A 15 /r ib. lccc
# historically emitted the legacy bytes for the memory form, silently
# producing a different instruction. Verified against godbolt oracles
# (gcc trunk, clang trunk, icx 2026.0.0, icc 2021.10): all four emit
# exactly these encodings.
    .text
    .globl sse_extract_insert
sse_extract_insert:
    # legacy reg form: 66 0F C5 /r ib
    pextrw $3, %xmm2, %eax
    pextrw $0, %xmm0, %ecx
    pextrw $7, %xmm15, %r10d
    # SSE4.1 mem form: 66 0F 3A 15 /r ib
    pextrw $3, %xmm2, (%rdi)
    pextrw $5, %xmm9, 8(%rdi)
    pextrw $1, %xmm0, 16(%rsp)
    # sibling extracts (always SSE4.1 3A-space)
    pextrb $2, %xmm1, %eax
    pextrb $2, %xmm1, (%rdi)
    pextrd $1, %xmm3, %edx
    pextrd $1, %xmm3, 4(%rsi)
    pextrq $1, %xmm4, %rax
    pextrq $1, %xmm4, (%rdx)
    # pinsrw both directions
    pinsrw $2, %eax, %xmm1
    pinsrw $2, (%rdi), %xmm1
    ret
