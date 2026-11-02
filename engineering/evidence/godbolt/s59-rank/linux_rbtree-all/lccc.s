.Lstr0:

node_pool:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $120, %rsp
    movq $0, 104(%rsp)
    movq $0, 96(%rsp)
    movq $1294694901, 88(%rsp)
    leaq node_pool(%rip), %rax
    movq %rax, 80(%rsp)
.LBB1:
    movl $0, %eax
    cmpq $16384, %rax
jae .LBB55
.LBB2:
    movq 88(%rsp), %rdx
    imull $1664525, %edx, %edx
    leal 1013904223(%rdx), %eax
    movq %rax, 88(%rsp)
    movq 80(%rsp), %rax
    leaq 24(%rax), %rsi
    movl 88(%rsp), %eax
    andq $2147483647, %rax
    movl %eax, (%rsi)
    movq 80(%rsp), %rax
    movl 96(%rsp), %ebp
    movl %ebp, 28(%rax)
    movq 80(%rsp), %rax
    movq $0, 16(%rax)
    movq 80(%rsp), %rax
    movq $0, 8(%rax)
    movq 80(%rsp), %rax
    leaq 24(%rax), %rsi
    leaq 104(%rsp), %rdx
    movq $0, 64(%rsp)
.LBB3:
    movq (%rdx), %rdi
    testq %rdi, %rdi
je .LBB5
.LBB4:
    movl (%rsi), %r9d
    movslq 24(%rdi), %rbp
    leaq 16(%rdi), %rax
    movq %rax, 24(%rsp)
    leaq 8(%rdi), %rax
    movq %rax, 16(%rsp)
    cmpl %ebp, %r9d
    movq 24(%rsp), %rcx
    movq %rax, %rdx
    cmovlq %rcx, %rdx
    movq %rdi, 64(%rsp)
    jmp .LBB3
.LBB5:
    movq 80(%rsp), %rcx
    movq 64(%rsp), %rax
    movq %rax, (%rcx)
    movq 80(%rsp), %rax
    movq %rax, (%rdx)
    movq 80(%rsp), %rcx
    movq (%rcx), %rdx
    movq %rdx, %rbp
    andq $-4, %rbp
    movq %rcx, %r9
    movq %rbp, %rdi
.LBB6:
    testq %rdi, %rdi
je .LBB54
.LBB7:
    movq (%rdi), %rsi
    movq %rsi, %rdx
    andq $1, %rdx
jne .LBB22
.LBB8:
    andq $-4, %rsi
    movq 8(%rsi), %r8
    cmpq %r8, %rdi
je .LBB12
.LBB9:
    testq %r8, %r8
je .LBB42
.LBB10:
    movq (%r8), %rdx
    testb $1, %dl
jne .LBB42
.LBB11:
    movq %rdx, %rax
    orq $1, %rax
    movq %rax, (%r8)
    movq (%rdi), %rax
    orq $1, %rax
    movq %rax, (%rdi)
    movq (%rsi), %r8
    andq $-2, %r8
    movq %r8, (%rsi)
    movq %r8, %r11
    andq $-4, %r11
    movq %rsi, %r9
    movq %r11, %rdi
    jmp .LBB6
.LBB12:
    movq 16(%rsi), %r8
    testq %r8, %r8
je .LBB15
.LBB13:
    movq (%r8), %rdx
    testb $1, %dl
jne .LBB15
.LBB14:
    movq %rdx, %rax
    orq $1, %rax
    movq %rax, (%r8)
    movq (%rdi), %rax
    orq $1, %rax
    movq %rax, (%rdi)
    movq (%rsi), %r8
    andq $-2, %r8
    movq %r8, (%rsi)
    movq %r8, %r10
    andq $-4, %r10
    movq %rsi, %r9
    movq %r10, %rdi
    jmp .LBB6
.LBB15:
    movq 16(%rdi), %r8
    cmpq %r8, %r9
je .LBB39
.LBB16:
    movq %rdi, %r15
.LBB17:
    movq %r8, 8(%rsi)
    movq %rsi, 16(%r15)
    testq %r8, %r8
je .LBB19
.LBB18:
    movq %rsi, %rax
    orq $1, %rax
    movq %rax, (%r8)
