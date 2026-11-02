main:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $184, %rsp
    .cfi_def_cfa_offset 240
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    leaq main.buf.0(%rip), %rdi
    movq $0, 160(%rsp)
    movq $0, 152(%rsp)
.LBB16:
    movl 152(%rsp), %eax
    cmpl $64, %eax
jge .LBB90
.LBB17:
    movl 152(%rsp), %r9d
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
    leal -500(%r9), %edx
    testl %edx, %edx
jl .LBB19
.LBB18:
    movq 160(%rsp), %rax
    movq %rax, 144(%rsp)
    movq %rdx, %r9
    jmp .LBB20
.LBB19:
    movslq 160(%rsp), %rax
    addl $1, %eax
    cltq
    movq %rax, 144(%rsp)
    movslq 160(%rsp), %r15
    movb $45, (%rdi, %r15)
    movl %edx, %r9d
    negl %r9d
.LBB20:
    testl %r9d, %r9d
je .LBB22
.LBB21:
    xorl %r15d, %r15d
    jmp .LBB23
.LBB22:
    movb $48, 168(%rsp)
    movl $1, %r15d
.LBB23:
    movq %r15, %r8
    movq %r9, %rdx
.LBB24:
    testl %edx, %edx
jle .LBB26
.p2align 4,,10
.p2align 3
.LBB25:
    leal 1(%r8), %r15d
    movslq %r8d, %r9
    movslq %edx, %rbp
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
    movq %rdx, %rbp
    subl %r10d, %ebp
    leal 48(%rbp), %r10d
    movzbl %r10b, %ebp
    movq %rbp, %rax
    movb %al, 168(%rsp, %r9)
    movq %r15, %r8
    movq %r11, %rdx
    testl %r11d, %r11d
jg .LBB25
.LBB26:
    movslq 144(%rsp), %r9
    movslq %r8d, %rsi
    movq %r9, %rdx
.LBB27:
    testq %rsi, %rsi
jle .LBB34
.p2align 4,,10
.p2align 3
.LBB28:
    leaq 1(%rdx), %r13
    subq $1, %rsi
    movsbq 168(%rsp, %rsi), %rax
    movb %al, (%rdi, %rdx)
    movq %r13, %rdx
    testq %rsi, %rsi
jg .LBB28
    jmp .LBB34
.LBB29:
    leal 1(%r9), %eax
    cltq
    movq %rax, 160(%rsp)
    movslq %r9d, %rdx
    movb $10, (%rdi, %rdx)
    movl 152(%rsp), %eax
    addl $1, %eax
    cltq
    movq %rax, 152(%rsp)
    movl %eax, %eax
    cmpl $64, %eax
jl .LBB17
    jmp .LBB90
.LBB30:
    testq %r8, %r8
jg .LBB39
    jmp .LBB46
.LBB31:
    testl %r9d, %r9d
jg .LBB40
    jmp .LBB36
.LBB32:
    cmpl $0, 120(%rsp)
je .LBB41
.LBB33:
    movq $0, 136(%rsp)
    jmp .LBB37
.LBB34:
    movslq %edx, %r11
    leal 1(%r11), %r10d
    movb $44, (%rdi, %rdx)
    movl 152(%rsp), %r9d
    imull $131, %r9d, %r9d
    addl $17, %r9d
    movslq %r9d, %rsi
    imulq $274877907, %rsi, %rsi
    sarq $38, %rsi
    movq %rsi, %rdx
    shrq $63, %rdx
    addq %rdx, %rsi
    movl %esi, %r8d
    imull $1000, %r8d, %r8d
    subl %r8d, %r9d
    subl $500, %r9d
    testl %r9d, %r9d
jl .LBB38
.LBB35:
    movq %r10, 128(%rsp)
    movq %r9, 120(%rsp)
    jmp .LBB32
.LBB36:
    movslq 128(%rsp), %r15
    movslq %esi, %r9
    movq %r15, %r10
    movq %r9, %r8
    jmp .LBB30
.LBB37:
    movq 136(%rsp), %rsi
    movq 120(%rsp), %r9
    jmp .LBB31
.LBB38:
    leal 2(%r11), %eax
    cltq
    movq %rax, 128(%rsp)
    movslq %r11d, %r11
    movb $45, 1(%rdi, %r11)
    movq %r9, %rax
    negl %eax
    movq %rax, 120(%rsp)
    jmp .LBB32
