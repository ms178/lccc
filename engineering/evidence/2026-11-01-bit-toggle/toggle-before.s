.text
.globl btog32
.p2align 4
.type btog32, @function
btog32:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    xorl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size btog32, .-btog32

.globl btog64
.p2align 4
.type btog64, @function
btog64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shlq %cl, %rdx
    xorq %rdx, %rdi
    movq %rdi, %rax
    ret
.cfi_endproc
.size btog64, .-btog64

.globl btog_masked
.p2align 4
.type btog_masked, @function
btog_masked:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    xorl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size btog_masked, .-btog_masked

.globl btog_imm
.p2align 4
.type btog_imm, @function
btog_imm:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorq $32, %rdi
    movl %edi, %eax
    ret
.cfi_endproc
.size btog_imm, .-btog_imm

.globl btog_mem
.p2align 4
.type btog_mem, @function
btog_mem:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    movl %esi, %ecx
    shll %cl, %eax
    xorl (%rdi), %eax
    ret
.cfi_endproc
.size btog_mem, .-btog_mem

.globl btog_twice
.p2align 4
.type btog_twice, @function
btog_twice:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl %edi, %eax
    ret
.cfi_endproc
.size btog_twice, .-btog_twice

.globl btog_swapped
.p2align 4
.type btog_swapped, @function
btog_swapped:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    movl %esi, %ecx
    shll %cl, %eax
    xorl %edi, %eax
    ret
.cfi_endproc
.size btog_swapped, .-btog_swapped

.globl btog_offset
.p2align 4
.type btog_offset, @function
btog_offset:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    addl $1, %esi
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    xorl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size btog_offset, .-btog_offset

.globl keep_xor_two
.p2align 4
.type keep_xor_two, @function
keep_xor_two:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $3, %edx
    movl %esi, %ecx
    shll %cl, %edx
    xorl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size keep_xor_two, .-keep_xor_two

.globl keep_xor_not
.p2align 4
.type keep_xor_not, @function
keep_xor_not:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    movq %rsi, %rcx
    shll %cl, %eax
    movq %rax, %rsi
    notl %esi
    xorl %esi, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size keep_xor_not, .-keep_xor_not

.globl keep_xor_hi
.p2align 4
.type keep_xor_hi, @function
keep_xor_hi:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shlq %cl, %rdx
    xorl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size keep_xor_hi, .-keep_xor_hi


.section .note.GNU-stack,"",@progbits
