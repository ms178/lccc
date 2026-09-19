.Lstr0:

main.buf.0:

sum_fields:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    xorl %edx, %edx
    xorl %r8d, %r8d
    xorl %ebx, %ebx
    xorl %r11d, %r11d
    xorl %r10d, %r10d
.LBB1:
    movsbl (%rdi), %r12d
    cmpb $48, %r12b
jl .LBB4
.LBB2:
    cmpb $57, %r12b
jg .LBB4
.LBB3:
    movq %r8, %r9
    leaq (%r9, %r9, 4), %r9
    addq %r9, %r9
    movsbq %r12b, %rax
    subl $48, %eax
    movslq %eax, %r14
    movq %r9, %rbp
    addq %r14, %rbp
    movq %rdx, %r15
    movl $1, %r13d
    movq %r11, %r14
    movq %r10, 24(%rsp)
    jmp .LBB13
.LBB4:
    movq %r8, %r15
    negq %r15
    testl %r10d, %r10d
    movq %r8, %rbp
    cmovneq %r15, %rbp
    leaq (%rdx, %rbp, 1), %r10
    cmpl %esi, %r11d
    movq %rdx, %r8
    cmoveq %r10, %r8
    testl %ebx, %ebx
    movq %rdx, %r9
    cmovneq %r8, %r9
    cmpb $44, %r12b
jne .LBB6
.LBB5:
    leal 1(%r11), %eax
    movl %eax, %r10d
    xorl %edx, %edx
    jmp .LBB12
.LBB6:
    cmpb $10, %r12b
je .LBB8
.LBB7:
    testb %r12b, %r12b
jne .LBB10
.LBB8:
    testb %r12b, %r12b
je .LBB14
.LBB9:
    xorl %r8d, %r8d
    xorl %ebx, %ebx
    jmp .LBB11
.LBB10:
    cmpb $45, %r12b
    movl $0, %eax
    movl $1, %ecx
    cmovel %ecx, %eax
    movq %r11, %r8
    movl %eax, %ebx
.LBB11:
    movq %r8, %r10
    movq %rbx, %rdx
.LBB12:
    movq %r9, %r15
    xorl %ebp, %ebp
    xorl %r13d, %r13d
    movq %r10, %r14
    movq %rdx, 24(%rsp)
.LBB13:
    addq $1, %rdi
    movq %r15, %rdx
    movq %rbp, %r8
    movq %r13, %rbx
    movq %r14, %r11
    movq 24(%rsp), %r10
    jmp .LBB1
.LBB14:
    movq %r9, %rax
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $104, %rsp
    leaq main.buf.0(%rip), %rdi
    xorl %esi, %esi
    movq $0, 80(%rsp)
.LBB16:
    movl $0, %eax
    cmpl $64, %eax
jge .LBB90
.LBB17:
    movl 80(%rsp), %r9d
    imull $131, %r9d, %r9d
    movslq %r9d, %rbp
    imulq $274877907, %rbp, %rbp
    sarq $38, %rbp
    movq %rbp, %r15
    shrq $63, %r15
    addq %r15, %rbp
    movslq %ebp, %r15
    movq %r15, %rbp
    imull $1000, %ebp, %ebp
    subl %ebp, %r9d
    subl $500, %r9d
    testl %r9d, %r9d
jl .LBB89
.LBB18:
    movq %rsi, 72(%rsp)
    movq %r9, 64(%rsp)
.LBB19:
    cmpl $0, 64(%rsp)
je .LBB88
.LBB20:
    movq $0, 56(%rsp)
.LBB21:
    movq 56(%rsp), %r9
    movq 64(%rsp), %rsi
.LBB22:
    testl %esi, %esi