.p2align 4,,10
.p2align 3
.LBB39:
    leaq 1(%r10), %r14
    subq $1, %r8
    movsbq 168(%rsp, %r8), %rax
    movb %al, (%rdi, %r10)
    movq %r14, %r10
    testq %r8, %r8
jg .LBB39
    jmp .LBB46
.p2align 4,,10
.p2align 3
.LBB40:
    leal 1(%rsi), %edx
    movslq %esi, %r11
    movslq %r9d, %r8
    imulq $1717986919, %r8, %r8
    sarq $34, %r8
    movq %r8, %r15
    shrq $63, %r15
    addq %r15, %r8
    movslq %r8d, %rbp
    movl %ebp, %r8d
    leaq (%r8, %r8, 4), %r8
    addl %r8d, %r8d
    movl %r9d, %r15d
    subl %r8d, %r15d
    leal 48(%r15), %r8d
    movb %r8b, 168(%rsp, %r11)
    movq %rdx, %rsi
    movq %rbp, %r9
    testl %r9d, %r9d
jg .LBB40
    jmp .LBB36
.LBB41:
    movb $48, 168(%rsp)
    movq $1, 136(%rsp)
    jmp .LBB37
.LBB42:
    testq %rsi, %rsi
jg .LBB51
    jmp .LBB58
.LBB43:
    testl %edx, %edx
jg .LBB52
    jmp .LBB48
.LBB44:
    cmpl $0, 96(%rsp)
je .LBB53
.LBB45:
    movq $0, 112(%rsp)
    jmp .LBB49
.LBB46:
    movslq %r10d, %r8
    leal 1(%r8), %r9d
    movb $44, (%rdi, %r10)
    movl 152(%rsp), %edx
    imull $131, %edx, %edx
    addl $34, %edx
    movslq %edx, %r11
    imulq $274877907, %r11, %r11
    sarq $38, %r11
    movq %r11, %r10
    shrq $63, %r10
    addq %r10, %r11
    movl %r11d, %esi
    imull $1000, %esi, %esi
    subl %esi, %edx
    subl $500, %edx
    testl %edx, %edx
jl .LBB50
.LBB47:
    movq %r9, 104(%rsp)
    movq %rdx, 96(%rsp)
    jmp .LBB44
.LBB48:
    movslq 104(%rsp), %r15
    movslq %r11d, %rdx
    movq %r15, %r9
    movq %rdx, %rsi
    jmp .LBB42
.LBB49:
    movq 112(%rsp), %r11
    movq 96(%rsp), %rdx
    jmp .LBB43
.LBB50:
    leal 2(%r8), %eax
    cltq
    movq %rax, 104(%rsp)
    movslq %r8d, %r8
    movb $45, 1(%rdi, %r8)
    movq %rdx, %rax
    negl %eax
    movq %rax, 96(%rsp)
    jmp .LBB44
.p2align 4,,10
.p2align 3
.LBB51:
    leaq 1(%r9), %rbx
    subq $1, %rsi
    movsbq 168(%rsp, %rsi), %rax
    movb %al, (%rdi, %r9)
    movq %rbx, %r9
    testq %rsi, %rsi
jg .LBB51
    jmp .LBB58
.p2align 4,,10
.p2align 3
.LBB52:
    leal 1(%r11), %r10d
    movslq %r11d, %r8
    movslq %edx, %rsi
    imulq $1717986919, %rsi, %rsi
    sarq $34, %rsi
    movq %rsi, %rbp
    shrq $63, %rbp
    addq %rbp, %rsi
    movslq %esi, %rsi
    movl %esi, %r15d
    leaq (%r15, %r15, 4), %r15
    addl %r15d, %r15d
    movq %rdx, %rbp
    subl %r15d, %ebp
    addl $48, %ebp
    movsbl %bpl, %r15d
    movb %r15b, 168(%rsp, %r8)
    movq %r10, %r11
    movq %rsi, %rdx
    testl %esi, %esi
jg .LBB52
    jmp .LBB48
.LBB53:
    movb $48, 168(%rsp)
    movq $1, 112(%rsp)
    jmp .LBB49
.LBB54:
    testq %r11, %r11
jg .LBB63
    jmp .LBB71
.LBB55:
    testl %r10d, %r10d
jg .LBB64
    jmp .LBB60
.LBB56:
    cmpl $0, 72(%rsp)
