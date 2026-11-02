.Lstr0:

glibc_left:
glibc_right:

glibc_memcmp_bytes:
    pushq %rbx
    pushq %r12
    subq $40, %rsp
    movq %rdi, 24(%rsp)
    movq %rsi, 16(%rsp)
    movq %rdi, 24(%rsp)
    movq %rsi, 16(%rsp)
    xorl %r11d, %r11d
.LBB1:
    cmpq $8, %r11
jae .LBB4
.LBB2:
    movzbl 24(%rsp, %r11), %r9d
    movzbl 16(%rsp, %r11), %edx
    cmpl %edx, %r9d
je .LBB3
.LBB5:
    movzbl %r9b, %r8d
    movzbl %dl, %r9d
    subl %r9d, %r8d
    movl %r8d, %eax
.L__lccc_epilogue_1:
    addq $40, %rsp
    popq %r12
    popq %rbx
    ret
.LBB3:
    addq $1, %r11
    cmpq $8, %r11
jb .LBB2
.LBB4:
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1

glibc_memcmp_common_alignment:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $16, %rsp
    movq %rdi, %rbx
    movq %rsi, %r12
    movq %rdx, %r13
    movq %rdi, %r8
    movq %rsi, %r9
    movq %rdx, %rdi
.LBB7:
    cmpq $4, %rdi
jb .LBB13
.LBB8:
    movq (%r8), %rsi
    movq (%r9), %r11
    cmpq %r11, %rsi
jne .LBB22
.LBB9:
    movq 8(%r8), %rsi
    movq 8(%r9), %r10
    cmpq %r10, %rsi
jne .LBB21
.LBB10:
    movq 16(%r8), %rdx
    movq 16(%r9), %r11
    cmpq %r11, %rdx
jne .LBB20
.LBB11:
    movq 24(%r8), %rsi
    movq 24(%r9), %r10
    cmpq %r10, %rsi
jne .LBB19
.LBB12:
    addq $32, %r8
    addq $32, %r9
    subq $4, %rdi
    cmpq $4, %rdi
jae .LBB8
.LBB13:
.LBB14:
    testq %rdi, %rdi
je .LBB17
.LBB15:
    movq (%r8), %rsi
    movq (%r9), %rdx
    cmpq %rdx, %rsi
jne .LBB18
.LBB16:
    addq $8, %r8
    addq $8, %r9
    subq $1, %rdi
    testq %rdi, %rdi
jne .LBB15
.LBB17:
    xorl %eax, %eax
.L__lccc_epilogue_2:
    addq $16, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB18:
    movq (%r8), %rdi
    movq (%r9), %rsi
    call glibc_memcmp_bytes
    movl %eax, %esi
    movl %eax, %eax
    jmp .L__lccc_epilogue_2
.LBB19:
    movq %rsi, %rdi
    movq %r10, %rsi
    call glibc_memcmp_bytes
    movl %eax, %edx
    movl %eax, %eax
    jmp .L__lccc_epilogue_2
.LBB20:
    movq %rdx, %rdi
    movq %r11, %rsi
    call glibc_memcmp_bytes
    movl %eax, %r8d
    movl %eax, %eax
    jmp .L__lccc_epilogue_2
.LBB21:
    movq %rsi, %rdi
    movq %r10, %rsi
    call glibc_memcmp_bytes
    movl %eax, %r9d
    movl %eax, %eax
    jmp .L__lccc_epilogue_2
.LBB22:
    movq %rsi, %rdi
    movq %r11, %rsi
    call glibc_memcmp_bytes
    movl %eax, %edi
    movl %eax, %eax
    jmp .L__lccc_epilogue_2

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $104, %rsp
    movq $0, 64(%rsp)
    movabsq $72623859790382856, %rax
    movq %rax, 72(%rsp)
    movq $-1, 80(%rsp)
    movq $7, 88(%rsp)
    vmovdqu 64(%rsp), %ymm2
    vmovdqu %ymm2, 32(%rsp)
    leaq 64(%rsp), %rdi
    leaq 32(%rsp), %rsi
    movl $4, %edx
    call glibc_memcmp_common_alignment
    movl %eax, %r9d
    testl %r9d, %r9d
jne .LBB25
.LBB24:
    movabsq $72623859790382857, %rax
    movq %rax, 40(%rsp)
    leaq 64(%rsp), %rdi
    leaq 32(%rsp), %rsi
    movl $4, %edx
    call glibc_memcmp_common_alignment
    testl %eax, %eax
    movl $0, %ecx
    movl $1, %edx
    cmovgel %ecx, %edx
    testl %edx, %edx
jne .LBB26
.LBB25:
    movl $2, %eax
    vzeroupper
.L__lccc_epilogue_3:
    addq $104, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB26:
    leaq glibc_left(%rip), %rbx
    leaq glibc_right(%rip), %r12
    xorl %r13d, %r13d
    movabsq $-7046029254386353131, %r14
.LBB27:
    cmpq $8192, %r13
jae .LBB29
.LBB28:
    movq %r14, %r11
    shlq $7, %r11
    movq %r14, %r10
    xorq %r11, %r10
    movq %r10, %r8
    shrq $9, %r8
    xorq %r8, %r10
    movq %r10, %r9
    shlq $8, %r9
    movq %r10, %r14
    xorq %r9, %r14
    movq %r14, (%rbx, %r13, 8)
    movq %r14, (%r12, %r13, 8)
    addq $1, %r13
    cmpq $8192, %r13
jb .LBB28
.LBB29:
    xorl %r15d, %r15d
    xorl %ebp, %ebp
.LBB30:
    cmpl $4096, %r15d
jae .LBB32
.LBB31:
    movl %r15d, %r13d
    imulq $4051, %r13, %r11
    andq $8191, %r11
    imull $11, %r15d, %r10d
    andl $63, %r10d
    movl $1, %r8d
    shlxq %r10, %r8, %r8
    movq %r11, %r14
    movq (%rbx, %r14, 8), %rax
    movq %rax, 24(%rsp)
    xorq %rax, %r8
    movq %r8, (%r12, %r14, 8)
    leaq glibc_left(%rip), %rdi
    leaq glibc_right(%rip), %rsi
    movl $8192, %edx
    call glibc_memcmp_common_alignment
    addl $257, %eax
    movslq %eax, %rsi
    leaq 1(%r13), %rdx
    imulq %rdx, %rsi
    addq %rsi, %rbp
    movq 24(%rsp), %rax
    movq %rax, (%r12, %r14, 8)
    addl $1, %r15d
    cmpl $4096, %r15d
jb .LBB31
.LBB32:
    leaq .Lstr0(%rip), %rdi
    movq %rbp, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    vzeroupper
    jmp .L__lccc_epilogue_3