.LBB19:
    movq (%rsi), %r8
    movq %r8, %r9
    andq $-4, %r9
    movq %r8, (%r15)
    movq %r15, (%rsi)
je .LBB38
.LBB20:
    movq 16(%r9), %rdx
    cmpq %rsi, %rdx
jne .LBB37
.LBB21:
    movq %r15, 16(%r9)
.LBB22:
    movq 96(%rsp), %rax
    addq $1, %rax
    movq %rax, 96(%rsp)
    movq 80(%rsp), %rax
    leaq 32(%rax), %rax
    movq %rax, 80(%rsp)
    movq 96(%rsp), %rax
    cmpq $16384, %rax
jb .LBB2
    jmp .LBB55
.LBB23:
    cmpl $16384, %r9d
jae .LBB56
.LBB24:
    imull $104729, %r9d, %edx
    movq %rdx, %rax
    andq $16383, %rax
    movl %eax, %r11d
    shlq $5, %r11
    leaq node_pool(%rip), %r10
    addq %r11, %r10
    movslq 24(%r10), %rsi
    movq 104(%rsp), %rdx
    movq %rdx, %r11
.LBB25:
    testq %r11, %r11
jne .LBB30
.LBB26:
    xorl %r10d, %r10d
.LBB27:
    testq %r10, %r10
jne .LBB36
.LBB28:
    movq %rdi, %r8
.LBB29:
    movq %r9, %rax
    addl $1, %eax
    movl %eax, 28(%rsp)
    movl %eax, %r11d
    movq %r11, %r9
    movq %r8, %rdi
    jmp .LBB23
.LBB30:
    movl 24(%r11), %r8d
    cmpl %r8d, %esi
jge .LBB33
.LBB31:
    movq 16(%r11), %rbx
.LBB32:
    movq %rbx, %r11
    testq %rbx, %rbx
jne .LBB30
    jmp .LBB26
.LBB33:
    cmpl %r8d, %esi
jle .LBB35
.LBB34:
    movq 8(%r11), %rax
    movq %rax, 24(%rsp)
    movq %rax, %rbx
    jmp .LBB32
.LBB35:
    movq %r11, %r10
    jmp .LBB27
.LBB36:
    movslq 28(%r10), %rax
    movslq %eax, %rsi
    movslq 24(%r10), %rax
    movslq %eax, %rdx
    shlq $16, %rdx
    xorq %rdx, %rsi
    addq %rsi, %rdi
    movq %rdi, 24(%rsp)
    movq %rdi, %r8
    jmp .LBB29
.LBB37:
    movq %r15, 8(%r9)
    jmp .LBB22
.LBB38:
    movq %r15, 104(%rsp)
    jmp .LBB22
.LBB39:
    leaq 8(%r9), %r8
    movq (%r8), %rdx
    movq %rdx, 16(%rdi)
    movq %rdi, (%r8)
    testq %rdx, %rdx
je .LBB41
.LBB40:
    movq %rdi, %rax
    orq $1, %rax
    movq %rax, (%rdx)
.LBB41:
    movq %r9, (%rdi)
    movq 16(%r9), %r8
    movq %r9, %r15
    jmp .LBB17
.LBB42:
    movq 8(%rdi), %rdx
    cmpq %rdx, %r9
je .LBB51
.LBB43:
    movq %rdi, %r8
.LBB44:
    movq %rdx, 16(%rsi)
    movq %rsi, 8(%r8)
    testq %rdx, %rdx
je .LBB46
.LBB45:
    movq %rsi, %rax
    orq $1, %rax
    movq %rax, (%rdx)
.LBB46:
    movq (%rsi), %r9
    movq %r9, %rdi
    andq $-4, %rdi
    movq %r9, (%r8)
    movq %r8, (%rsi)
je .LBB50
.LBB47:
    cmpq %rsi, 16(%rdi)
jne .LBB49
.LBB48:
    movq %r8, 16(%rdi)
    jmp .LBB22
.LBB49:
    movq %r8, 8(%rdi)
    jmp .LBB22
.LBB50:
    movq %r8, 104(%rsp)
    jmp .LBB22
.LBB51:
    leaq 16(%r9), %r8
    movq (%r8), %rdx
    movq %rdx, 8(%rdi)
    movq %rdi, (%r8)
    testq %rdx, %rdx
je .LBB53
.LBB52:
    movq %rdi, %rax
    orq $1, %rax
    movq %rax, (%rdx)
.LBB53:
    movq %r9, (%rdi)
    movq 8(%r9), %rdx
    movq %r9, %r8
    jmp .LBB44
.LBB54:
    movq (%r9), %r8
    movq %r8, %rax
    orq $1, %rax
    movq %rax, (%r9)
    jmp .LBB22
.LBB55:
    xorl %r9d, %r9d
    xorl %edi, %edi
    jmp .LBB23
.LBB56:
    xorl %r9d, %r9d
    movq %rdi, %rsi
.LBB57:
    cmpl $16384, %r9d
jae .LBB71
.LBB58:
    imull $104729, %r9d, %r11d
    movq %r11, %rax
    andq $16383, %rax
    movl %eax, %r10d
    shlq $5, %r10
    leaq node_pool(%rip), %r8
    addq %r10, %r8
    movslq 24(%r8), %rdx
    movq 104(%rsp), %r11
    movq %r11, %r10
.LBB59:
    testq %r10, %r10
je .LBB63
.LBB60:
    movslq 24(%r10), %rdi
    cmpl %edi, %edx
jge .LBB67
.LBB61:
    movq 16(%r10), %r12
.LBB62:
    movq %r12, %r10
    testq %r12, %r12
jne .LBB60
.LBB63:
    xorl %edx, %edx
.LBB64:
    testq %rdx, %rdx
jne .LBB70
.LBB65:
    movq %rsi, %rdi
.LBB66:
    addl $1, %r9d
    movq %rdi, %rsi
    jmp .LBB57
.LBB67:
    cmpl %edi, %edx
jle .LBB69
.LBB68:
    movq 8(%r10), %rax
    movq %rax, 24(%rsp)
    movq %rax, %r12
    jmp .LBB62
.LBB69:
    movq %r10, %rdx
    jmp .LBB64
.LBB70:
    movslq 28(%rdx), %rax
    movslq %eax, %r8
    leaq 24(%rdx), %rbp
    movslq (%rbp), %rax
    movslq %eax, %rdx
    shlq $16, %rdx
    xorq %rdx, %r8
    leaq (%rsi, %r8, 1), %rdi
    jmp .LBB66
.LBB71:
    xorl %r11d, %r11d
    movq %rsi, %rdi
.LBB72:
    cmpl $16384, %r11d
jae .LBB86
.LBB73:
    imull $104729, %r11d, %r8d
    movq %r8, %rax
    andq $16383, %rax
    movl %eax, %r9d
    shlq $5, %r9
    leaq node_pool(%rip), %rsi
    addq %r9, %rsi
    movslq 24(%rsi), %r10
    movq 104(%rsp), %r8
    movq %r8, %r9
.LBB74:
    testq %r9, %r9
je .LBB78
.LBB75:
    movslq 24(%r9), %rdx
    cmpl %edx, %r10d
jge .LBB82
.LBB76:
    movq 16(%r9), %r13
.LBB77:
    movq %r13, %r9
    testq %r13, %r13
jne .LBB75
.LBB78:
    xorl %edx, %edx
.LBB79:
    testq %rdx, %rdx
jne .LBB85
.LBB80:
    movq %rdi, %r10
.LBB81:
    addl $1, %r11d
    movq %r10, %rdi
    jmp .LBB72
.LBB82:
    cmpl %edx, %r10d
jle .LBB84
.LBB83:
    movq 8(%r9), %rax
    movq %rax, 24(%rsp)
    movq %rax, %r13
    jmp .LBB77
.LBB84:
    movq %r9, %rdx
    jmp .LBB79
.LBB85:
    movslq 28(%rdx), %rax
    movslq %eax, %rsi
    movslq 24(%rdx), %rax
    movslq %eax, %rdx
    shlq $16, %rdx
    xorq %rdx, %rsi
    leaq (%rdi, %rsi, 1), %r10
    jmp .LBB81
.LBB86:
    xorl %r11d, %r11d
    movq %rdi, %r10
.LBB87:
    cmpl $16384, %r11d
jae .LBB101
.LBB88:
    imull $104729, %r11d, %r9d
    movq %r9, %rax
    andq $16383, %rax
    movl %eax, %edi
    shlq $5, %rdi
    leaq node_pool(%rip), %rsi
    addq %rdi, %rsi
    movslq 24(%rsi), %r8
    movq 104(%rsp), %r9
    movq %r9, %rsi
.LBB89:
    testq %rsi, %rsi
je .LBB93
.LBB90:
    movslq 24(%rsi), %rdi
    cmpl %edi, %r8d
jge .LBB97
.LBB91:
    movq 16(%rsi), %r14
.LBB92:
    movq %r14, %rsi
    testq %r14, %r14
jne .LBB90
.LBB93:
    xorl %r8d, %r8d
.LBB94:
    testq %r8, %r8
jne .LBB100
.LBB95:
    movq %r10, %rdi
.LBB96:
    addl $1, %r11d
    movq %rdi, %r10
    jmp .LBB87
.LBB97:
    cmpl %edi, %r8d
jle .LBB99
.LBB98:
    movq 8(%rsi), %rax
    movq %rax, 24(%rsp)
    movq %rax, %r14
    jmp .LBB92
.LBB99:
    movq %rsi, %r8
    jmp .LBB94
.LBB100:
    movslq 28(%r8), %rax
    movslq %eax, %rdx
    movslq 24(%r8), %rax
    movslq %eax, %r8
    shlq $16, %r8
    xorq %r8, %rdx
    leaq (%r10, %rdx, 1), %rdi
    jmp .LBB96
.LBB101:
    xorl %r9d, %r9d
    movq %r10, %rdi
.LBB102:
    cmpl $16384, %r9d
jae .LBB116
.LBB103:
    imull $104729, %r9d, %edx
    movq %rdx, %rax
    andq $16383, %rax
    movl %eax, %r11d
    shlq $5, %r11
    leaq node_pool(%rip), %r10
    addq %r11, %r10
    movslq 24(%r10), %rsi
    movq 104(%rsp), %rdx
    movq %rdx, %r11
.LBB104:
    testq %r11, %r11
je .LBB108
.LBB105:
    movl 24(%r11), %r8d
    cmpl %r8d, %esi
jge .LBB112
.LBB106:
    movq 16(%r11), %rax
    movq %rax, 56(%rsp)
.LBB107:
    movq 56(%rsp), %r11
    testq %r11, %r11
jne .LBB105
.LBB108:
    xorl %r8d, %r8d
.LBB109:
    testq %r8, %r8
jne .LBB115
.LBB110:
    movq %rdi, %rsi
.LBB111:
    addl $1, %r9d
    movq %rsi, %rdi
    jmp .LBB102
.LBB112:
    cmpl %r8d, %esi
jle .LBB114
.LBB113:
    movq 8(%r11), %rax
    movq %rax, 56(%rsp)
    jmp .LBB107
.LBB114:
    movq %r11, %r8
    jmp .LBB109
.LBB115:
    movslq 28(%r8), %rax
    movslq %eax, %r10
    movslq 24(%r8), %rax
    movslq %eax, %r8
    shlq $16, %r8
    xorq %r8, %r10
    leaq (%rdi, %r10, 1), %rsi
    jmp .LBB111
.LBB116:
    xorl %r9d, %r9d
    movq %rdi, %rsi
.LBB117:
    cmpl $16384, %r9d
jae .LBB157
.LBB118:
    imull $104729, %r9d, %edx
    movq %rdx, %rax
    andq $16383, %rax
    movl %eax, %r11d
    shlq $5, %r11
    leaq node_pool(%rip), %r10
    addq %r11, %r10
    movslq 24(%r10), %rdi
    movq 104(%rsp), %r10
.LBB119:
    testq %r10, %r10
je .LBB123
.LBB120:
    movslq 24(%r10), %rdx
    cmpl %edx, %edi
jge .LBB149
.LBB121:
    movq 16(%r10), %rax
    movq %rax, 48(%rsp)
.LBB122:
    movq 48(%rsp), %r10
    testq %r10, %r10
jne .LBB120
.LBB123:
    xorl %r15d, %r15d
.LBB124:
    testq %r15, %r15
jne .LBB156
.LBB125:
    movq %rsi, %rdi
.LBB126:
    addl $1, %r9d
    movq %rdi, %rsi
    cmpl $16384, %r9d
jb .LBB118
    jmp .LBB157
.LBB127:
    imull $104729, %r9d, %esi
    movq %rsi, %rax
    andq $16383, %rax
    movl %eax, %edx
    shlq $5, %rdx
    leaq node_pool(%rip), %r11
    addq %rdx, %r11
    movl 24(%r11), %r8d
    movq 104(%rsp), %rsi
    movq %rsi, %rdx
.LBB128:
    testq %rdx, %rdx
je .LBB132
.LBB129:
    movl 24(%rdx), %r10d
    cmpl %r10d, %r8d
jge .LBB147
.LBB130:
    movq 16(%rdx), %rax
    movq %rax, 40(%rsp)
.LBB131:
    movq 40(%rsp), %rdx
    testq %rdx, %rdx
jne .LBB129
.LBB132:
    xorl %ebp, %ebp
.LBB133:
    testq %rbp, %rbp
jne .LBB155
.LBB134:
    movq %rdi, %r8
.LBB135:
    leal 1(%r9), %eax
    movl %eax, 28(%rsp)
    movl %eax, %r9d
    movq %r8, %rdi
    cmpl $16384, %r9d
jb .LBB127
    jmp .LBB159
.LBB136:
    imull $104729, %r9d, %edi
    movq %rdi, %rax
    andq $16383, %rax
    movl %eax, %esi
    shlq $5, %rsi
    leaq node_pool(%rip), %rdx
    addq %rsi, %rdx
    movl 24(%rdx), %r8d
    movq 104(%rsp), %rdi
    movq %rdi, %rsi
.LBB137:
    testq %rsi, %rsi
je .LBB141
.LBB138:
    movslq 24(%rsi), %r10
    cmpl %r10d, %r8d
jge .LBB145
.LBB139:
    movq 16(%rsi), %rax
    movq %rax, 32(%rsp)
.LBB140:
    movq 32(%rsp), %rsi
    testq %rsi, %rsi
jne .LBB138
.LBB141:
    xorl %r10d, %r10d
.LBB142:
    testq %r10, %r10
jne .LBB154
.LBB143:
    movq %r11, %r8
.LBB144:
    leal 1(%r9), %eax
    movl %eax, 28(%rsp)
    movl %eax, %r9d
    movq %r8, %r11
    cmpl $16384, %r9d
jb .LBB136
    jmp .LBB161
.LBB145:
    cmpl %r10d, %r8d
jle .LBB153
.LBB146:
    movq 8(%rsi), %rax
    movq %rax, 32(%rsp)
    jmp .LBB140
.LBB147:
    cmpl %r10d, %r8d
jle .LBB152
.LBB148:
    movq 8(%rdx), %rax
    movq %rax, 40(%rsp)
    jmp .LBB131
.LBB149:
    cmpl %edx, %edi
jle .LBB151
.LBB150:
    movq 8(%r10), %rax
    movq %rax, 48(%rsp)
    jmp .LBB122
.LBB151:
    movq %r10, %r15
    jmp .LBB124
.LBB152:
    movq %rdx, %rbp
    jmp .LBB133
.LBB153:
    movq %rsi, %r10
    jmp .LBB142
.LBB154:
    movslq 28(%r10), %rax
    movslq %eax, %rdx
    movslq 24(%r10), %rax
    movslq %eax, %r10
    shlq $16, %r10
    xorq %r10, %rdx
    addq %rdx, %r11
    movq %r11, 24(%rsp)
    movq %r11, %r8
    jmp .LBB144
.LBB155:
    movslq 28(%rbp), %rax
    movslq %eax, %r10
    movslq 24(%rbp), %rax
    movslq %eax, %rsi
    shlq $16, %rsi
    xorq %rsi, %r10
    leaq (%rdi, %r10, 1), %r8
    jmp .LBB135
.LBB156:
    movslq 28(%r15), %rdx
    movslq 24(%r15), %rax
    movslq %eax, %r10
    shlq $16, %r10
    xorq %r10, %rdx
    addq %rdx, %rsi
    movq %rsi, 24(%rsp)
    movq %rsi, %rdi
    jmp .LBB126
.LBB157:
    xorl %r9d, %r9d
    movq %rsi, %rdi
.LBB158:
    cmpl $16384, %r9d
jb .LBB127
.LBB159:
    xorl %r9d, %r9d
    movq %rdi, %r11
.LBB160:
    cmpl $16384, %r9d
jb .LBB136
.LBB161:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $120, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


