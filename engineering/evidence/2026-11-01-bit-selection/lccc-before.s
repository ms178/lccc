.text
.globl bset32
.p2align 4
.type bset32, @function
bset32:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    orl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size bset32, .-bset32

.globl bset64
.p2align 4
.type bset64, @function
bset64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shlq %cl, %rdx
    orq %rdx, %rdi
    movq %rdi, %rax
    ret
.cfi_endproc
.size bset64, .-bset64

.globl bset_imm
.p2align 4
.type bset_imm, @function
bset_imm:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    orl $128, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size bset_imm, .-bset_imm

.globl bset_masked
.p2align 4
.type bset_masked, @function
bset_masked:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    orl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size bset_masked, .-bset_masked

.globl bclr32
.p2align 4
.type bclr32, @function
bclr32:
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
    andl %esi, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size bclr32, .-bclr32

.globl bclr64
.p2align 4
.type bclr64, @function
bclr64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    movq %rsi, %rcx
    shlq %cl, %rax
    movq %rax, %rsi
    notq %rsi
    andq %rsi, %rdi
    movq %rdi, %rax
    ret
.cfi_endproc
.size bclr64, .-bclr64

.globl bclr_imm
.p2align 4
.type bclr_imm, @function
bclr_imm:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    andl $-129, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size bclr_imm, .-bclr_imm

.globl btest32
.p2align 4
.type btest32, @function
btest32:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    andl %edx, %edi
    setne %r8b
    movzbl %r8b, %eax
    ret
.cfi_endproc
.size btest32, .-btest32

.globl btest32z
.p2align 4
.type btest32z, @function
btest32z:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    andl %edx, %edi
    sete %r8b
    movzbl %r8b, %eax
    ret
.cfi_endproc
.size btest32z, .-btest32z

.globl btest64
.p2align 4
.type btest64, @function
btest64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shlq %cl, %rdx
    andq %rdx, %rdi
    setne %r8b
    movzbl %r8b, %eax
    ret
.cfi_endproc
.size btest64, .-btest64

.globl btest_imm
.p2align 4
.type btest_imm, @function
btest_imm:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    andl $128, %edi
    setne %sil
    movzbl %sil, %eax
    ret
.cfi_endproc
.size btest_imm, .-btest_imm

.globl bset_mem
.p2align 4
.type bset_mem, @function
bset_mem:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    orl (%rdi), %edx
    movl %edx, (%rdi)
    ret
.cfi_endproc
.size bset_mem, .-bset_mem

.globl bclr_mem
.p2align 4
.type bclr_mem, @function
bclr_mem:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl $1, %eax
    movq %rsi, %rcx
    shll %cl, %eax
    notl %eax
    andl (%rdi), %eax
    movl %eax, (%rdi)
    ret
.cfi_endproc
.size bclr_mem, .-bclr_mem

.globl pop32
.p2align 4
.type pop32, @function
pop32:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, %rax
    movl %edi, %ecx
    shrl $1, %eax
    andl $0x55555555, %eax
    subl %eax, %ecx
    movl %ecx, %eax
    shrl $2, %eax
    andl $0x33333333, %eax
    andl $0x33333333, %ecx
    addl %eax, %ecx
    movl %ecx, %eax
    shrl $4, %eax
    addl %ecx, %eax
    andl $0x0f0f0f0f, %eax
    imull $0x01010101, %eax, %eax
    shrl $24, %eax
    ret
.cfi_endproc
.size pop32, .-pop32

.globl pop64
.p2align 4
.type pop64, @function
pop64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, %rax
    movq %rdi, %rcx
    shrq $1, %rax
    movabsq $0x5555555555555555, %rdx
    andq %rdx, %rax
    subq %rax, %rcx
    movq %rcx, %rax
    shrq $2, %rax
    movabsq $0x3333333333333333, %rdx
    andq %rdx, %rax
    andq %rdx, %rcx
    addq %rax, %rcx
    movq %rcx, %rax
    shrq $4, %rax
    addq %rcx, %rax
    movabsq $0x0f0f0f0f0f0f0f0f, %rdx
    andq %rdx, %rax
    movabsq $0x0101010101010101, %rdx
    imulq %rdx, %rax
    shrq $56, %rax
    ret
.cfi_endproc
.size pop64, .-pop64

.globl parity32
.p2align 4
.type parity32, @function
parity32:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, %rax
    movl %edi, %ecx
    shrl $1, %eax
    andl $0x55555555, %eax
    subl %eax, %ecx
    movl %ecx, %eax
    shrl $2, %eax
    andl $0x33333333, %eax
    andl $0x33333333, %ecx
    addl %eax, %ecx
    movl %ecx, %eax
    shrl $4, %eax
    addl %ecx, %eax
    andl $0x0f0f0f0f, %eax
    imull $0x01010101, %eax, %eax
    shrl $24, %eax
    andl $1, %eax
    ret
.cfi_endproc
.size parity32, .-parity32

.globl parity64
.p2align 4
.type parity64, @function
parity64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, %rax
    movq %rdi, %rcx
    shrq $1, %rax
    movabsq $0x5555555555555555, %rdx
    andq %rdx, %rax
    subq %rax, %rcx
    movq %rcx, %rax
    shrq $2, %rax
    movabsq $0x3333333333333333, %rdx
    andq %rdx, %rax
    andq %rdx, %rcx
    addq %rax, %rcx
    movq %rcx, %rax
    shrq $4, %rax
    addq %rcx, %rax
    movabsq $0x0f0f0f0f0f0f0f0f, %rdx
    andq %rdx, %rax
    movabsq $0x0101010101010101, %rdx
    imulq %rdx, %rax
    shrq $56, %rax
    andl $1, %eax
    ret
.cfi_endproc
.size parity64, .-parity64

.globl pop_and1
.p2align 4
.type pop_and1, @function
pop_and1:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, %rax
    movl %edi, %ecx
    shrl $1, %eax
    andl $0x55555555, %eax
    subl %eax, %ecx
    movl %ecx, %eax
    shrl $2, %eax
    andl $0x33333333, %eax
    andl $0x33333333, %ecx
    addl %eax, %ecx
    movl %ecx, %eax
    shrl $4, %eax
    addl %ecx, %eax
    andl $0x0f0f0f0f, %eax
    imull $0x01010101, %eax, %eax
    shrl $24, %eax
    andl $1, %eax
    ret
.cfi_endproc
.size pop_and1, .-pop_and1

.globl keep_shl
.p2align 4
.type keep_shl, @function
keep_shl:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl %edi, %eax
    movl %esi, %ecx
    shll %cl, %eax
    orl %edi, %eax
    ret
.cfi_endproc
.size keep_shl, .-keep_shl

.globl keep_two
.p2align 4
.type keep_two, @function
keep_two:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $3, %edx
    movl %esi, %ecx
    shll %cl, %edx
    andl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size keep_two, .-keep_two

.globl keep_xor
.p2align 4
.type keep_xor, @function
keep_xor:
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
.size keep_xor, .-keep_xor

.globl keep_eq4
.p2align 4
.type keep_eq4, @function
keep_eq4:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    andl %edx, %edi
    xorl %eax, %eax
    cmpl $4, %edi
    sete %al
    ret
.cfi_endproc
.size keep_eq4, .-keep_eq4

.globl keep_and
.p2align 4
.type keep_and, @function
keep_and:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %edx
    movl %esi, %ecx
    shll %cl, %edx
    andl %edx, %edi
    movl %edi, %eax
    ret
.cfi_endproc
.size keep_and, .-keep_and


.section .note.GNU-stack,"",@progbits