jle .LBB24
.LBB23:
    leal 1(%r9), %r8d
    movslq %r9d, %rdx
    movslq %esi, %rbp
    imulq $1717986919, %rbp, %rbp
    movq %rbp, %r10
    sarq $34, %r10
    movq %r10, %rbp
    shrq $63, %rbp
    addq %rbp, %r10
    movslq %r10d, %r11
    movl %r11d, %r10d
    leaq (%r10, %r10, 4), %r10
    addl %r10d, %r10d
    movq %rsi, %rbp
    subl %r10d, %ebp
    leal 48(%rbp), %r10d
    movzbl %r10b, %ebp
    movq %rbp, %rax
    movb %al, 88(%rsp, %rdx)
    movq %r8, %r9
    movq %r11, %rsi
    testl %r11d, %r11d
jg .LBB23
.LBB24:
    movslq 72(%rsp), %r8
    movslq %r9d, %rsi
    movq %r8, %rdx
.LBB25:
    testq %rsi, %rsi
jle .LBB27
.LBB26:
    leaq 1(%rdx), %r13
    addq $-1, %rsi
    movsbq 88(%rsp, %rsi), %rax
    movb %al, (%rdi, %rdx)
    movq %r13, %rdx
    testq %rsi, %rsi
jg .LBB26
.LBB27:
    movslq %edx, %rsi
    leal 1(%rsi), %r11d
    movb $44, (%rdi, %rdx)
    movl 80(%rsp), %r8d
    imull $131, %r8d, %r8d
    addl $17, %r8d
    movslq %r8d, %r9
    imulq $274877907, %r9, %r9
    sarq $38, %r9
    movq %r9, %rdx
    shrq $63, %rdx
    addq %rdx, %r9
    movl %r9d, %r10d
    imull $1000, %r10d, %r10d
    subl %r10d, %r8d
    subl $500, %r8d
    testl %r8d, %r8d
jl .LBB87
.LBB28:
    movq %r11, 48(%rsp)
    movq %r8, %rbp
.LBB29:
    testl %ebp, %ebp
je .LBB86
.LBB30:
    movq $0, 40(%rsp)
.LBB31:
    movq 40(%rsp), %r11
    movq %rbp, %rdx
.LBB32:
    testl %edx, %edx
jg .LBB77
.LBB33:
    movslq 48(%rsp), %r10
    movslq %r11d, %r8
    movq %r10, %r9
.LBB34:
    testq %r8, %r8
jle .LBB36
.LBB35:
    leaq 1(%r9), %r14
    addq $-1, %r8
    movsbq 88(%rsp, %r8), %rax
    movb %al, (%rdi, %r9)
    movq %r14, %r9
    testq %r8, %r8
jg .LBB35
.LBB36:
    movslq %r9d, %r8
    leal 1(%r8), %esi
    movb $44, (%rdi, %r9)
    movl 80(%rsp), %r11d
    imull $131, %r11d, %r11d
    addl $34, %r11d
    movslq %r11d, %r10
    imulq $274877907, %r10, %r10
    sarq $38, %r10
    movq %r10, %r9
    shrq $63, %r9
    addq %r9, %r10
    movl %r10d, %edx
    imull $1000, %edx, %edx
    subl %edx, %r11d
    subl $500, %r11d
    testl %r11d, %r11d
jl .LBB85
.LBB37:
    movq %rsi, 32(%rsp)
    movq %r11, %r9
.LBB38:
    testl %r9d, %r9d
je .LBB84
.LBB39:
    xorl %esi, %esi
.LBB40:
.LBB41:
    testl %r9d, %r9d
jg .LBB76
.LBB42:
    movslq 32(%rsp), %rdx
    movslq %esi, %r11
    movq %rdx, %r10
.LBB43:
    testq %r11, %r11
jle .LBB45
.LBB44:
    leaq 1(%r10), %rbx
    addq $-1, %r11
    movsbq 88(%rsp, %r11), %rax
    movb %al, (%rdi, %r10)
    movq %rbx, %r10
    testq %r11, %r11