je .LBB65
.LBB57:
    movq $0, 88(%rsp)
    jmp .LBB61
.LBB58:
    movslq %r9d, %rsi
    leal 1(%rsi), %edx
    movb $44, (%rdi, %r9)
    movl 152(%rsp), %r10d
    imull $131, %r10d, %r10d
    addl $51, %r10d
    movslq %r10d, %r8
    imulq $274877907, %r8, %r8
    sarq $38, %r8
    movq %r8, %r9
    shrq $63, %r9
    addq %r9, %r8
    movl %r8d, %r11d
    imull $1000, %r11d, %r11d
    subl %r11d, %r10d
    subl $500, %r10d
    testl %r10d, %r10d
jl .LBB62
.LBB59:
    movq %rdx, 80(%rsp)
    movq %r10, 72(%rsp)
    jmp .LBB56
.LBB60:
    movslq 80(%rsp), %r15
    movslq %r8d, %r10
    movq %r15, %rdx
    movq %r10, %r11
    jmp .LBB54
.LBB61:
    movq 88(%rsp), %r8
    movq 72(%rsp), %r10
    jmp .LBB55
.LBB62:
    leal 2(%rsi), %eax
    cltq
    movq %rax, 80(%rsp)
    movslq %esi, %rsi
    movb $45, 1(%rdi, %rsi)
    movq %r10, %rax
    negl %eax
    movq %rax, 72(%rsp)
    jmp .LBB56
.p2align 4,,10
.p2align 3
.LBB63:
    leaq 1(%rdx), %r12
    subq $1, %r11
    movsbq 168(%rsp, %r11), %rax
    movb %al, (%rdi, %rdx)
    movq %r12, %rdx
    testq %r11, %r11
jg .LBB63
    jmp .LBB71
.p2align 4,,10
.p2align 3
.LBB64:
    leal 1(%r8), %r9d
    movslq %r8d, %rsi
    movslq %r10d, %r11
    imulq $1717986919, %r11, %r11
    sarq $34, %r11
    movq %r11, %rbp
    shrq $63, %rbp
    addq %rbp, %r11
    movslq %r11d, %r15
    movl %r15d, %r11d
    leaq (%r11, %r11, 4), %r11
    addl %r11d, %r11d
    movq %r10, %rbp
    subl %r11d, %ebp
    leal 48(%rbp), %r11d
    movzbl %r11b, %ebp
    movq %rbp, %rax
    movb %al, 168(%rsp, %rsi)
    movq %r9, %r8
    movq %r15, %r10
    testl %r15d, %r15d
jg .LBB64
    jmp .LBB60
.LBB65:
    movb $48, 168(%rsp)
    movq $1, 88(%rsp)
    jmp .LBB61
.LBB66:
    testl %r8d, %r8d
jg .LBB75
    jmp .LBB83
.LBB67:
    testl %r9d, %r9d
jg .LBB76
.LBB68:
    movq 56(%rsp), %r10
    movq %rsi, %r8
    jmp .LBB66
.LBB69:
    cmpl $0, 48(%rsp)
je .LBB77
.LBB70:
    movq $0, 64(%rsp)
    jmp .LBB73
.LBB71:
    movslq %edx, %r11
    leal 1(%r11), %r10d
    movb $44, (%rdi, %rdx)
    movl 152(%rsp), %r9d
    imull $131, %r9d, %r9d
    addl $68, %r9d
    movslq %r9d, %rsi
    imulq $274877907, %rsi, %rsi
    sarq $38, %rsi
    movq %rsi, %rdx
    shrq $63, %rdx
    addq %rdx, %rsi
    movl %esi, %r8d
    imull $1000, %r8d, %r8d
    subl %r8d, %r9d
    subl $500, %r9d
    testl %r9d, %r9d
jl .LBB74
.LBB72:
    movq %r10, 56(%rsp)
    movq %r9, 48(%rsp)
    jmp .LBB69
.LBB73:
    movq 64(%rsp), %rsi
    movq 48(%rsp), %r9
    jmp .LBB67
.LBB74:
    leal 2(%r11), %eax
    cltq
    movq %rax, 56(%rsp)
    movslq %r11d, %r11
    movb $45, 1(%rdi, %r11)
    movq %r9, %rax
    negl %eax
    movq %rax, 48(%rsp)
    jmp .LBB69
