# Each pair differs only by deletion of the marked candidate instruction.
# Non-privileged FXSAVE/FXRSTOR/XSAVE are sufficient to observe dependencies;
# this does not claim execution coverage of privileged XSAVES/XRSTORS.
.text
.macro SHA_WITNESS name, keep
.globl \name
.type \name, @function
\name:
    vpxor %xmm0, %xmm0, %xmm0
    vpcmpeqd %xmm6, %xmm6, %xmm6
    vpxor %xmm5, %xmm5, %xmm5
    vpxor %xmm7, %xmm7, %xmm7
.if \keep
    movaps %xmm6, %xmm0 # candidate: implicit message input
.endif
    sha256rnds2 %xmm5, %xmm7
    ptest %xmm7, %xmm7
    setnz %al
    movzbl %al, %eax
    ret
.size \name, .-\name
.endm
SHA_WITNESS original_sha, 1
SHA_WITNESS removed_sha, 0

.macro SAVE_WITNESS name, keep, upper
.globl \name
.type \name, @function
\name:
    pushq %rbp
    movq %rsp, %rbp
    subq $1152, %rsp
    andq $-64, %rsp
    # Deterministic image even when XSAVE omits an initial-state component.
    movq %rsp, %rdi
    xorl %eax, %eax
    movl $128, %ecx
    rep stosq
    vpcmpeqd %ymm7, %ymm7, %ymm7
    vpxor %xmm6, %xmm6, %xmm6
    vpxor %xmm5, %xmm5, %xmm5
.if \keep
    vmovaps %xmm6, %xmm7 # candidate: low data + upper zeros
.endif
.if \upper
    movaps %xmm5, %xmm7 # legacy overwrite kills low, not high
    movl $6, %eax # request SSE and AVX only; image fits 1024 bytes
    xorl %edx, %edx
    xsave64 (%rsp)
    cmpq $0, 688(%rsp) # standard image: 576 + YMM7 * 16
.else
    fxsave64 (%rsp)
    cmpq $0, 272(%rsp) # legacy image: 160 + XMM7 * 16
.endif
    sete %al
    movzbl %al, %eax
    vzeroupper
    leave
    ret
.size \name, .-\name
.endm
SAVE_WITNESS original_fxsave, 1, 0
SAVE_WITNESS removed_fxsave, 0, 0
SAVE_WITNESS original_xsave_upper, 1, 1
SAVE_WITNESS removed_xsave_upper, 0, 1

.macro RESTORE_WITNESS name, keep
.globl \name
.type \name, @function
\name:
    pushq %rbp
    movq %rsp, %rbp
    subq $512, %rsp
    vpxor %xmm7, %xmm7, %xmm7
    fxsave64 (%rsp)
    movl $1, %eax
    movq %rax, %xmm6
.if \keep
    movsd %xmm6, 272(%rsp) # candidate inside image, far from its base
.endif
    fxrstor64 (%rsp)
    movq %xmm7, %rax
    leave
    ret
.size \name, .-\name
.endm
RESTORE_WITNESS original_fxrstor_slot, 1
RESTORE_WITNESS removed_fxrstor_slot, 0
# These pairs execute the original and illegally rewritten vector-copy shapes.
.macro MASK_COPY name, rewrite
.globl \name
.type \name, @function
\name:
    vpcmpeqd %xmm0, %xmm0, %xmm0
    movapd %xmm0, %xmm2
    vpxor %xmm3, %xmm3, %xmm3
    vpxor %xmm5, %xmm5, %xmm5
    vpxor %xmm7, %xmm7, %xmm7
    vpcmpistrm $0, %xmm5, %xmm7 # implicit xmm0 becomes zero
.if \rewrite
    vorpd %xmm0, %xmm3, %xmm4 # stale relation substituted xmm0 for xmm2
.else
    vorpd %xmm2, %xmm3, %xmm4