jg .LBB44
.LBB45:
    movslq %r10d, %r11
    leal 1(%r11), %r8d
    movb $44, (%rdi, %r10)
    movl 80(%rsp), %esi
    imull $131, %esi, %esi
    addl $51, %esi
    movslq %esi, %rdx
    imulq $274877907, %rdx, %rdx
    sarq $38, %rdx
    movq %rdx, %r10
    shrq $63, %r10
    addq %r10, %rdx
    movl %edx, %r9d
    imull $1000, %r9d, %r9d
    subl %r9d, %esi
    subl $500, %esi
    testl %esi, %esi
jl .LBB83
.LBB46:
    movq %r8, 24(%rsp)
    movq %rsi, %r15
.LBB47:
    testl %r15d, %r15d
je .LBB82
.LBB48:
    xorl %r8d, %r8d
.LBB49:
    movq %r15, %r10
.LBB50:
    testl %r10d, %r10d
jg .LBB75
.LBB51:
    movslq 24(%rsp), %r9
    movslq %r8d, %rsi
    movq %r9, %rdx
.LBB52:
    testq %rsi, %rsi
jle .LBB54
.LBB53:
    leaq 1(%rdx), %r12
    addq $-1, %rsi
    movsbq 88(%rsp, %rsi), %rax
    movb %al, (%rdi, %rdx)
    movq %r12, %rdx
    testq %rsi, %rsi
jg .LBB53
.LBB54:
    movslq %edx, %rsi
    leal 1(%rsi), %r11d
    movb $44, (%rdi, %rdx)
    movl 80(%rsp), %r8d
    imull $131, %r8d, %r8d
    addl $68, %r8d
    movslq %r8d, %r9
    imulq $274877907, %r9, %r9
    sarq $38, %r9
    movq %r9, %rdx
    shrq $63, %rdx
    addq %rdx, %r9
    movl %r9d, %r10d
    imull $1000, %r10d, %r10d
    subl %r10d, %r8d
    subl $500, %r8d
    testl %r8d, %r8d
jl .LBB81
.LBB55:
    movq %r11, 16(%rsp)
    movq %r8, %rdx
.LBB56:
    testl %edx, %edx
je .LBB80
.LBB57:
    xorl %r11d, %r11d
.LBB58:
.LBB59:
    testl %edx, %edx
jg .LBB74
.LBB60:
    movq 16(%rsp), %r9
    movq %r11, %r10
.LBB61:
    testl %r10d, %r10d
jle .LBB63
.LBB62:
    leal 1(%r9), %r8d
    movslq %r9d, %rsi
    leal -1(%r10), %r11d
    movslq %r11d, %r11
    movsbq 88(%rsp, %r11), %rax
    movb %al, (%rdi, %rsi)
    movq %r8, %r9
    movq %r11, %r10
    testl %r11d, %r11d
jg .LBB62
.LBB63:
    leal 1(%r9), %r10d
    movslq %r9d, %r8
    movb $44, (%rdi, %r8)
    movl 80(%rsp), %edx
    imull $131, %edx, %edx
    addl $85, %edx
    movslq %edx, %r11
    imulq $274877907, %r11, %r11
    sarq $38, %r11
    movq %r11, %r8
    shrq $63, %r8
    addq %r8, %r11
    movl %r11d, %esi
    imull $1000, %esi, %esi
    subl %esi, %edx
    subl $500, %edx
    testl %edx, %edx
jl .LBB79
.LBB64:
    movq %r10, %r15
    movq %rdx, %r8
.LBB65:
    testl %r8d, %r8d
je .LBB78
.LBB66:
    xorl %ebp, %ebp
.LBB67:
    movq %rbp, %r9
.LBB68:
    testl %r8d, %r8d
jg .LBB73
.LBB69:
    movq %r15, %r11
    movq %r9, %rsi
.LBB70:
    testl %esi, %esi
