.Lstr0:

strings:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $248, %rsp
    movl $42, %r11d
    xorl %r10d, %r10d
    leaq strings(%rip), %r12
.LBB1:
    cmpq $100000, %r10
jge .LBB6
.LBB2:
    imull $1664525, %r11d, %r11d
    leal 1013904223(%r11), %r13d
    movl %r13d, %r8d
    imulq $1813430637, %r8, %r9
    shrq $32, %r9
    subq %r9, %r8
    shrq $1, %r8
    addq %r9, %r8
    shrq $7, %r8
    movl %r8d, %edi
    imull $180, %edi, %edi
    movl %r13d, %esi
    subl %edi, %esi
    leal 10(%rsi), %eax
    movl %eax, %edx
    movslq %edx, %r15
    xorl %ebp, %ebp
.LBB3:
    cmpq %r15, %rbp
jge .LBB5
.LBB4:
    leaq (%r12, %rbp), %r14
    imull $1664525, %r13d, %eax
    addl $1013904223, %eax
    movl %eax, %r13d
    movl %eax, %r8d
    imulq $1321528399, %r8, %r8
    shrq $35, %r8
    movl %r8d, %r9d
    imull $26, %r9d, %r9d
    movl %eax, %edi
    subl %r9d, %edi
    addl $97, %edi
    movzbl %dil, %esi
    movb %sil, (%r14)
    leaq 1(%rbp), %r11
    movq %r11, %rbp
    cmpq %r15, %rbp
jl .LBB4
.LBB5:
    movslq %edx, %r8
    movslq %edx, %rdx
    movb $0, (%r12, %rdx)
    addq $1, %r10
    leaq 200(%r12), %r12
    movq %r13, %r11
    cmpq $100000, %r10
jl .LBB2
.LBB6:
    xorl %ebx, %ebx
    movq $0, 24(%rsp)
.LBB7:
    cmpl $50, %ebx
jge .LBB12
.LBB8:
    xorl %r13d, %r13d
    movq 24(%rsp), %r14
    leaq strings(%rip), %r15
.LBB9:
    cmpq $100000, %r13
jge .LBB11
.LBB10:
    movq %r15, %rdi
    call strlen@PLT
    addq %rax, %r14
    addq $1, %r13
    leaq 200(%r15), %r15
    cmpq $100000, %r13
jl .LBB10
.LBB11:
    addl $1, %ebx
    movq %r14, 24(%rsp)
    cmpl $50, %ebx
jl .LBB8
.LBB12:
    leaq strings+200(%rip), %rbp
    movq $0, 16(%rsp)
    xorl %r13d, %r13d
    leaq strings(%rip), %r14
.LBB13:
    cmpq $99999, %r13
jge .LBB15
.LBB14:
    addq $1, %r13
    movq %r14, %rdi
    movq %rbp, %rsi
    call strcmp@PLT
    movq %rax, %rsi
    testl %eax, %eax
    setg %dl
    movzbl %dl, %edx
    testl %eax, %eax
    setl %r11b
    movzbl %r11b, %r11d
    movl %edx, %eax
    subl %r11d, %eax
    movslq %eax, %r10
    addq %r10, 16(%rsp)
    leaq 200(%r14), %r14
    leaq 200(%rbp), %rbp
    cmpq $99999, %r13
jl .LBB14
.LBB15:
    movb $97, 232(%rsp)
    movb $98, 233(%rsp)
    movb $99, 234(%rsp)
    movb $0, 235(%rsp)
    xorl %ebp, %ebp
    xorl %r13d, %r13d
    leaq strings(%rip), %r14
.LBB16:
    cmpq $100000, %r13
jge .LBB38
.LBB17:
    movsbl 232(%rsp), %esi
    testb %sil, %sil
jne .LBB24
.LBB18:
    movq %r14, %rdx
    jmp .LBB19
.LBB24:
    leaq 232(%rsp), %r11
.LBB25:
    movsbl (%r11), %r10d
    testb %r10b, %r10b
jne .LBB26
.LBB27:
    movq %r14, %r15
    jmp .LBB28
.LBB26:
    addq $1, %r11
    jmp .LBB25
.LBB28:
    movsbl (%r15), %r8d
    testb %r8b, %r8b
je .LBB37
.LBB29:
    leaq 232(%rsp), %r12
    movq %r15, %rbx
    jmp .LBB30
.LBB37:
    xorl %edx, %edx
    jmp .LBB19
.LBB30:
    movsbl (%rbx), %r9d
    testb %r9b, %r9b
je .LBB34
.LBB31:
    movsbl (%r12), %edi
    testb %dil, %dil
je .LBB34
.LBB32:
    movl %r9d, %esi
    cmpb %dil, %sil
jne .LBB34
.LBB33:
    addq $1, %rbx
    addq $1, %r12
    jmp .LBB30
.LBB34:
    movsbl (%r12), %edx
    testb %dl, %dl
jne .LBB35
.LBB36:
    movq %r15, %rdx
    jmp .LBB19
.LBB35:
    addq $1, %r15
    jmp .LBB28
.LBB19:
    leaq 1(%rbp), %r11
    testq %rdx, %rdx
    cmovneq %r11, %rbp
    addq $1, %r13
    leaq 200(%r14), %r14
    cmpq $100000, %r13
jl .LBB17
.LBB38:
    xorl %ebx, %ebx
    xorl %r11d, %r11d
.LBB39:
    cmpl $50, %ebx
jge .LBB40
.LBB20:
    xorl %r12d, %r12d
    movq %r11, %r13
    leaq strings(%rip), %r14
.LBB21:
    cmpq $100000, %r12
jge .LBB23
.LBB22:
    movq %r14, %rdi
    call strlen@PLT
    leal 1(%rax), %eax
    movslq %eax, %r10
    leaq 32(%rsp), %rdi
    movq %r14, %rsi
    movq %r10, %rdx
    call memcpy@PLT
    movsbq 32(%rsp), %rax
    movsbq %al, %r9
    addq %r9, %r13
    addq $1, %r12
    leaq 200(%r14), %r14
    cmpq $100000, %r12
jl .LBB22
.LBB23:
    addl $1, %ebx
    movq %r13, %r11
    cmpl $50, %ebx
jl .LBB20
.LBB40:
    leaq .Lstr0(%rip), %rdi
    movq 24(%rsp), %rsi
    movq 16(%rsp), %rdx
    movq %rbp, %rcx
    movq %r11, %r8
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $248, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


