.Lstr0:

main.check.0:

zlib_ng_adler_data:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $600, %rsp
    leaq main.check.0(%rip), %rdi
    testq %rdi, %rdi
je .LBB5
.LBB1:
    leaq main.check.0(%rip), %rdx
    xorl %r11d, %r11d
    movl $1, %edi
    movl $9, %esi
.LBB2:
    testq %rsi, %rsi
je .LBB4
.LBB3:
    subq $1, %rsi
    leaq 1(%rdx), %r9
    movzbl (%rdx), %r13d
    addl %r13d, %edi
    addl %edi, %r11d
    movq %r9, %rdx
    testq %rsi, %rsi
jne .LBB3
.LBB4:
    movl %edi, %esi
    movl $2147975281, %eax
    imulq %rax, %rsi
    shrq $47, %rsi
    movl %esi, %r9d
    imull $65521, %r9d, %r9d
    subl %r9d, %edi
    movl %r11d, %esi
    movl $2147975281, %eax
    imulq %rax, %rsi
    shrq $47, %rsi
    movl %esi, %r9d
    imull $65521, %r9d, %r9d
    subl %r9d, %r11d
    shll $16, %r11d
    orl %r11d, %edi
    cmpl $152961502, %edi
je .LBB6
.LBB5:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $600, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB6:
    leaq zlib_ng_adler_data(%rip), %rax
    movq %rax, 56(%rsp)
    testq %rax, %rax
    sete %al
    movzbl %al, %eax
    movl %eax, 44(%rsp)
    vmovdqu .LCVEC_0(%rip), %ymm0
    vmovdqu %ymm0, 544(%rsp)
    vpxor %ymm3, %ymm3, %ymm3
    vpxor %ymm4, %ymm4, %ymm4
    vpxor %ymm5, %ymm5, %ymm5
    vmovdqu .LCVEC_1(%rip), %ymm6
    vmovdqu .LCVEC_2(%rip), %ymm7
    xorl %r11d, %r11d
    movl $2654435769, %r9d
.LBB7:
    cmpq $2097152, %r11
jae .LBB9
.LBB8:
    imull $1103515245, %r9d, %r8d
    leal 12345(%r8), %r9d
    movl %r9d, %r14d
    shrl $16, %r14d
    leaq zlib_ng_adler_data(%rip), %rcx
    movb %r14b, (%rcx, %r11)
    addq $1, %r11
    cmpq $2097152, %r11
jb .LBB8
.LBB9:
    movq $0, 536(%rsp)
    xorl %r11d, %r11d
.LBB10:
    movl $0, %eax
    cmpl $48, %eax
jae .LBB32
.LBB11:
    movl 536(%rsp), %r8d
    addl $1, %r8d
    movl %r8d, %r9d
    shrl $16, %r9d
    movzwl %r9w, %edx
    movzwl %r8w, %r9d
    cmpb $0, 44(%rsp)
je .LBB14
.LBB12:
    movl $1, %r14d
.LBB13:
    movl %r14d, %r8d
    addl 536(%rsp), %r8d
    xorl %r8d, %r11d
    shrl $9, %r14d
    movzbl %r14b, %r9d
    movq 536(%rsp), %rax
    imull $12289, %eax, %eax
    movl %eax, %r8d
    movq %r8, %rax
    andq $2097151, %rax
    leaq zlib_ng_adler_data(%rip), %r8
    addq %rax, %r8
    movzbl (%r8), %edx
    xorl %r9d, %edx
    movb %dl, (%r8)
    movq 536(%rsp), %rax
    addl $1, %eax
    movq %rax, 536(%rsp)
    cmpl $48, %eax
jb .LBB11
    jmp .LBB32
.LBB14:
    movq %r9, %r14
    movq 56(%rsp), %rax
    movq %rax, %rbp
    movl $2097152, %r15d
    movq %rdx, %r13
.LBB15:
    cmpq $5552, %r15
jb .LBB19
.LBB16:
    subq $5552, %r15
    movq %r15, 48(%rsp)
    movq %r14, %r9
    movq %rbp, %r8
    movq %r13, %rdx
    movl $694, %edi
.LBB17:
    movzbl (%r8), %esi
    addl %r9d, %esi
    leal (%rdx, %rsi), %r15d
    movzbl 1(%r8), %r14d
    addl %r14d, %esi
    addl %esi, %r15d
    movzbl 2(%r8), %r14d
    addl %r14d, %esi
    addl %esi, %r15d
    movzbl 3(%r8), %r14d
    addl %r14d, %esi
    addl %esi, %r15d
    movzbl 4(%r8), %r14d
    addl %r14d, %esi
    addl %esi, %r15d
    movzbl 5(%r8), %r14d
    addl %r14d, %esi
    addl %esi, %r15d
    movzbl 6(%r8), %r14d
    addl %r14d, %esi
    addl %esi, %r15d
    movzbl 7(%r8), %r14d
    addl %r14d, %esi
    leal (%r15, %rsi), %edx
    leaq 8(%r8), %r10
    subl $1, %edi
    movq %rsi, %r9
    movq %r10, %r8
    testl %edi, %edi
jne .LBB17
.LBB18:
    movl %esi, %r9d
    movl $2147975281, %eax
    imulq %rax, %r9
    shrq $47, %r9
    movl %r9d, %edi
    imull $65521, %edi, %edi
    movl %esi, %eax
    subl %edi, %eax
    movl %eax, 36(%rsp)
    movl %edx, %esi
    movl $2147975281, %eax
    imulq %rax, %rsi
    shrq $47, %rsi
    movl %esi, %r8d
    imull $65521, %r8d, %r8d
    movl %edx, %r13d
    subl %r8d, %r13d
    movl 36(%rsp), %r14d
    movq %r10, %rbp
    movq 48(%rsp), %r15
    cmpq $5552, %r15
jae .LBB16
.LBB19:
    movl %r14d, %r9d
    vmovq %r9, %xmm0
    vpbroadcastq %xmm0, %ymm0
    vpand 544(%rsp), %ymm0, %ymm0
    vmovdqa %ymm0, %ymm8
    vmovdqa %ymm3, %ymm9
    vmovdqa %ymm4, %ymm10
    movq %r15, %rsi
    movq %rbp, %r14
.LBB20:
    cmpq $32, %rsi
jb .LBB22
.LBB21:
    vpaddq %ymm8, %ymm10, %ymm10
    vmovdqu (%r14), %ymm0
    vmovdqu %ymm0, 448(%rsp)
    vpmaddubsw %ymm6, %ymm0, %ymm0
    vpmaddwd %ymm7, %ymm0, %ymm0
    vpaddd %ymm0, %ymm9, %ymm9
    vpsadbw 448(%rsp), %ymm5, %ymm0
    vpaddq %ymm0, %ymm8, %ymm8
    addq $32, %r14
    subq $32, %rsi
    cmpq $32, %rsi
jae .LBB21
.LBB22:
    vmovdqa %ymm8, %ymm0
    vextracti128 $1, %ymm0, %ymm1
    vpaddq %ymm1, %ymm0, %ymm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 32(%rsp)
    vmovdqa %ymm10, %ymm0
    vextracti128 $1, %ymm0, %ymm1
    vpaddq %ymm1, %ymm0, %ymm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 24(%rsp)
    vmovdqa %ymm9, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movl 24(%rsp), %r8d
    shll $5, %r8d
    movl %eax, %r9d
    addl %r8d, %r9d
    movl 32(%rsp), %edi
    addl %r9d, %r13d
    movl %edi, %r8d
    vmovq %r8, %xmm0
    vpbroadcastq %xmm0, %ymm0
    vmovdqu %ymm0, 256(%rsp)
    vmovdqu .LCVEC_0(%rip), %ymm14
    vmovdqa %ymm14, %ymm1
    vmovdqu 256(%rsp), %ymm0
    vpand %ymm1, %ymm0, %ymm0
    vmovdqa %ymm0, %ymm15
    vpxor %ymm8, %ymm8, %ymm8
    vpxor %ymm9, %ymm9, %ymm9
    vpxor %ymm10, %ymm10, %ymm10
    vmovdqu .LCVEC_1(%rip), %ymm11
    vmovdqu .LCVEC_2(%rip), %ymm12
    movq %rsi, %r15
    movq %r14, %rdi
.LBB23:
    cmpq $32, %r15
jb .LBB25
.LBB24:
    vpaddq %ymm15, %ymm9, %ymm9
    vmovdqu (%rdi), %ymm0
    vmovdqu %ymm0, 208(%rsp)
    vpmaddubsw %ymm11, %ymm0, %ymm0
    vpmaddwd %ymm12, %ymm0, %ymm0
    vpaddd %ymm0, %ymm8, %ymm8
    vpsadbw 208(%rsp), %ymm10, %ymm0
    vpaddq %ymm0, %ymm15, %ymm15
    leaq 32(%rdi), %rax
    movq %rax, 32(%rsp)
    subq $32, %r15
    movq %rax, %rdi
    cmpq $32, %r15
jae .LBB24
.LBB25:
    vmovdqa %ymm15, %ymm0
    vextracti128 $1, %ymm0, %ymm1
    vpaddq %ymm1, %ymm0, %ymm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 32(%rsp)
    vmovdqa %ymm9, %ymm0
    vextracti128 $1, %ymm0, %ymm1
    vpaddq %ymm1, %ymm0, %ymm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 24(%rsp)
    vmovdqa %ymm8, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movl 24(%rsp), %esi
    shll $5, %esi
    movl %eax, %r8d
    addl %esi, %r8d
    movl 32(%rsp), %r12d
    addl %r8d, %r13d
    movq %r15, %rsi
    movq %rdi, %r9
.LBB26:
    cmpq $8, %rsi
jae .LBB31
.LBB27:
    movq %r12, %r8
    movq %rsi, %r15
    movq %r9, %rdi
    movq %r13, %rdx
.LBB28:
    testq %r15, %r15
je .LBB30
.LBB29:
    subq $1, %r15
    leaq 1(%rdi), %r14
    movzbl (%rdi), %r9d
    addl %r9d, %r8d
    addl %r8d, %edx
    movq %r14, %rdi
    testq %r15, %r15
jne .LBB29
.LBB30:
    movl %r8d, %edi
    movl $2147975281, %eax
    imulq %rax, %rdi
    shrq $47, %rdi
    movl %edi, %esi
    imull $65521, %esi, %esi
    subl %esi, %r8d
    movl %edx, %r9d
    movl $2147975281, %eax
    imulq %rax, %r9
    shrq $47, %r9
    movl %r9d, %edi
    imull $65521, %edi, %edi
    subl %edi, %edx
    shll $16, %edx
    orl %edx, %r8d
    movl %r8d, 36(%rsp)
    movl %r8d, %r14d
    jmp .LBB13
.LBB31:
    subq $8, %rsi
    movzbl (%r9), %r8d
    leal (%r12, %r8), %edx
    leal (%r13, %rdx), %r8d
    movzbl 1(%r9), %eax
    addl %eax, %edx
    addl %edx, %r8d
    movzbl 2(%r9), %eax
    addl %eax, %edx
    addl %edx, %r8d
    movzbl 3(%r9), %eax
    addl %eax, %edx
    addl %edx, %r8d
    movzbl 4(%r9), %eax
    addl %eax, %edx
    addl %edx, %r8d
    movzbl 5(%r9), %eax
    addl %eax, %edx
    addl %edx, %r8d
    movzbl 6(%r9), %eax
    addl %eax, %edx
    addl %edx, %r8d
    movzbl 7(%r9), %r15d
    leal (%rdx, %r15), %r12d
    leal (%r8, %r12), %r13d
    addq $8, %r9
    cmpq $8, %rsi
jae .LBB31
    jmp .LBB27
.LBB32:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    vzeroupper
    jmp .L__lccc_epilogue_1

.LCVEC_0:
.LCVEC_1:
.LCVEC_2:

