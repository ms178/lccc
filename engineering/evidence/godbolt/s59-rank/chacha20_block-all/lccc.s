.Lstr0:

check_known_vector.test_in.0:

chacha20_core:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $208, %rsp
    leaq 136(%rsp), %rdx
    subq %rsi, %rdx
    subq $4, %rdx
    cmpq $25, %rdx
jb .LBB21
.LBB1:
    xorl %r9d, %r9d
.LBB2:
    cmpq $64, %r9
jge .LBB4
.LBB3:
    vmovdqu (%rsi,%r9), %ymm2
    vmovdqu %ymm2, 136(%rsp,%r9)
    addq $32, %r9
    cmpq $64, %r9
jl .LBB3
.LBB4:
    shrq $2, %r9
    movslq %r9d, %r11
.LBB5:
    cmpl $16, %r11d
jge .LBB7
.LBB6:
    movslq %r11d, %r10
    movl (%rsi, %r10, 4), %eax
    movl %eax, 136(%rsp, %r10, 4)
    addl $1, %r11d
    cmpl $16, %r11d
jl .LBB6
.LBB7:
    leaq 136(%rsp), %rax
    vmovdqu (%rax), %xmm0
    vmovdqu %xmm0, 112(%rsp)
    vmovdqu 16(%rax), %xmm0
    vmovdqu %xmm0, 80(%rsp)
    vmovdqu 32(%rax), %xmm0
    vmovdqu %xmm0, 48(%rsp)
    vmovdqu 48(%rax), %xmm0
    vmovdqu %xmm0, 16(%rsp)
    movl $33619971, %eax
    vmovd %eax, %xmm0
    movl $100992007, %ecx
    vmovd %ecx, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movl $168364043, %eax
    movl $235736079, %ecx
    shlq $32, %rcx
    orq %rcx, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vmovdqa %xmm0, %xmm15
    movl $16777986, %eax
    vmovd %eax, %xmm0
    movl $84150022, %ecx
    vmovd %ecx, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movl $151522058, %eax
    movl $218894094, %ecx
    shlq $32, %rcx
    orq %rcx, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vmovdqa %xmm0, %xmm14
    vmovdqu 112(%rsp), %xmm0
    vmovdqa %xmm0, %xmm13
    vmovdqu 80(%rsp), %xmm0
    vmovdqa %xmm0, %xmm12
    vmovdqu 48(%rsp), %xmm0
    vmovdqa %xmm0, %xmm11
    vmovdqu 16(%rsp), %xmm0
    vmovdqa %xmm0, %xmm10
    xorl %r11d, %r11d
.LBB8:
    cmpl $10, %r11d
jge .LBB10
.LBB9:
    leal 1(%r11), %eax
    vpaddd %xmm12, %xmm13, %xmm9
    vpxor %xmm9, %xmm10, %xmm8
    vpshufb %xmm14, %xmm8, %xmm8
    vpaddd %xmm8, %xmm11, %xmm7
    vpxor %xmm7, %xmm12, %xmm6
    vpsrld $20, %xmm6, %xmm1
    vpslld $12, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpaddd %xmm6, %xmm9, %xmm9
    vpxor %xmm9, %xmm8, %xmm8
    vpshufb %xmm15, %xmm8, %xmm8
    vpaddd %xmm8, %xmm7, %xmm7
    vpxor %xmm7, %xmm6, %xmm6
    vpsrld $25, %xmm6, %xmm1
    vpslld $7, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpshufd $57, %xmm6, %xmm6
    vpaddd %xmm6, %xmm9, %xmm9
    vpshufd $147, %xmm8, %xmm8
    vpxor %xmm9, %xmm8, %xmm8
    vpshufb %xmm14, %xmm8, %xmm8
    vpshufd $78, %xmm7, %xmm7
    vpaddd %xmm8, %xmm7, %xmm7
    vpxor %xmm7, %xmm6, %xmm6
    vpsrld $20, %xmm6, %xmm1
    vpslld $12, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpaddd %xmm6, %xmm9, %xmm9
    vpxor %xmm9, %xmm8, %xmm8
    vpshufb %xmm15, %xmm8, %xmm8
    vpaddd %xmm8, %xmm7, %xmm7
    vpxor %xmm7, %xmm6, %xmm6
    vpsrld $25, %xmm6, %xmm1
    vpslld $7, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpshufd $147, %xmm6, %xmm6
    vpshufd $78, %xmm7, %xmm7
    vpshufd $57, %xmm8, %xmm8
    vmovdqa %xmm9, %xmm13
    vmovdqa %xmm6, %xmm12
    vmovdqa %xmm7, %xmm11
    vmovdqa %xmm8, %xmm10
    movl %eax, %r11d
    cmpl $10, %r11d
