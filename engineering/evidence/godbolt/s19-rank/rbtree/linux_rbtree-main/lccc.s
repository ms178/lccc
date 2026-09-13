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
    movq $0, 168(%rsp)
    movq $0, 160(%rsp)
    movq $1294694901, 152(%rsp)
    leaq node_pool(%rip), %rax
    movq %rax, 144(%rsp)
    movq %rax, 136(%rsp)
    movq %rax, 128(%rsp)
.LBB1:
    movq 160(%rsp), %rax
    cmpq $16384, %rax
jb .LBB3
.LBB2:
    xorl %esi, %esi
    xorl %ebx, %ebx
    jmp .LBB42
.p2align 4,,10
.p2align 3
.LBB3:
    movq 152(%rsp), %rdx
    imull $1664525, %edx, %edx
    leal 1013904223(%rdx), %eax
    movq %rax, 152(%rsp)
    movq 136(%rsp), %rsi
    leaq 24(%rsi), %rsi
    movl 152(%rsp), %eax
    andq $2147483647, %rax
    movl %eax, (%rsi)
    movq 136(%rsp), %rax
    leaq 28(%rax), %rax
    movl 160(%rsp), %ebp
    movl %ebp, (%rax)
    movq 136(%rsp), %rax
    leaq 16(%rax), %rax
    movq $0, (%rax)
    movq 136(%rsp), %rax
    leaq 8(%rax), %rax
    movq $0, (%rax)
    movq 128(%rsp), %rax
    leaq 24(%rax), %rax
    movq %rax, %rsi
    leaq 168(%rsp), %r11
    xorl %ebp, %ebp
    jmp .LBB4
.p2align 4,,10
.p2align 3
.LBB4:
    movq (%r11), %rdx
    testq %rdx, %rdx
je .LBB6
.LBB5:
    movl (%rsi), %edi
    leaq 24(%rdx), %r10
    leaq 16(%rdx), %r8
    leaq 8(%rdx), %r15
    cmpl (%r10), %edi
    movq %r15, %r11
    cmovlq %r8, %r11
    movq %rdx, %rbp
    jmp .LBB4
.LBB6:
    movq 144(%rsp), %rcx
    movq %rbp, (%rcx)
    movq 144(%rsp), %rax
    movq %rax, (%r11)
    movq 144(%rsp), %rcx
    movq (%rcx), %r8
    andq $-4, %r8
    movq %rcx, %r9
    movq %r8, %rdi
    jmp .LBB7
.p2align 4,,10
.p2align 3
.LBB7:
    testq %rdi, %rdi
jne .LBB9
.LBB8:
    movq (%r9), %rax
    orq $1, %rax
    movq %rax, (%r9)
    jmp .LBB41
.LBB9:
    movq (%rdi), %rdx
    movq %rdx, %r11
    andq $1, %r11
jne .LBB41
.LBB10:
    andq $-4, %rdx
    movq 8(%rdx), %rsi
    cmpq %rsi, %rdi
je .LBB26
.LBB11:
    testq %rsi, %rsi
je .LBB14
.LBB12:
    movq (%rsi), %r11
    movq %r11, %r10
    andq $1, %r10
jne .LBB14
.LBB13:
    orq $1, %r11
    movq %r11, (%rsi)
    movq (%rdi), %rax
    orq $1, %rax
    movq %rax, (%rdi)
    movq (%rdx), %r11
    andq $-2, %r11
    movq %r11, (%rdx)
    movq %r11, %rbx
    andq $-4, %rbx
    movq %rdx, %r9
    movq %rbx, %rdi
    jmp .LBB7
.LBB14:
    movq 8(%rdi), %rsi
    cmpq %rsi, %r9
je .LBB16
.LBB15:
    movq %rdi, %r11
    jmp .LBB19
.LBB16:
    leaq 16(%r9), %r10
    movq (%r10), %rsi
    movq %rsi, 8(%rdi)
    movq %rdi, (%r10)
    testq %rsi, %rsi
