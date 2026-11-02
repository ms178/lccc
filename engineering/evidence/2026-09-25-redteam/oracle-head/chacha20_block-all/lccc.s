.Lstr0:

check_known_vector.test_in.0:

chacha20_core:
    pushq %rbx
    pushq %r12
    subq $88, %rsp
    xorl %edx, %edx
.LBB1:
    cmpq $64, %rdx
jge .LBB3
.LBB2:
    vmovdqu (%rsi,%rdx), %ymm2
    vmovdqu %ymm2, 16(%rsp, %rdx)
    addq $32, %rdx
    cmpq $64, %rdx
jl .LBB2
.LBB3:
    shrq $2, %rdx
    movslq %edx, %r9
    leaq 0(,%r9,4), %r11
    leaq 16(%rsp), %rcx
    addq %r11, %rcx
    movq %rcx, %r10
    leaq (%rsi, %r11), %rdx
.LBB4:
    cmpq $16, %r9
jge .LBB6
.LBB5:
    movl (%rdx), %eax
    movl %eax, (%r10)
    addq $1, %r9
    leaq 4(%r10), %r10
    leaq 4(%rdx), %rdx
    cmpq $16, %r9
jl .LBB5
.LBB6:
    leaq 16(%rsp), %rax
    vmovdqu (%rax), %xmm15
    vmovdqu 16(%rax), %xmm14
    vmovdqu 32(%rax), %xmm13
    vmovdqu 48(%rax), %xmm12
    vmovdqa .LCVEC_0(%rip), %xmm11
    vmovdqa .LCVEC_1(%rip), %xmm10
    xorl %r8d, %r8d
.LBB7:
    cmpl $10, %r8d
jge .LBB9
.LBB8:
    addl $1, %r8d
    vpaddd %xmm14, %xmm15, %xmm9
    vpxor %xmm9, %xmm12, %xmm8
    vpshufb %xmm10, %xmm8, %xmm8
    vpaddd %xmm8, %xmm13, %xmm7
    vpxor %xmm7, %xmm14, %xmm6
    vpsrld $20, %xmm6, %xmm1
    vpslld $12, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpaddd %xmm6, %xmm9, %xmm9
    vpxor %xmm9, %xmm8, %xmm8
    vpshufb %xmm11, %xmm8, %xmm8
    vpaddd %xmm8, %xmm7, %xmm7
    vpxor %xmm7, %xmm6, %xmm6
    vpsrld $25, %xmm6, %xmm1
    vpslld $7, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpshufd $57, %xmm6, %xmm6
    vpaddd %xmm6, %xmm9, %xmm9
    vpshufd $147, %xmm8, %xmm8
    vpxor %xmm9, %xmm8, %xmm8
    vpshufb %xmm10, %xmm8, %xmm8
    vpshufd $78, %xmm7, %xmm7
    vpaddd %xmm8, %xmm7, %xmm7
    vpxor %xmm7, %xmm6, %xmm6
    vpsrld $20, %xmm6, %xmm1
    vpslld $12, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpaddd %xmm6, %xmm9, %xmm9
    vpxor %xmm9, %xmm8, %xmm8
    vpshufb %xmm11, %xmm8, %xmm8
    vpaddd %xmm8, %xmm7, %xmm7
    vpxor %xmm7, %xmm6, %xmm6
    vpsrld $25, %xmm6, %xmm1
    vpslld $7, %xmm6, %xmm6
    vpor %xmm1, %xmm6, %xmm6
    vpshufd $147, %xmm6, %xmm6
    vpshufd $78, %xmm7, %xmm7
    vpshufd $57, %xmm8, %xmm8
    vmovdqa %xmm9, %xmm15
    vmovdqa %xmm6, %xmm14
    vmovdqa %xmm7, %xmm13
    vmovdqa %xmm8, %xmm12
    cmpl $10, %r8d
jl .LBB8
.LBB9:
    leaq 16(%rsp), %rax
    vmovdqu %xmm15, (%rax)
    vmovdqu %xmm14, 16(%rax)
    vmovdqu %xmm13, 32(%rax)
    vmovdqu %xmm12, 48(%rax)
    movq %rdi, %r9
    subq %rsi, %r9
    subq $4, %r9
    cmpq $25, %r9
jb .LBB17
.LBB10:
    xorl %r10d, %r10d
.LBB11:
    cmpq $64, %r10
jge .LBB13
.LBB12:
    vmovdqu 16(%rsp, %r10), %ymm4
    vpaddd (%rsi,%r10), %ymm4, %ymm5
    vmovdqu %ymm5, (%rdi,%r10)
    addq $32, %r10
    cmpq $64, %r10
jl .LBB12
.LBB13:
    shrq $2, %r10
    movslq %r10d, %rdx
.LBB14:
    cmpl $16, %edx
jge .LBB16
.LBB15:
    movslq %edx, %r8
    movl 16(%rsp, %r8, 4), %ebx
    movl (%rsi, %r8, 4), %r12d
    addl %ebx, %r12d
    movl %r12d, (%rdi, %r8, 4)
    addl $1, %edx
    cmpl $16, %edx
jl .LBB15
.LBB16:
    vzeroupper
    addq $88, %rsp
    popq %r12
    popq %rbx
    ret
.LBB17:
    xorl %edx, %edx
    jmp .LBB14

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
jne .LBB22
.LBB19:
    movl 36(%rsp), %r8d
    cmpl $358169553, %r8d
jne .LBB22
.LBB20:
    movl 88(%rsp), %edi
    cmpl $3900952779, %edi
jne .LBB22
.LBB21:
    movl 92(%rsp), %edx
    cmpl $1312575650, %edx
    movl $0, %ecx
    movl $1, %r11d
    cmovnel %ecx, %r11d
    testl %r11d, %r11d
jne .LBB24
    jmp .LBB23
.LBB22:
.LBB23:
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
.LBB24:
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
.LBB25:
    cmpl $16, %r15d
jae .LBB30
.LBB26:
    movq %r11, %rbp
    xorl %r14d, %r14d
.LBB27:
    cmpl $131072, %r14d
jae .LBB29
.LBB28:
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
jb .LBB28
.LBB29:
    addl $1, %r15d
    movq %rbp, %r11
    cmpl $16, %r15d
jb .LBB26
.LBB30:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1

.LCVEC_0:
.LCVEC_1:
