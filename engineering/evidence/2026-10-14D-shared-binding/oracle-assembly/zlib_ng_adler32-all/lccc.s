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
    subq $280, %rsp
    leaq main.check.0(%rip), %r8
    xorl %r9d, %r9d
    movl $1, %edi
    movl $9, %esi
.LBB1:
    testq %rsi, %rsi
je .LBB3
.LBB2:
    subq $1, %rsi
    movzbl (%r8), %r11d
    addl %r11d, %edi
    addl %edi, %r9d
    addq $1, %r8
    testq %rsi, %rsi
jne .LBB2
.LBB3:
    movl %edi, %r8d
    movl $2147975281, %eax
    imulq %rax, %r8
    shrq $47, %r8
    movl %r8d, %esi
    imull $65521, %esi, %esi
    subl %esi, %edi
    movl %r9d, %r11d
    movl $2147975281, %eax
    imulq %rax, %r11
    shrq $47, %r11
    movl %r11d, %r8d
    imull $65521, %r8d, %r8d
    subl %r8d, %r9d
    shll $16, %r9d
    orl %r9d, %edi
    cmpl $152961502, %edi
je .LBB5
.LBB4:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $280, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB5:
    vmovdqu .LCVEC_0(%rip), %ymm2
    vpxor %ymm3, %ymm3, %ymm3
    vpxor %ymm4, %ymm4, %ymm4
    vpxor %ymm5, %ymm5, %ymm5
    vmovdqu .LCVEC_1(%rip), %ymm6
    vmovdqu .LCVEC_2(%rip), %ymm7
    xorl %edi, %edi
    movl $2654435769, %esi
.LBB6:
    cmpq $2097152, %rdi
jae .LBB8
.LBB7:
    imull $1103515245, %esi, %r11d
    leal 12345(%r11), %esi
    movl %esi, %r13d
    shrl $16, %r13d
    leaq zlib_ng_adler_data(%rip), %rcx
    movb %r13b, (%rcx, %rdi)
    addq $1, %rdi
    cmpq $2097152, %rdi
jb .LBB7
.LBB8:
    movq $0, 264(%rsp)
    xorl %r11d, %r11d
.LBB9:
    movl $0, %eax
    cmpl $48, %eax
jae .LBB25
.LBB10:
    movl 264(%rsp), %esi
    addl $1, %esi
    movl %esi, %ebx
    shrl $16, %ebx
    movzwl %si, %r13d
    leaq zlib_ng_adler_data(%rip), %r14
    movl $2097152, %ebp
    movzwl %bx, %r12d
.LBB11:
    cmpq $5552, %rbp
jb .LBB15
.LBB12:
    movq %rbp, %r15
    subq $5552, %r15
    movq %r13, %rdi
    movq %r14, %r9
    movq %r12, %rdx
    movl $694, %ebx
.LBB13:
    movzbl (%r9), %esi
    leal (%rdi, %rsi), %r8d
    leal (%rdx, %r8), %esi
    movzbl 1(%r9), %r12d
    addl %r12d, %r8d
    addl %r8d, %esi
    movzbl 2(%r9), %ebp
    addl %ebp, %r8d
    addl %r8d, %esi
    movzbl 3(%r9), %r13d
    addl %r13d, %r8d
    addl %r8d, %esi
    movzbl 4(%r9), %r12d
    addl %r12d, %r8d
    addl %r8d, %esi
    movzbl 5(%r9), %ebp
    addl %ebp, %r8d
    addl %r8d, %esi
    movzbl 6(%r9), %r13d
    addl %r13d, %r8d
    addl %r8d, %esi
    movzbl 7(%r9), %r12d
    addl %r12d, %r8d
    leal (%rsi, %r8), %edx
    leaq 8(%r9), %r10
    subl $1, %ebx
    movq %r8, %rdi
    movq %r10, %r9
jne .LBB13
.LBB14:
    movl %r8d, %r9d
    movl $2147975281, %eax
    imulq %rax, %r9
    shrq $47, %r9
    movl %r9d, %edi
    imull $65521, %edi, %edi
    movl %r8d, %r13d
    subl %edi, %r13d
    movl %edx, %r8d
    movl $2147975281, %eax
    imulq %rax, %r8
    shrq $47, %r8
    movl %r8d, %r9d
    imull $65521, %r9d, %r9d
    movl %edx, %r12d
    subl %r9d, %r12d
    movq %r10, %r14
    movq %r15, %rbp
    cmpq $5552, %rbp
jae .LBB12
.LBB15:
    movl %r13d, %edi
    vmovq %rdi, %xmm0
    vpbroadcastq %xmm0, %ymm0
    vpand %ymm2, %ymm0, %ymm0
    vmovdqa %ymm0, %ymm8
    vmovdqa %ymm3, %ymm9
    vmovdqa %ymm4, %ymm10
    movq %rbp, %rsi
    movq %r14, %r9
.LBB16:
    cmpq $32, %rsi
jb .LBB18
.LBB17:
    vpaddq %ymm8, %ymm10, %ymm10
    vmovdqu (%r9), %ymm0
    vmovdqu %ymm0, 176(%rsp)
    vpmaddubsw %ymm6, %ymm0, %ymm0
    vpmaddwd %ymm7, %ymm0, %ymm0
    vpaddd %ymm0, %ymm9, %ymm9
    vpsadbw 176(%rsp), %ymm5, %ymm0
    vpaddq %ymm0, %ymm8, %ymm8
    leaq 32(%r9), %rax
    movq %rax, 24(%rsp)
    subq $32, %rsi
    movq %rax, %r9
    cmpq $32, %rsi
jae .LBB17
.LBB18:
    vextracti128 $1, %ymm8, %ymm1
    vpaddq %ymm1, %ymm8, %ymm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 24(%rsp)
    vextracti128 $1, %ymm10, %ymm1
    vpaddq %ymm1, %ymm10, %ymm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 16(%rsp)
    vextracti128 $1, %ymm9, %xmm1
    vpaddd %xmm1, %xmm9, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movl 16(%rsp), %edi
    shll $5, %edi
    movl %eax, %r8d
    addl %edi, %r8d
    movl 24(%rsp), %r15d
    movq %rsi, %rbp
    movq %r9, %rdi
    leal (%r8, %r12), %ebx
.LBB19:
    cmpq $8, %rbp
jae .LBB24
.LBB20:
    movq %r15, %rsi
    movq %rbp, %r9
    movq %rdi, %r8
    movq %rbx, %rdx
.LBB21:
    testq %r9, %r9
je .LBB23
.LBB22:
    leaq -1(%r9), %rax
    movq %rax, 24(%rsp)
    movzbl (%r8), %edi
    addl %edi, %esi
    addl %esi, %edx
    movq 24(%rsp), %r9
    addq $1, %r8
    testq %r9, %r9
jne .LBB22
.LBB23:
    movl %esi, %r8d
    movl $2147975281, %eax
    imulq %rax, %r8
    shrq $47, %r8
    movl %r8d, %r9d
    imull $65521, %r9d, %r9d
    subl %r9d, %esi
    movl %edx, %edi
    movl $2147975281, %eax
    imulq %rax, %rdi
    shrq $47, %rdi
    movl %edi, %r8d
    imull $65521, %r8d, %r8d
    subl %r8d, %edx
    shll $16, %edx
    orl %edx, %esi
    movl %esi, %edi
    addl 264(%rsp), %edi
    xorl %edi, %r11d
    shrl $9, %esi
    movzbl %sil, %r9d
    movl 264(%rsp), %eax
    imull $12289, %eax, %eax
    movl %eax, %edi
    movq %rdi, %rax
    andq $2097151, %rax
    leaq zlib_ng_adler_data(%rip), %r8
    addq %rax, %r8
    movzbl (%r8), %edi
    xorl %r9d, %edi
    movb %dil, (%r8)
    movl 264(%rsp), %eax
    addl $1, %eax
    movq %rax, 264(%rsp)
    cmpl $48, %eax
jb .LBB10
    jmp .LBB25
.LBB24:
    subq $8, %rbp
    movzbl (%rdi), %esi
    leal (%r15, %rsi), %r8d
    leal (%rbx, %r8), %esi
    movzbl 1(%rdi), %eax
    addl %eax, %r8d
    addl %r8d, %esi
    movzbl 2(%rdi), %eax
    addl %eax, %r8d
    addl %r8d, %esi
    movzbl 3(%rdi), %eax
    addl %eax, %r8d
    addl %r8d, %esi
    movzbl 4(%rdi), %eax
    addl %eax, %r8d
    addl %r8d, %esi
    movzbl 5(%rdi), %eax
    addl %eax, %r8d
    addl %r8d, %esi
    movzbl 6(%rdi), %eax
    addl %eax, %r8d
    addl %r8d, %esi
    movzbl 7(%rdi), %edx
    leal (%r8, %rdx), %r15d
    leal (%rsi, %r15), %ebx
    addq $8, %rdi
    cmpq $8, %rbp
jae .LBB24
    jmp .LBB20
.LBB25:
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

