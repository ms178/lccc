.Lstr0:

check_known_values.values.0:

sqlite_varint_bytes:
sqlite_varint_offsets:
sqlite_varint_used:

sqlite_put_varint:
    pushq %rbx
    subq $32, %rsp
    cmpq $127, %rsi
ja .LBB2
.LBB1:
    andq $127, %rsi
    movb %sil, (%rdi)
    movl $1, %eax
.L__lccc_epilogue_1:
    addq $32, %rsp
    popq %rbx
    ret
.LBB2:
    cmpq $16383, %rsi
ja .LBB4
.LBB3:
    movq %rsi, %r9
    shrq $7, %r9
    andq $127, %r9
    orq $128, %r9
    movb %r9b, (%rdi)
    andq $127, %rsi
    movb %sil, 1(%rdi)
    movl $2, %eax
    jmp .L__lccc_epilogue_1
.LBB4:
    movq %rsi, %r9
    movabsq $-72057594037927936, %rax
    andq %rax, %r9
jne .LBB12
.LBB5:
    movq %rsi, %r11
    xorl %r10d, %r10d
    jmp .LBB6
.LBB12:
    movb %sil, 8(%rdi)
    movq %rsi, %r9
    shrq $8, %r9
    andq $127, %r9
    orq $128, %r9
    movb %r9b, 7(%rdi)
    movq %rsi, %rdx
    shrq $15, %rdx
    andq $127, %rdx
    orq $128, %rdx
    movb %dl, 6(%rdi)
    movq %rsi, %r11
    shrq $22, %r11
    andq $127, %r11
    orq $128, %r11
    movb %r11b, 5(%rdi)
    movq %rsi, %r8
    shrq $29, %r8
    andq $127, %r8
    orq $128, %r8
    movb %r8b, 4(%rdi)
    movq %rsi, %r10
    shrq $36, %r10
    andq $127, %r10
    orq $128, %r10
    movb %r10b, 3(%rdi)
    movq %rsi, %r9
    shrq $43, %r9
    andq $127, %r9
    orq $128, %r9
    movb %r9b, 2(%rdi)
    movq %rsi, %rdx
    shrq $50, %rdx
    andq $127, %rdx
    orq $128, %rdx
    movb %dl, 1(%rdi)
    shrq $57, %rsi
    andq $127, %rsi
    orq $128, %rsi
    movb %sil, (%rdi)
    movl $9, %eax
    jmp .L__lccc_epilogue_1
.LBB6:
    leal 1(%r10), %esi
    movslq %r10d, %rdx
    movq %r11, %r9
    andq $127, %r9
    orq $128, %r9
    movb %r9b, 8(%rsp, %rdx)
    shrq $7, %r11
    testq %r11, %r11
je .LBB8
.LBB7:
    movq %rsi, %r10
    jmp .LBB6
.LBB8:
    movzbl 8(%rsp), %r11d
    andl $127, %r11d
    movb %r11b, 8(%rsp)
    movslq %r10d, %r8
    xorl %r9d, %r9d
.LBB9:
    testq %r8, %r8
jl .LBB11
.LBB10:
    movzbl 8(%rsp, %r8), %eax
    movb %al, (%rdi, %r9)
    subq $1, %r8
    addq $1, %r9
    testq %r8, %r8
jge .LBB10
.LBB11:
    movl %esi, %eax
    jmp .L__lccc_epilogue_1

sqlite_get_varint:
    movsbl (%rdi), %edx
    testb %dl, %dl
jl .LBB15
.LBB14:
    movzbl %dl, %eax
    movq %rax, (%rsi)
    movl $1, %eax
    ret
.LBB15:
    movsbq 1(%rdi), %r9
    testb %r9b, %r9b
jl .LBB17
.LBB16:
    andl $127, %edx
    shll $7, %edx
    movzbl %r9b, %r10d
    orl %r10d, %edx
    movq %rdx, (%rsi)
    movl $2, %eax
    ret
.LBB17:
    movzbl %dl, %r8d
    shll $14, %r8d
    movzbl %r9b, %r11d
    movzbl 2(%rdi), %edx
    orl %edx, %r8d
    testb $128, %r8b
jne .LBB19
.LBB18:
    andl $2080895, %r8d
    andl $127, %r11d
    shll $7, %r11d
    orl %r11d, %r8d
    movq %r8, (%rsi)
    movl $3, %eax
    ret
.LBB19:
    andl $2080895, %r8d
    shll $14, %r11d
    movzbl 3(%rdi), %r9d
    orl %r9d, %r11d
    testb $128, %r11b
jne .LBB21
.LBB20:
    andl $2080895, %r11d
    shll $7, %r8d
    orl %r11d, %r8d
    movq %r8, (%rsi)
    movl $4, %eax
    ret
.LBB21:
    andl $2080895, %r11d
    movl %r8d, %r10d
    shll $14, %r10d
    movzbl 4(%rdi), %edx
    orl %edx, %r10d
    testb $128, %r10b
jne .LBB23
.LBB22:
    shll $7, %r11d
    orl %r11d, %r10d
    shrl $18, %r8d
    shlq $32, %r8
    orq %r10, %r8
    movq %r8, (%rsi)
    movl $5, %eax
    ret
.LBB23:
    shll $7, %r8d
    orl %r11d, %r8d
    shll $14, %r11d
    movzbl 5(%rdi), %r9d
    orl %r9d, %r11d
    testb $128, %r11b
jne .LBB25
.LBB24:
    andl $2080895, %r10d
    shll $7, %r10d
    orl %r11d, %r10d
    shrl $18, %r8d
    shlq $32, %r8
    orq %r10, %r8
    movq %r8, (%rsi)
    movl $6, %eax
    ret
.LBB25:
    shll $14, %r10d
    movzbl 6(%rdi), %edx
    orl %edx, %r10d
    testb $128, %r10b
jne .LBB27
.LBB26:
    andl $-266354561, %r10d
    andl $2080895, %r11d
    shll $7, %r11d
    orl %r11d, %r10d
    shrl $11, %r8d
    shlq $32, %r8
    orq %r10, %r8
    movq %r8, (%rsi)
    movl $7, %eax
    ret
.LBB27:
    andl $2080895, %r10d
    shll $14, %r11d
    movzbl 7(%rdi), %edx
    orl %edx, %r11d
    testb $128, %r11b
jne .LBB29
.LBB28:
    andl $-266354561, %r11d
    shll $7, %r10d
    orl %r11d, %r10d
    shrl $4, %r8d
    shlq $32, %r8
    orq %r10, %r8
    movq %r8, (%rsi)
    movl $8, %eax
    ret
.LBB29:
    shll $15, %r10d
    movzbl 8(%rdi), %r9d
    orl %r9d, %r10d
    andl $2080895, %r11d
    shll $8, %r11d
    orl %r11d, %r10d
    shll $4, %r8d
    movzbl 4(%rdi), %eax
    movl %eax, %edi
    andl $127, %edi
    shrl $3, %edi
    orl %r8d, %edi
    shlq $32, %rdi
    orq %r10, %rdi
    movq %rdi, (%rsi)
    movl $9, %eax
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $72, %rsp
    leaq check_known_values.values.0(%rip), %rbx
    xorl %r12d, %r12d
.LBB31:
    cmpq $11, %r12
jae .LBB36
.LBB32:
    movq $0, 32(%rsp)
    movq (%rbx, %r12, 8), %r11
    leaq 40(%rsp), %rdi
    movq %r11, %rsi
    call sqlite_put_varint
    movl %eax, %r13d
    leaq 40(%rsp), %rdi
    leaq 32(%rsp), %rsi
    call sqlite_get_varint
    movzbl %al, %esi
    cmpl %r13d, %esi
jne .LBB35
.LBB33:
    movq 32(%rsp), %r11
    movq (%rbx, %r12, 8), %r9
    cmpq %r9, %r11
jne .LBB35
.LBB34:
    addq $1, %r12
    cmpq $11, %r12
jb .LBB32
    jmp .LBB36
.LBB35:
    movl $2, %eax
.L__lccc_epilogue_2:
    addq $72, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB36:
    leaq sqlite_varint_offsets(%rip), %r15
    movabsq $34359738368, %rbp
    movabsq $4398046511104, %rbx
    movabsq $562949953421312, %r12
    movabsq $-9223372036854775808, %rax
    movq %rax, 8(%rsp)
    xorl %r14d, %r14d
    xorl %r13d, %r13d
    movq $1831565813, 24(%rsp)
.LBB37:
    cmpl $262144, %r14d