.p2align 4,,10
.p2align 3
.LBB75:
    leal 1(%r10), %r11d
    movslq %r10d, %r9
    subl $1, %r8d
    movslq %r8d, %r8
    movsbq 168(%rsp, %r8), %rax
    movb %al, (%rdi, %r9)
    movq %r11, %r10
    testl %r8d, %r8d
jg .LBB75
    jmp .LBB83
.p2align 4,,10
.p2align 3
.LBB76:
    leal 1(%rsi), %r8d
    movslq %esi, %r11
    movslq %r9d, %rdx
    imulq $1717986919, %rdx, %rdx
    sarq $34, %rdx
    movq %rdx, %r15
    shrq $63, %r15
    addq %r15, %rdx
    movslq %edx, %rbp
    movl %ebp, %edx
    leaq (%rdx, %rdx, 4), %rdx
    addl %edx, %edx
    movl %r9d, %r15d
    subl %edx, %r15d
    leal 48(%r15), %edx
    movb %dl, 168(%rsp, %r11)
    movq %r8, %rsi
    movq %rbp, %r9
    testl %r9d, %r9d
jg .LBB76
    jmp .LBB68
.LBB77:
    movb $48, 168(%rsp)
    movq $1, 64(%rsp)
    jmp .LBB73
.LBB78:
    testl %r11d, %r11d
jg .LBB87
    jmp .LBB29
.LBB79:
    testl %r10d, %r10d
jg .LBB88
.LBB80:
    movq 32(%rsp), %r9
    movq %r8, %r11
    jmp .LBB78
.LBB81:
    cmpl $0, 24(%rsp)
je .LBB89
.LBB82:
    movq $0, 40(%rsp)
    jmp .LBB85
.LBB83:
    leal 1(%r10), %r9d
    movslq %r10d, %rsi
    movb $44, (%rdi, %rsi)
    movl 152(%rsp), %r11d
    imull $131, %r11d, %r11d
    addl $85, %r11d
    movslq %r11d, %r8
    imulq $274877907, %r8, %r8
    sarq $38, %r8
    movq %r8, %rsi
    shrq $63, %rsi
    addq %rsi, %r8
    movl %r8d, %edx
    imull $1000, %edx, %edx
    subl %edx, %r11d
    subl $500, %r11d
    testl %r11d, %r11d
jl .LBB86
.LBB84:
    movq %r9, 32(%rsp)
    movq %r11, 24(%rsp)
    jmp .LBB81
.LBB85:
    movq 40(%rsp), %r8
    movq 24(%rsp), %r10
    jmp .LBB79
.LBB86:
    leal 2(%r10), %eax
    cltq
    movq %rax, 32(%rsp)
    movslq %r10d, %r10
    movb $45, 1(%rdi, %r10)
    movq %r11, %rax
    negl %eax
    movq %rax, 24(%rsp)
    jmp .LBB81
.p2align 4,,10
.p2align 3
.LBB87:
    leal 1(%r9), %esi
    movslq %r9d, %rdx
    subl $1, %r11d
    movslq %r11d, %r11
    movsbq 168(%rsp, %r11), %rax
    movb %al, (%rdi, %rdx)
    movq %rsi, %r9
    testl %r11d, %r11d
jg .LBB87
    jmp .LBB29
.p2align 4,,10
.p2align 3
.LBB88:
    leal 1(%r8), %r11d
    movslq %r8d, %r9
    movslq %r10d, %rdx
    imulq $1717986919, %rdx, %rdx
    sarq $34, %rdx
    movq %rdx, %rbp
    shrq $63, %rbp
    addq %rbp, %rdx
    movslq %edx, %r15
    movl %r15d, %edx
    leaq (%rdx, %rdx, 4), %rdx
    addl %edx, %edx
    movq %r10, %rbp
    subl %edx, %ebp
    leal 48(%rbp), %edx
    movzbl %dl, %ebp
    movq %rbp, %rax
    movb %al, 168(%rsp, %r9)
    movq %r11, %r8
    movq %r15, %r10
    testl %r15d, %r15d
jg .LBB88
    jmp .LBB80
.LBB89:
    movb $48, 168(%rsp)
    movq $1, 40(%rsp)
    jmp .LBB85
.LBB90:
    movslq 160(%rsp), %r11
    leaq main.buf.0(%rip), %rcx
    movb $0, (%rcx, %r11)
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
    # LCCC_VA_CALL
    xorl %eax, %eax
    addq $184, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