jl .LBB9
.LBB10:
    leaq 136(%rsp), %rax
    vmovdqu %xmm13, (%rax)
    vmovdqu %xmm12, 16(%rax)
    vmovdqu %xmm11, 32(%rax)
    vmovdqu %xmm10, 48(%rax)
    movq %rdi, %rbx
    leaq 136(%rsp), %r10
    movq %rdi, %rdx
    subq %r10, %rdx
    subq $4, %rdx
    cmpq $25, %rdx
jae .LBB15
.LBB11:
    xorl %r8d, %r8d
.LBB12:
    cmpl $16, %r8d
jge .LBB14
.LBB13:
    movslq %r8d, %r9
    movl 136(%rsp, %r9, 4), %r12d
    movl (%rsi, %r9, 4), %r13d
    addl %r12d, %r13d
    movl %r13d, (%rdi, %r9, 4)
    addl $1, %r8d
    cmpl $16, %r8d
jl .LBB13
.LBB14:
    vzeroupper
    addq $208, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB15:
    subq %rsi, %rbx
    subq $4, %rbx
    cmpq $25, %rbx
jb .LBB20
.LBB16:
    xorl %r11d, %r11d
.LBB17:
    cmpq $64, %r11
jge .LBB19
.LBB18:
    vmovdqu 136(%rsp,%r11), %ymm4
    vpaddd (%rsi,%r11), %ymm4, %ymm5
    vmovdqu %ymm5, (%rdi,%r11)
    addq $32, %r11
    cmpq $64, %r11
jl .LBB18
.LBB19:
    shrq $2, %r11
    movslq %r11d, %r8
    jmp .LBB12
.LBB20:
    xorl %r8d, %r8d
    jmp .LBB12
.LBB21:
    xorl %r11d, %r11d
    jmp .LBB5

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $232, %rsp
    leaq 32(%rsp), %rdi
    leaq check_known_vector.test_in.0(%rip), %rsi
    call chacha20_core
    movl 32(%rsp), %r11d
    cmpl $3840405776, %r11d
jne .LBB26
.LBB23:
    movl 36(%rsp), %r8d
    cmpl $358169553, %r8d
jne .LBB26
.LBB24:
    movl 88(%rsp), %edi
    cmpl $3900952779, %edi
jne .LBB26
.LBB25:
    movl 92(%rsp), %edx
    cmpl $1312575650, %edx
    movl $0, %ecx
    movl $1, %r11d
    cmovnel %ecx, %r11d
    testl %r11d, %r11d
jne .LBB28
    jmp .LBB27
.LBB26:
.LBB27:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $232, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB28:
    movl $1634760805, 160(%rsp)
    movl $857760878, 164(%rsp)
    movl $2036477234, 168(%rsp)
    movl $1797285236, 172(%rsp)
    movl $67372036, 176(%rsp)
    movl $84215045, 180(%rsp)
    movl $101058054, 184(%rsp)
    movl $117901063, 188(%rsp)
    movl $134744072, 192(%rsp)
    movl $151587081, 196(%rsp)
    movl $168430090, 200(%rsp)
    movl $185273099, 204(%rsp)
    movl $0, 208(%rsp)
    movl $3735928559, 212(%rsp)
    movl $19088743, 216(%rsp)
    movl $2309737967, 220(%rsp)
    xorl %r11d, %r11d
    xorl %r15d, %r15d
.LBB29:
    cmpl $16, %r15d
jae .LBB34
.LBB30:
    movq %r11, %rbp
    xorl %r14d, %r14d
.LBB31:
    cmpl $131072, %r14d
jae .LBB33
.LBB32:
    movl %r14d, %eax
    xorl %r15d, %eax
    movl %eax, 208(%rsp)
    leaq 96(%rsp), %rdi
    leaq 160(%rsp), %rsi
    call chacha20_core
    movl 96(%rsp), %r13d
    movl %r13d, %r10d
    shlq $32, %r10
    movl 156(%rsp), %eax
    xorq %rax, %r10
    movl 124(%rsp), %r9d
    shlq $16, %r9
    xorq %r9, %r10
    addq %r10, %rbp
    movzbl %r13b, %edi
    movl 176(%rsp), %eax
    xorl %edi, %eax
    movl %eax, 176(%rsp)
    addl $1, %r14d
    cmpl $131072, %r14d
jb .LBB32
.LBB33:
    addl $1, %r15d
    movq %rbp, %r11
    cmpl $16, %r15d
jb .LBB30
.LBB34:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1