jae .LBB49
.LBB38:
    movq 24(%rsp), %rdi
    imull $1664525, %edi, %edi
    leal 1013904223(%rdi), %eax
    movq %rax, 24(%rsp)
    movl %r14d, %esi
    imulq $954437177, %rsi, %rsi
    shrq $33, %rsi
    movl %esi, %r11d
    leaq (%r11, %r11, 8), %r11
    movl %r14d, %r10d
    subl %r11d, %r10d
    movl %r10d, %eax
    cmpq $8, %rax
    jae .LBB47
    leaq .Ljt_0(%rip), %rcx
    movslq (%rcx,%rax,4), %rdx
    addq %rcx, %rdx
    jmpq *%rdx
.Ljt_0:
.LBB39:
    movl 24(%rsp), %eax
    andq $127, %rax
    movl %eax, %r10d
    jmp .LBB48
.LBB40:
    movl 24(%rsp), %eax
    andq $16383, %rax
    movl %eax, %r8d
    movq %r8, %r10
    orq $128, %r10
    jmp .LBB48
.LBB41:
    movl 24(%rsp), %eax
    andq $2097151, %rax
    movl %eax, %r10d
    orq $16384, %r10
    jmp .LBB48
.LBB42:
    movl 24(%rsp), %eax
    andq $268435455, %rax
    movl %eax, %r10d
    orq $2097152, %r10
    jmp .LBB48
.LBB43:
    movl 24(%rsp), %esi
    shlq $3, %rsi
    movq %rsi, %r10
    orq $268435456, %r10
    jmp .LBB48
.LBB44:
    movl 24(%rsp), %r11d
    shlq $10, %r11
    movq %r11, %r10
    orq %rbp, %r10
    jmp .LBB48
.LBB45:
    movl 24(%rsp), %r8d
    shlq $17, %r8
    movq %r8, %r10
    orq %rbx, %r10
    jmp .LBB48
.LBB46:
    movl 24(%rsp), %r9d
    shlq $24, %r9
    movq %r9, %r10
    orq %r12, %r10
    jmp .LBB48
.LBB47:
    movl 24(%rsp), %edi
    shlq $25, %rdi
    orq 8(%rsp), %rdi
    movl %r14d, %esi
    movq %rdi, %r10
    orq %rsi, %r10
.LBB48:
    movl %r13d, (%r15, %r14, 4)
    movl %r13d, %r9d
    leaq sqlite_varint_bytes(%rip), %rdi
    addq %r9, %rdi
    movq %r10, %rsi
    call sqlite_put_varint
    movl %eax, %esi
    addl %esi, %r13d
    addl $1, %r14d
    cmpl $262144, %r14d
jb .LBB38
.LBB49:
    movl %r13d, sqlite_varint_used(%rip)
    xorl %r14d, %r14d
    xorl %r11d, %r11d
.LBB50:
    cmpl $24, %r14d
jae .LBB55
.LBB51:
    movq %r11, %rbp
    xorl %ebx, %ebx
.LBB52:
    cmpq $262144, %rbx
jae .LBB54
.LBB53:
    movq $0, 56(%rsp)
    movl (%r15, %rbx, 4), %r9d
    leaq sqlite_varint_bytes(%rip), %rdi
    addq %r9, %rdi
    leaq 56(%rsp), %rsi
    call sqlite_get_varint
    movl %eax, %esi
    movzbl %sil, %r10d
    movq 56(%rsp), %r8
    movl %r10d, %r9d
    movq %rbx, %rdi
    andq $31, %rdi
    shlxq %rdi, %r9, %r9
    xorq %r9, %r8
    addq %r8, %rbp
    addq $1, %rbx
    cmpq $262144, %rbx
jb .LBB53
.LBB54:
    imull $104729, %r14d, %esi
    andl $262143, %esi
    movl (%r15, %rsi, 4), %eax
    leaq sqlite_varint_bytes(%rip), %rdi
    addq %rax, %rdi
    movzbl (%rdi), %esi
    xorl $1, %esi
    movb %sil, (%rdi)
    addl $1, %r14d
    movq %rbp, %r11
    cmpl $24, %r14d
jb .LBB51
.LBB55:
    movl sqlite_varint_used(%rip), %r8d
    testl %r8d, %r8d
jne .LBB57
.LBB56:
    movl $3, %eax
    jmp .L__lccc_epilogue_2
.LBB57:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_2


