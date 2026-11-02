sha256_transform:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $280, %rsp
    .cfi_def_cfa_offset 336
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movq %rdi, %rbx
    xorl %edi, %edi
.LBB1:
    cmpq $64, %rdi
jge .LBB3
.p2align 5,,15
.p2align 4
.LBB2:
    vmovdqu (%rsi,%rdi), %ymm2
    vmovdqu %ymm2, 16(%rsp, %rdi)
    addq $32, %rdi
    cmpq $64, %rdi
jl .LBB2
.LBB3:
    shrq $2, %rdi
    movslq %edi, %r8
    leaq 0(,%r8,4), %r9
    leaq 16(%rsp), %rcx
    addq %r9, %rcx
    movq %rcx, %r11
    leaq (%rsi, %r9), %r10
.LBB4:
    cmpq $16, %r8
jge .LBB6
.p2align 4,,10
.p2align 3
.LBB5:
    movl (%r10), %eax
    movl %eax, (%r11)
    addq $1, %r8
    leaq 4(%r11), %r11
    leaq 4(%r10), %r10
    cmpq $16, %r8
jl .LBB5
.LBB6:
    movl $16, %edi
.LBB7:
    cmpl $64, %edi
jge .LBB9
.p2align 5,,15
.p2align 4
.LBB8:
    movslq %edi, %rsi
    shlq $2, %rsi
    addl $2, %edi
    leaq 16(%rsp), %rcx
    addq %rsi, %rcx
    leaq 16(%rsp), %rax
    vmovq -8(%rax,%rsi), %xmm3
    vpslld $15, %xmm3, %xmm15
    vpsrld $17, %xmm3, %xmm14
    vpor %xmm14, %xmm15, %xmm15
    vpslld $13, %xmm3, %xmm14
    vpsrld $19, %xmm3, %xmm13
    vpor %xmm13, %xmm14, %xmm14
    vpxor %xmm14, %xmm15, %xmm15
    vpsrld $10, %xmm3, %xmm14
    vpxor %xmm14, %xmm15, %xmm15
    vmovq -28(%rax,%rsi), %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq -60(%rax,%rsi), %xmm4
    vpslld $25, %xmm4, %xmm14
    vpsrld $7, %xmm4, %xmm13
    vpor %xmm13, %xmm14, %xmm14
    vpslld $14, %xmm4, %xmm13
    vpsrld $18, %xmm4, %xmm12
    vpor %xmm12, %xmm13, %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    vpsrld $3, %xmm4, %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq -64(%rax,%rsi), %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq %xmm15, (%rax,%rsi)
    cmpl $64, %edi
jl .LBB8
.LBB9:
    movl (%rbx), %r12d
    movl 4(%rbx), %r11d
    movl 8(%rbx), %edi
    movl 12(%rbx), %r13d
    movl 16(%rbx), %r14d
    movl 20(%rbx), %r9d
    movl 24(%rbx), %r10d
    movl 28(%rbx), %ebp
    xorl %r8d, %r8d
    movq %rbp, %rdx
.LBB10:
    cmpq $64, %r8
jge .LBB12
    leaq K(%rip), %rcx
.p2align 4,,10
.p2align 3
.LBB11:
    rorxl $6, %r14d, %esi
    rorxl $11, %r14d, %r15d
    xorl %r15d, %esi
    rorxl $25, %r14d, %ebp
    xorl %ebp, %esi
    leal (%rdx, %rsi), %r15d
    movl %r14d, %esi
    andl %r9d, %esi
    andnl %r10d, %r14d, %ebp
    xorl %ebp, %esi
    addl %esi, %r15d
    addl (%rcx, %r8, 4), %r15d
    addl 16(%rsp, %r8, 4), %r15d
    rorxl $2, %r12d, %esi
    rorxl $13, %r12d, %ebp
    xorl %ebp, %esi
    rorxl $22, %r12d, %ebp
    xorl %ebp, %esi
    movq %r12, %rbp
    andl %r11d, %ebp
    movl %r12d, %eax
    xorl %r11d, %eax
    andl %edi, %eax
    xorl %eax, %ebp
    addl %ebp, %esi
    movq %r13, %rbp
    addl %r15d, %ebp
    addl %esi, %r15d
    addq $1, %r8
    movq %r10, %rdx
    movq %r9, %r10
    movq %rdi, %r13
    movq %r14, %r9
    movq %r11, %rdi
    movq %rbp, %r14
    movq %r12, %r11
    movq %r15, %r12
    cmpq $64, %r8
jl .LBB11
.LBB12:
    vmovdqu (%rbx), %xmm15
    vmovd %r12d, %xmm0
    vmovd %r11d, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %rdi, %rax
    shlq $32, %r13
    orq %r13, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vpaddd %xmm0, %xmm15, %xmm15
    vmovdqu %xmm15, (%rbx)
    leaq 16(%rbx), %r8
    vmovdqu (%r8), %xmm15
    vmovd %r14d, %xmm0
    vmovd %r9d, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %r10, %rax
    shlq $32, %rdx
    orq %rdx, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vpaddd %xmm0, %xmm15, %xmm15
    vmovdqu %xmm15, (%r8)
    vzeroupper
    addq $280, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