jg .LBB72
.LBB71:
    leal 1(%r11), %eax
    movl %eax, 12(%rsp)
    movslq %r11d, %rdx
    movb $10, (%rdi, %rdx)
    movl 80(%rsp), %eax
    addl $1, %eax
    movq %rax, 80(%rsp)
    movl 12(%rsp), %esi
    cmpl $64, %eax
jl .LBB17
    jmp .LBB90
.LBB72:
    leal 1(%r11), %r8d
    movslq %r11d, %r9
    leal -1(%rsi), %r10d
    movslq %r10d, %r10
    movsbq 88(%rsp, %r10), %rax
    movb %al, (%rdi, %r9)
    movq %r8, %r11
    movq %r10, %rsi
    testl %r10d, %r10d
jg .LBB72
    jmp .LBB71
.LBB73:
    leal 1(%r9), %esi
    movslq %r9d, %rdx
    movslq %r8d, %r11
    imulq $1717986919, %r11, %r11
    sarq $34, %r11
    movq %r11, %rbp
    shrq $63, %rbp
    addq %rbp, %r11
    movslq %r11d, %r11
    movq %r11, %rbp
    leal (%ebp, %ebp, 4), %ebp
    shll $1, %ebp
    movq %rbp, %rax
    movq %r8, %rbp
    subl %eax, %ebp
    addl $48, %ebp
    movzbl %bpl, %ebp
    movq %rbp, %rax
    movb %al, 88(%rsp, %rdx)
    movq %rsi, %r9
    movq %r11, %r8
    testl %r11d, %r11d
jg .LBB73
    jmp .LBB69
.LBB74:
    leal 1(%r11), %r8d
    movslq %r11d, %rsi
    movslq %edx, %r9
    imulq $1717986919, %r9, %r9
    sarq $34, %r9
    movq %r9, %r15
    shrq $63, %r15
    addq %r15, %r9
    movslq %r9d, %r9
    movq %r9, %rbp
    leal (%ebp, %ebp, 4), %ebp
    shll $1, %ebp
    movl %edx, %r15d
    subl %ebp, %r15d
    addl $48, %r15d
    movzbl %r15b, %ebp
    movq %rbp, %rax
    movb %al, 88(%rsp, %rsi)
    movq %r8, %r11
    movq %r9, %rdx
    testl %r9d, %r9d
jg .LBB74
    jmp .LBB60
.LBB75:
    leal 1(%r8), %r9d
    movslq %r8d, %rsi
    movslq %r10d, %rdx
    imulq $1717986919, %rdx, %rdx
    sarq $34, %rdx
    movq %rdx, %r15
    shrq $63, %r15
    addq %r15, %rdx
    movslq %edx, %rdx
    movq %rdx, %rbp
    leal (%ebp, %ebp, 4), %ebp
    shll $1, %ebp
    movl %r10d, %r15d
    subl %ebp, %r15d
    addl $48, %r15d
    movzbl %r15b, %ebp
    movq %rbp, %rax
    movb %al, 88(%rsp, %rsi)
    movq %r9, %r8
    movq %rdx, %r10
    testl %edx, %edx
jg .LBB75
    jmp .LBB51
.LBB76:
    leal 1(%rsi), %r8d
    movslq %esi, %rdx
    movslq %r9d, %r10
    imulq $1717986919, %r10, %r10
    sarq $34, %r10
    movq %r10, %rbp
    shrq $63, %rbp
    addq %rbp, %r10
    movslq %r10d, %r10
    movq %r10, %rbp
    leal (%ebp, %ebp, 4), %ebp
    shll $1, %ebp
    movl %r9d, %r15d
    subl %ebp, %r15d
    addl $48, %r15d
    movzbl %r15b, %ebp
    movq %rbp, %rax
    movb %al, 88(%rsp, %rdx)
    movq %r8, %rsi
    movq %r10, %r9
    testl %r10d, %r10d
jg .LBB76
    jmp .LBB42