je .LBB18
.LBB17:
    movq %rdi, %rax
    orq $1, %rax
    movq %rax, (%rsi)
.LBB18:
    movq %r9, (%rdi)
    movq 8(%r9), %rax
    movq %r9, %r11
    movq %rax, %rsi
.LBB19:
    movq %rsi, 16(%rdx)
    leaq 8(%r11), %r10
    movq %rdx, (%r10)
    testq %rsi, %rsi
je .LBB21
.LBB20:
    movq %rdx, %rax
    orq $1, %rax
    movq %rax, (%rsi)
.LBB21:
    movq (%rdx), %r8
    movq %r8, %r9
    andq $-4, %r9
    movq %r8, (%r11)
    movq %r11, (%rdx)
je .LBB25
.LBB22:
    movq 16(%r9), %rsi
    cmpq %rdx, %rsi
jne .LBB24
.LBB23:
    movq %r11, 16(%r9)
    jmp .LBB41
.LBB24:
    movq %r11, 8(%r9)
    jmp .LBB41
.LBB25:
    movq %r11, 168(%rsp)
    jmp .LBB41
.LBB26:
    movq 16(%rdx), %r11
    testq %r11, %r11
je .LBB29
.LBB27:
    movq (%r11), %r10
    testb $1, %r10b
jne .LBB29
.LBB28:
    orq $1, %r10
    movq %r10, (%r11)
    movq (%rdi), %r11
    movq %r11, %rax
    orq $1, %rax
    movq %rax, (%rdi)
    movq (%rdx), %r10
    andq $-2, %r10
    movq %r10, (%rdx)
    movq %r10, %rax
    andq $-4, %rax
    movq %rdx, %r9
    movq %rax, %rdi
    jmp .LBB7
.LBB29:
    movq 16(%rdi), %r11
    cmpq %r11, %r9
je .LBB31
.LBB30:
    movq %rdi, %r10
    jmp .LBB34
.LBB31:
    leaq 8(%r9), %rsi
    movq (%rsi), %r11
    movq %r11, 16(%rdi)
    movq %rdi, (%rsi)
    testq %r11, %r11
je .LBB33
.LBB32:
    movq %rdi, %rax
    orq $1, %rax
    movq %rax, (%r11)
.LBB33:
    movq %r9, (%rdi)
    movq 16(%r9), %r11
    movq %r9, %r10
.LBB34:
    movq %r11, 8(%rdx)
    movq %rdx, 16(%r10)
    testq %r11, %r11
je .LBB36
.LBB35:
    movq %rdx, %rax
    orq $1, %rax
    movq %rax, (%r11)
.LBB36:
    movq (%rdx), %r11
    movq %r11, %r8
    andq $-4, %r8
    movq %r11, (%r10)
    movq %r10, (%rdx)
je .LBB40
.LBB37:
    movq 16(%r8), %rdi
    cmpq %rdx, %rdi
jne .LBB39
.LBB38:
    movq %r10, 16(%r8)
    jmp .LBB41
.LBB39:
    movq %r10, 8(%r8)
    jmp .LBB41
.LBB40:
    movq %r10, 168(%rsp)
.LBB41:
    movq 160(%rsp), %rax
    addq $1, %rax
    movq %rax, 160(%rsp)
    movq 144(%rsp), %rax
    leaq 32(%rax), %rax
    movq %rax, 144(%rsp)
    movq 136(%rsp), %rax
    leaq 32(%rax), %rax
    movq %rax, 136(%rsp)
    movq 128(%rsp), %rax
    leaq 32(%rax), %rax
    movq %rax, 128(%rsp)
    movq 160(%rsp), %rax
    cmpq $16384, %rax
jb .LBB3
    jmp .LBB2
.p2align 4,,10
.p2align 3
.LBB42:
    cmpl $16384, %esi