.endif
    ptest %xmm4, %xmm4
    setnz %al
    movzbl %al, %eax
    ret
.size \name, .-\name
.endm
MASK_COPY original_mask_copy, 0
MASK_COPY rewritten_mask_copy, 1

.macro MASK_WIDTH name, rewrite
.globl \name
.type \name, @function
\name:
    vpxor %xmm5, %xmm5, %xmm5
    vpxor %xmm7, %xmm7, %xmm7
    pcmpistrm $0, %xmm5, %xmm7 # defines xmm0, including its upper lane
    vpcmpeqd %xmm3, %xmm3, %xmm3
.if \rewrite
    movapd %xmm3, %xmm0 # not a private/undefined upper lane
.else
    movsd %xmm3, %xmm0
.endif
    pextrq $1, %xmm0, %rax
    testq %rax, %rax
    sete %al
    movzbl %al, %eax
    ret
.size \name, .-\name
.endm
MASK_WIDTH original_mask_width, 0
MASK_WIDTH rewritten_mask_width, 1

.macro SAVED_BRACKET name, rewrite
.globl \name
.type \name, @function
\name:
    pushq %rbp
    movq %rsp, %rbp
    subq $512, %rsp
    vpxor %xmm0, %xmm0, %xmm0
    vpcmpeqd %xmm3, %xmm3, %xmm3
.if \rewrite
    vorpd %xmm3, %xmm0, %xmm0
    fxsave64 (%rsp)
.else
    movapd %xmm0, %xmm2
    vorpd %xmm3, %xmm2, %xmm2
    fxsave64 (%rsp)
    movapd %xmm2, %xmm0
.endif
    cmpq $0, 160(%rsp) # image observes xmm0 before the copy-out
    sete %al
    movzbl %al, %eax
    leave
    ret
.size \name, .-\name
.endm
SAVED_BRACKET original_saved_bracket, 0
SAVED_BRACKET rewritten_saved_bracket, 1

.macro BLEND_RESULT name, rewrite
.globl \name
.type \name, @function
\name:
    vpxor %xmm0, %xmm0, %xmm0
    vpxor %xmm2, %xmm2, %xmm2
    vpxor %xmm7, %xmm7, %xmm7
    vpcmpeqd %xmm3, %xmm3, %xmm3
    movsd %xmm0, %xmm3 # source's upper lane is all ones, low lane zero
    vpcmpeqd %xmm5, %xmm5, %xmm5
.if \rewrite
    vaddsd %xmm2, %xmm3, %xmm0 # changes the implicit mask's upper lane
.else
    vaddsd %xmm2, %xmm3, %xmm4
    movsd %xmm4, %xmm0 # preserves xmm0's zero upper lane
.endif
    blendvpd %xmm5, %xmm7 # implicit xmm0 selects each lane separately
    pextrq $1, %xmm7, %rax
    testq %rax, %rax
    sete %al
    movzbl %al, %eax
    ret
.size \name, .-\name
.endm
BLEND_RESULT original_blend_result, 0
BLEND_RESULT rewritten_blend_result, 1
.macro INDEX_SPILL name, rewrite
.globl \name
.type \name, @function
\name:
    subq $8, %rsp
    movl $37, %ecx
    movq %rcx, (%rsp)
    vpxor %xmm5, %xmm5, %xmm5
    vpxor %xmm7, %xmm7, %xmm7
    vpcmpistri $0, %xmm5, %xmm7 # implicit ECX index, not a vector output
.if \rewrite
    movq %rcx, %rax # stale register-backed spill home
.else
    movq (%rsp), %rax
.endif
    cmpq $37, %rax
    sete %al
    movzbl %al, %eax
    addq $8, %rsp
    ret
.size \name, .-\name
.endm
INDEX_SPILL original_index_spill, 0
INDEX_SPILL rewritten_index_spill, 1
.section .note.GNU-stack,"",@progbits
