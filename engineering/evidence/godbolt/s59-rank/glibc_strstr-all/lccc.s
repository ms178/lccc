.Lstr0:

haystack:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $328, %rsp
    leaq haystack(%rip), %r10
    xorl %r8d, %r8d
    movl $2084213038, %r9d
.LBB1:
    cmpq $524288, %r8
jae .LBB3
.LBB2:
    imull $1664525, %r9d, %edi
    leal 1013904223(%rdi), %r9d
    movl %r9d, %r11d
    imulq $1321528399, %r11, %r11
    shrq $35, %r11
    movl %r11d, %ebx
    imull $26, %ebx, %r11d
    movl %r9d, %r12d
    subl %r11d, %r12d
    leal 97(%r12), %r11d
    movb %r11b, (%r10, %r8)
    addq $1, %r8
    cmpq $524288, %r8
jb .LBB2
.LBB3:
    movb $0, haystack+524288(%rip)
    movl $4294967295, %eax
    movq %rax, 32(%rsp)
    movq $0, 40(%rsp)
    xorl %r11d, %r11d
    movl $523124044, %r9d
.LBB4:
    movl $0, %eax
    cmpl $8, %eax
jae .LBB32
.LBB5:
    movq %r11, %r15
    xorl %ebp, %ebp
    movq %r9, %rbx
.LBB6:
    cmpl $2048, %ebp
jae .LBB27
.LBB7:
    movl %ebp, %r12d
    andl $7, %r12d
    leal 4(%r12), %r13d
    imull $1664525, %ebx, %r9d
    leal 1013904223(%r9), %eax
    movl %eax, 28(%rsp)
    testl $1, %ebp
jne .LBB29
.LBB8:
    xorl %edx, %edx
.LBB9:
    cmpl %r13d, %edx
jae .LBB11
.LBB10:
    movl %edx, %ebx
    movl %edx, %esi
    leaq (%rsi, %rsi, 2), %rsi
    movl 28(%rsp), %r8d
    shrxl %esi, %r8d, %r8d
    movl %r8d, %esi
    imulq $1321528399, %rsi, %rsi
    shrq $35, %rsi
    movl %esi, %r14d
    imull $26, %r14d, %esi
    subl %esi, %r8d
    addl $97, %r8d
    movb %r8b, 304(%rsp, %rbx)
    addl $1, %edx
    cmpl %r13d, %edx
jb .LBB10
.LBB11:
    movzbl %r13b, %edx
    xorl %r8d, %r8d
.LBB12:
    cmpq $256, %r8
jae .LBB14
.LBB13:
    movb %dl, 48(%rsp, %r8)
    addq $1, %r8
    cmpq $256, %r8
jb .LBB13
.LBB14:
    leal 3(%r12), %edx
    xorl %r8d, %r8d
.LBB15:
    cmpl %edx, %r8d
jae .LBB17
.LBB16:
    movl %r8d, %edi
    movzbl 304(%rsp, %rdi), %eax
    leaq 48(%rsp), %rcx
    addq %rax, %rcx
    movl %edx, %edi
    subl %r8d, %edi
    movb %dil, (%rcx)
    addl $1, %r8d
    cmpl %edx, %r8d
jb .LBB16
.LBB17:
    leal -1(%r13), %eax
    movslq %eax, %r14
    movl $524288, %ebx
    subl %r13d, %ebx
    xorl %edx, %edx
.LBB18:
    cmpl %ebx, %edx
ja .LBB25
.LBB19:
    movq %r14, %rdi
.LBB20:
    testq %rdi, %rdi
jl .LBB23
.LBB21:
    movzbl 304(%rsp, %rdi), %esi
    movl %edi, %r8d
    leal (%rdx, %r8), %r9d
    movzbl (%r10, %r9), %eax
    movl %eax, %r9d
    cmpl %r9d, %esi
jne .LBB23
.LBB22:
    subq $1, %rdi
    testq %rdi, %rdi
jge .LBB21
.LBB23:
    testq %rdi, %rdi
jl .LBB28
.LBB24:
    leal (%rdx, %r13), %edi
    movzbl -1(%r10, %rdi), %r9d
    leaq 48(%rsp), %rcx
    addq %r9, %rcx
    movzbl (%rcx), %esi
    addl %esi, %edx
    cmpl %ebx, %edx
jbe .LBB19
.LBB25:
    movq $-1, %r8
.LBB26:
    movslq %r8d, %rdi
    movl %ebp, %esi
    movq %rsi, %rdx
    shlq $24, %rdx
    xorq %rdx, %rdi
    leaq (%r15, %rdi, 1), %rdx
    imulq $104729, %rsi, %rsi
    leaq (%r15, %rsi, 1), %rdi
    testl %r8d, %r8d
    movq %rdi, %r15
    cmovgeq %rdx, %r15
    addl $1, %ebp
    movl 28(%rsp), %ebx
    cmpl $2048, %ebp
jb .LBB7
.LBB27:
    movq 40(%rsp), %rax
    imull $16381, %eax, %eax
    movl %eax, %esi
    movq %rsi, %rax
    andq $524287, %rax
    movl 40(%rsp), %edi
    imulq $1321528399, %rdi, %rdi
    shrq $35, %rdi
    movl %edi, %esi
    imull $26, %esi, %esi
    movl 40(%rsp), %edi
    subl %esi, %edi
    addl $97, %edi
    movb %dil, (%r10, %rax)
    movq 40(%rsp), %rax
    addl $1, %eax
    movq %rax, 40(%rsp)
    movq %r15, %r11
    movq %rbx, %r9
    cmpl $8, %eax
jb .LBB5
    jmp .LBB32
.LBB28:
    movslq %edx, %r8
    jmp .LBB26
.LBB29:
    movl 28(%rsp), %eax
    imull $31337, %eax, %eax
    movl %eax, %edx
    andq 32(%rsp), %rdx
    imulq $262161, %rdx, %r8
    shrq $32, %r8
    movq %rdx, %r9
    subq %r8, %r9
    shrq $1, %r9
    addq %r8, %r9
    shrq $18, %r9
    imulq $524256, %r9, %r9
    subq %r9, %rdx
    movl %edx, %edi
    xorl %esi, %esi
.LBB30:
    cmpl %r13d, %esi
jae .LBB11
.LBB31:
    movl %esi, %edx
    leal (%rdi, %rsi), %r9d
    movzbl (%r10, %r9), %eax
    movb %al, 304(%rsp, %rdx)
    addl $1, %esi
    cmpl %r13d, %esi
jb .LBB31
    jmp .LBB11
.LBB32:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $328, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