jae .LBB56
.LBB43:
    imull $104729, %esi, %r10d
    movq %r10, %rax
    andq $16383, %rax
    movl %eax, %r8d
    shlq $5, %r8
    leaq node_pool(%rip), %r9
    addq %r8, %r9
    movslq 24(%r9), %rdx
    movq 168(%rsp), %r11
    movq %r11, %r10
.LBB44:
    testq %r10, %r10
jne .LBB46
.LBB45:
    xorl %r8d, %r8d
    jmp .LBB52
.p2align 4,,10
.p2align 3
.LBB46:
    movslq 24(%r10), %rdi
    cmpl %edi, %edx
jge .LBB48
.LBB47:
    movq 16(%r10), %r12
    jmp .LBB51
.LBB48:
    cmpl %edi, %edx
jg .LBB50
.LBB49:
    movq %r10, %r8
    jmp .LBB52
.LBB50:
    movq 8(%r10), %rax
    movq %rax, %r12
.LBB51:
    movq %r12, %r10
    testq %r12, %r12
jne .LBB46
    jmp .LBB45
.LBB52:
    testq %r8, %r8
jne .LBB54
.LBB53:
    movq %rbx, %rdi
    jmp .LBB55
.LBB54:
    movslq 28(%r8), %rax
    movslq %eax, %r9
    movslq 24(%r8), %rax
    movslq %eax, %r11
    shlq $16, %r11
    xorq %r11, %r9
    leaq (%rbx, %r9, 1), %rdi
.LBB55:
    addl $1, %esi
    movq %rdi, %rbx
    jmp .LBB42
.LBB56:
    xorl %r10d, %r10d
    movq %rbx, %r8
    jmp .LBB65
.LBB57:
    leaq .Lstr0(%rip), %rdi
    movq %rbx, %rsi
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
.LBB58:
    testq %rdi, %rdi
jne .LBB66
.LBB59:
    movq %r8, 112(%rsp)
    jmp .LBB62
.LBB60:
    movq 16(%r9), %r13
    jmp .LBB67
.LBB61:
    imull $104729, %r10d, %r11d
    movq %r11, %rax
    andq $16383, %rax
    movl %eax, %r9d
    shlq $5, %r9
    leaq node_pool(%rip), %rdi
    addq %r9, %rdi
    movslq 24(%rdi), %rsi
    movq 168(%rsp), %r9
    jmp .LBB70
.LBB62:
    addl $1, %r10d
    movq 112(%rsp), %r8
    jmp .LBB65
.LBB63:
    movq 8(%r9), %rax
    movq %rax, %r13
    jmp .LBB67
.p2align 4,,10
.p2align 3
.LBB64:
    movslq 24(%r9), %rdi
    cmpl %edi, %esi
jl .LBB60
    jmp .LBB68
.p2align 4,,10
.p2align 3
.LBB65:
    cmpl $16384, %r10d
jb .LBB61
    jmp .LBB72
.LBB66:
    movslq 28(%rdi), %rax
    movslq %eax, %r11
    movslq 24(%rdi), %rax
    movslq %eax, %rdx
    shlq $16, %rdx
    xorq %rdx, %r11
    addq %r11, %r8
    movq %r8, 112(%rsp)
    jmp .LBB62
.LBB67:
    movq %r13, %r9
    testq %r13, %r13
jne .LBB64
    jmp .LBB71
.LBB68:
    cmpl %edi, %esi
jg .LBB63
.LBB69:
    movq %r9, %rdi
    jmp .LBB58
.LBB70:
    testq %r9, %r9
jne .LBB64
.LBB71:
    xorl %edi, %edi
    jmp .LBB58
.LBB72:
    xorl %r11d, %r11d
    jmp .LBB80
.LBB73:
    testq %r9, %r9
jne .LBB81
.LBB74:
    movq %r8, 104(%rsp)
    jmp .LBB77
.LBB75:
    movq 16(%r10), %r14
    jmp .LBB82
.LBB76:
    imull $104729, %r11d, %edx
    movq %rdx, %rax
    andq $16383, %rax
    movl %eax, %r10d
    shlq $5, %r10
    leaq node_pool(%rip), %r9
    addq %r10, %r9
    movslq 24(%r9), %rdi
    movq 168(%rsp), %r10
    jmp .LBB85
.LBB77:
    addl $1, %r11d
    movq 104(%rsp), %r8
    jmp .LBB80
.LBB78:
    movq 8(%r10), %rax
    movq %rax, %r14
    jmp .LBB82
.p2align 4,,10
.p2align 3
.LBB79:
    movslq 24(%r10), %r9
    cmpl %r9d, %edi
jl .LBB75
    jmp .LBB83
.p2align 4,,10
.p2align 3
.LBB80:
    cmpl $16384, %r11d
jb .LBB76
    jmp .LBB87
.LBB81:
    movslq 28(%r9), %rax
    movslq %eax, %rdx
    movslq 24(%r9), %rax
    movslq %eax, %rsi
    shlq $16, %rsi
    xorq %rsi, %rdx
    movq %r8, %rax
    movq %rdx, %rcx
    addq %rdx, %rax
    movq %rax, 104(%rsp)
    jmp .LBB77
.LBB82:
    movq %r14, %r10
    testq %r14, %r14
jne .LBB79
    jmp .LBB86
.LBB83:
    cmpl %r9d, %edi
jg .LBB78
.LBB84:
    movq %r10, %r9
    jmp .LBB73
.LBB85:
    testq %r10, %r10
jne .LBB79
.LBB86:
    xorl %r9d, %r9d
    jmp .LBB73
.LBB87:
    xorl %edx, %edx
    jmp .LBB95
.LBB88:
    testq %r10, %r10
jne .LBB96
.LBB89:
    movq %r8, 96(%rsp)
    jmp .LBB92
.LBB90:
    leaq 16(%r11), %rdi
    movq (%rdi), %rax
    movq %rax, 88(%rsp)
    jmp .LBB97
.LBB91:
    imull $104729, %edx, %esi
    movq %rsi, %rax
    andq $16383, %rax
    movl %eax, %r11d
    shlq $5, %r11
    leaq node_pool(%rip), %r10
    addq %r11, %r10
    movslq 24(%r10), %r9
    movq 168(%rsp), %r11
    jmp .LBB100
.LBB92:
    addl $1, %edx
    movq 96(%rsp), %r8
    jmp .LBB95
.LBB93:
    movq 8(%r11), %rax
    movq %rax, 88(%rsp)
    jmp .LBB97
.p2align 4,,10
.p2align 3
.LBB94:
    leaq 24(%r11), %rsi
    movl (%rsi), %r10d
    cmpl %r10d, %r9d
jl .LBB90
    jmp .LBB98
.p2align 4,,10
.p2align 3
.LBB95:
    cmpl $16384, %edx
jb .LBB91
    jmp .LBB102
.LBB96:
    movslq 28(%r10), %rax
    movslq %eax, %rsi
    movslq 24(%r10), %rax
    movslq %eax, %rdi
    shlq $16, %rdi
    xorq %rdi, %rsi
    addq %rsi, %r8
    movq %r8, 96(%rsp)
    jmp .LBB92
.LBB97:
    movq 88(%rsp), %r11
    testq %r11, %r11
jne .LBB94
    jmp .LBB101
.LBB98:
    cmpl %r10d, %r9d
jg .LBB93
.LBB99:
    movq %r11, %r10
    jmp .LBB88
.LBB100:
    testq %r11, %r11
jne .LBB94
.LBB101:
    xorl %r10d, %r10d
    jmp .LBB88
.LBB102:
    xorl %esi, %esi
    jmp .LBB110
.LBB103:
    testq %r11, %r11
jne .LBB111
.LBB104:
    movq %r8, 80(%rsp)
    jmp .LBB107
.LBB105:
    leaq 16(%rdx), %r9
    movq (%r9), %rax
    movq %rax, 72(%rsp)
    jmp .LBB112
.LBB106:
    imull $104729, %esi, %edi
    movq %rdi, %rax
    andq $16383, %rax
    movl %eax, %edx
    shlq $5, %rdx
    leaq node_pool(%rip), %r11
    addq %rdx, %r11
    leaq 24(%r11), %r9
    movl (%r9), %r10d
    movq 168(%rsp), %rdx
    jmp .LBB115
.LBB107:
    addl $1, %esi
    movq 80(%rsp), %r8
    jmp .LBB110
.LBB108:
    movq 8(%rdx), %rax
    movq %rax, 72(%rsp)
    jmp .LBB112
.p2align 4,,10
.p2align 3
.LBB109:
    leaq 24(%rdx), %rdi
    movl (%rdi), %r11d
    cmpl %r11d, %r10d
jl .LBB105
    jmp .LBB113
.p2align 4,,10
.p2align 3
.LBB110:
    cmpl $16384, %esi
jb .LBB106
    jmp .LBB117
.LBB111:
    movslq 28(%r11), %rax
    movslq %eax, %rdi
    movslq 24(%r11), %rax
    movslq %eax, %r9
    shlq $16, %r9
    xorq %r9, %rdi
    addq %rdi, %r8
    movq %r8, 80(%rsp)
    jmp .LBB107
.LBB112:
    movq 72(%rsp), %rdx
    testq %rdx, %rdx
jne .LBB109
    jmp .LBB116
.LBB113:
    cmpl %r11d, %r10d
jg .LBB108
.LBB114:
    movq %rdx, %r11
    jmp .LBB103
.LBB115:
    testq %rdx, %rdx
jne .LBB109
.LBB116:
    xorl %r11d, %r11d
    jmp .LBB103
.LBB117:
    xorl %edi, %edi
    jmp .LBB128
.LBB118:
    testq %rdx, %rdx
jne .LBB125
.LBB119:
    movq %r8, %r10
    jmp .LBB122
.LBB120:
    movq 16(%rsi), %rax
    movq %rax, 64(%rsp)
    jmp .LBB126
.p2align 4,,10
.p2align 3
.LBB121:
    imull $104729, %edi, %r11d
    movq %r11, %rax
    andq $16383, %rax
    movl %eax, %r10d
    shlq $5, %r10
    leaq node_pool(%rip), %rsi
    addq %r10, %rsi
    movslq 24(%rsi), %r9
    movq 168(%rsp), %r11
    movq %r11, %rsi
    jmp .LBB130
.LBB122:
    addl $1, %edi
    movq %r10, %r8
    cmpl $16384, %edi
jb .LBB121
    jmp .LBB132
.LBB123:
    movq 8(%rsi), %rax
    movq %rax, 64(%rsp)
    jmp .LBB126
.p2align 4,,10
.p2align 3
.LBB124:
    movslq 24(%rsi), %r10
    cmpl %r10d, %r9d
jl .LBB120
    jmp .LBB127
.LBB125:
    movslq 28(%rdx), %rax
    movslq %eax, %r9
    movslq 24(%rdx), %rax
    movslq %eax, %rdx
    shlq $16, %rdx
    xorq %rdx, %r9
    addq %r9, %r8
    movq %r8, %r10
    jmp .LBB122
.LBB126:
    movq 64(%rsp), %rsi
    testq %rsi, %rsi
jne .LBB124
    jmp .LBB131
.LBB127:
    cmpl %r10d, %r9d
jg .LBB123
    jmp .LBB129
.LBB128:
    cmpl $16384, %edi
jb .LBB121
    jmp .LBB132
.LBB129:
    movq %rsi, %rdx
    jmp .LBB118
.LBB130:
    testq %rsi, %rsi
jne .LBB124
.LBB131:
    xorl %edx, %edx
    jmp .LBB118
.LBB132:
    xorl %r11d, %r11d
    jmp .LBB143
.LBB133:
    testq %r9, %r9
jne .LBB140
.LBB134:
    movq %r8, %rsi
    jmp .LBB137