.LBB77:
    leal 1(%r11), %r10d
    movslq %r11d, %r8
    movslq %edx, %r9
    imulq $1717986919, %r9, %r9
    sarq $34, %r9
    movq %r9, %rbp
    shrq $63, %rbp
    addq %rbp, %r9
    movslq %r9d, %r9
    movq %r9, %rbp
    leal (%ebp, %ebp, 4), %ebp
    shll $1, %ebp
    movl %edx, %r15d
    subl %ebp, %r15d
    addl $48, %r15d
    movzbl %r15b, %ebp
    movq %rbp, %rax
    movb %al, 88(%rsp, %r8)
    movq %r10, %r11
    movq %r9, %rdx
    testl %r9d, %r9d
jg .LBB77
    jmp .LBB33
.LBB78:
    movb $48, 88(%rsp)
    movl $1, %ebp
    jmp .LBB67
.LBB79:
    leal 2(%r9), %eax
    movl %eax, 12(%rsp)
    movslq %r9d, %r9
    movb $45, 1(%rdi, %r9)
    movq %rdx, %rax
    negl %eax
    movl 12(%rsp), %r15d
    movl %eax, %r8d
    jmp .LBB65
.LBB80:
    movb $48, 88(%rsp)
    movl $1, %r11d
    jmp .LBB58
.LBB81:
    leal 2(%rsi), %eax
    movslq %eax, %rax
    movq %rax, 16(%rsp)
    movslq %esi, %rsi
    movb $45, 1(%rdi, %rsi)
    movl %r8d, %edx
    negl %edx
    jmp .LBB56
.LBB82:
    movb $48, 88(%rsp)
    movl $1, %r8d
    jmp .LBB49
.LBB83:
    leal 2(%r11), %eax
    movslq %eax, %rax
    movq %rax, 24(%rsp)
    movslq %r11d, %r11
    movb $45, 1(%rdi, %r11)
    movl %esi, %r15d
    negl %r15d
    jmp .LBB47
.LBB84:
    movb $48, 88(%rsp)
    movl $1, %esi
    jmp .LBB40
.LBB85:
    leal 2(%r8), %eax
    movslq %eax, %rax
    movq %rax, 32(%rsp)
    movslq %r8d, %r8
    movb $45, 1(%rdi, %r8)
    movq %r11, %rax
    negl %eax
    movl %eax, %r9d
    jmp .LBB38
.LBB86:
    movb $48, 88(%rsp)
    movq $1, 40(%rsp)
    jmp .LBB31
.LBB87:
    leal 2(%rsi), %eax
    movslq %eax, %rax
    movq %rax, 48(%rsp)
    movslq %esi, %rsi
    movb $45, 1(%rdi, %rsi)
    movq %r8, %rbp
    negl %ebp
    jmp .LBB29
.LBB88:
    movb $48, 88(%rsp)
    movq $1, 56(%rsp)
    jmp .LBB21
.LBB89:
    leal 1(%rsi), %eax
    movslq %eax, %rax
    movq %rax, 72(%rsp)
    movslq %esi, %r11
    movb $45, (%rdi, %r11)
    movq %r9, %rax
    negl %eax
    movq %rax, 64(%rsp)
    jmp .LBB19
.LBB90:
    movslq %esi, %r8
    leaq main.buf.0(%rip), %rcx
    movb $0, (%rcx, %r8)
    leaq main.buf.0(%rip), %rdi
    xorl %esi, %esi
    call sum_fields
    movq %rax, %rbx
    leaq main.buf.0(%rip), %rdi
    movl $3, %esi
    call sum_fields
    movq %rax, %r12
    leaq main.buf.0(%rip), %rdi
    movl $5, %esi
    call sum_fields
    movq %rax, %r11
    leaq .Lstr0(%rip), %rdi
    movq %rbx, %rsi
    movq %r12, %rdx
    movq %rax, %rcx
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $104, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