.LBB135:
    movq 16(%r10), %rax
    movq %rax, 56(%rsp)
    jmp .LBB141
.p2align 4,,10
.p2align 3
.LBB136:
    imull $104729, %r11d, %edi
    movq %rdi, %rax
    andq $16383, %rax
    movl %eax, %esi
    shlq $5, %rsi
    leaq node_pool(%rip), %r10
    addq %rsi, %r10
    movslq 24(%r10), %rdx
    movq 168(%rsp), %rdi
    movq %rdi, %r10
    jmp .LBB145
.LBB137:
    addl $1, %r11d
    movq %rsi, %r8
    cmpl $16384, %r11d
jb .LBB136
    jmp .LBB147
.LBB138:
    movq 8(%r10), %rax
    movq %rax, 56(%rsp)
    jmp .LBB141
.p2align 4,,10
.p2align 3
.LBB139:
    movslq 24(%r10), %rsi
    cmpl %esi, %edx
jl .LBB135
    jmp .LBB142
.LBB140:
    movslq 28(%r9), %rax
    movslq %eax, %rdx
    movslq 24(%r9), %rax
    movslq %eax, %r9
    shlq $16, %r9
    xorq %r9, %rdx
    movq %r8, %rax
    movq %rdx, %rcx
    addq %rdx, %rax
    movq %rax, %rsi
    jmp .LBB137
.LBB141:
    movq 56(%rsp), %r10
    testq %r10, %r10
jne .LBB139
    jmp .LBB146
.LBB142:
    cmpl %esi, %edx
jg .LBB138
    jmp .LBB144
.LBB143:
    cmpl $16384, %r11d
jb .LBB136
    jmp .LBB147
.LBB144:
    movq %r10, %r9
    jmp .LBB133
.LBB145:
    testq %r10, %r10
jne .LBB139
.LBB146:
    xorl %r9d, %r9d
    jmp .LBB133
.LBB147:
    xorl %edi, %edi
    movq %r8, %rbx
    jmp .LBB158
.LBB148:
    testq %rdx, %rdx
jne .LBB155
.LBB149:
    movq %rbx, %r10
    jmp .LBB152
.LBB150:
    movq 16(%rsi), %rax
    movq %rax, 48(%rsp)
    jmp .LBB156
.p2align 4,,10
.p2align 3
.LBB151:
    imull $104729, %edi, %esi
    movq %rsi, %rax
    andq $16383, %rax
    movl %eax, %edx
    shlq $5, %rdx
    leaq node_pool(%rip), %r11
    addq %rdx, %r11
    movl 24(%r11), %r8d
    movq 168(%rsp), %rsi
    jmp .LBB160
.LBB152:
    addl $1, %edi
    movq %r10, %rbx
    cmpl $16384, %edi
jb .LBB151
    jmp .LBB57
.LBB153:
    movq 8(%rsi), %rax
    movq %rax, 48(%rsp)
    jmp .LBB156
.p2align 4,,10
.p2align 3
.LBB154:
    movslq 24(%rsi), %r10
    cmpl %r10d, %r8d
jl .LBB150
    jmp .LBB157
.LBB155:
    movslq 28(%rdx), %rax
    movslq %eax, %r9
    leaq 24(%rdx), %r8
    movslq (%r8), %rax
    movslq %eax, %rdx
    shlq $16, %rdx
    xorq %rdx, %r9
    addq %r9, %rbx
    movq %rbx, %r10
    jmp .LBB152
.LBB156:
    movq 48(%rsp), %rsi
    testq %rsi, %rsi
jne .LBB154
    jmp .LBB161
.LBB157:
    cmpl %r10d, %r8d
jg .LBB153
    jmp .LBB159
.LBB158:
    cmpl $16384, %edi
jb .LBB151
    jmp .LBB57
.LBB159:
    movq %rsi, %rdx
    jmp .LBB148
.LBB160:
    testq %rsi, %rsi
jne .LBB154
.LBB161:
    xorl %edx, %edx
    jmp .LBB148
