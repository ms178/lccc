.Lstr0:

src_data:
dst_data:
hash_table:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $168, %rsp
    leaq dst_data(%rip), %rax
    movq %rax, 88(%rsp)
    leaq src_data(%rip), %rax
    movq %rax, 80(%rsp)
    addq $524288, %rax
    movq %rax, 72(%rsp)
    movq 80(%rsp), %rax
    addq $524276, %rax
    movq %rax, 64(%rsp)
    movq 80(%rsp), %rax
    addq $1, %rax
    movq %rax, 56(%rsp)
    xorl %ebp, %ebp
    movl $1511734397, %r11d
.LBB1:
    cmpl $524288, %ebp
jb .LBB2
.LBB48:
    movq $0, 152(%rsp)
    movq $0, 144(%rsp)
    jmp .LBB49
.LBB2:
    imull $1664525, %r11d, %r11d
    leal 1013904223(%r11), %r10d
    cmpl $128, %ebp
    setae %r8b
    movzbl %r8b, %r8d
    movzbl %bpl, %r9d
    cmpl $96, %r9d
    setb %dil
    movzbl %dil, %edi
    andl %edi, %r8d
je .LBB45
.LBB3:
    movl %ebp, %ebp
    leaq src_data(%rip), %rcx
    movzbl -128(%rcx, %rbp), %r8d
    jmp .LBB4
.LBB45:
    movl %r10d, %r9d
    andl $15, %r9d
    cmpl $6, %r9d
jae .LBB47
    cmpl $128, %ebp
jb .LBB47
.LBB46:
    leal -128(%rbp), %edx
    movl %r10d, %r11d
    andl $63, %r11d
    addl %r11d, %edx
    leaq src_data(%rip), %rcx
    movzbl (%rcx, %rdx), %r8d
    jmp .LBB4
.LBB47:
    movl %r10d, %edi
    shrl $24, %edi
    movzbl %dil, %r8d
.LBB4:
    movl %ebp, %esi
    leaq src_data(%rip), %rcx
    movb %r8b, (%rcx, %rsi)
    addl $1, %ebp
    movq %r10, %r11
    cmpl $524288, %ebp
jb .LBB2
    jmp .LBB48
.LBB49:
    movl 152(%rsp), %eax
    cmpl $2048, %eax
jae .LBB50
.LBB5:
    jmp .LBB6
.LBB7:
    movq 56(%rsp), %r10
    movq 88(%rsp), %rax
    movq %rax, 136(%rsp)
    movq 80(%rsp), %r14
.LBB8:
    cmpq 64(%rsp), %r10
jae .LBB32
.LBB9:
    movl (%r10), %r8d
    imull $-1640531535, %r8d, %r8d
    shrl $18, %r8d
    shlq $2, %r8
    leaq hash_table(%rip), %rdi
    addq %r8, %rdi
    movl (%rdi), %esi
    leaq src_data(%rip), %rcx
    leaq (%rcx, %rsi), %rdx
    movq %r10, %r11
    subq 80(%rsp), %r11
    movl %r11d, (%rdi)
    cmpq %r10, %rdx
jae .LBB42
.LBB10:
    cmpq 80(%rsp), %rdx
jb .LBB42
.LBB11:
    movl (%rdx), %r11d
    cmpl (%r10), %r11d
jne .LBB42
.LBB12:
    movq %rdx, %rbp
    addq $4, %rbp
    leaq 4(%r10), %rbx
.LBB13:
    cmpq 72(%rsp), %rbx
jae .LBB16
.LBB14:
    movzbl (%rbx), %r9d
    movzbl (%rbp), %edi
    cmpl %edi, %r9d
jne .LBB16
.LBB15:
    leaq 1(%rbx), %rax
    movq %rax, 24(%rsp)
    addq $1, %rbp
    movq 24(%rsp), %rbx
    cmpq 72(%rsp), %rbx
jb .LBB14
.LBB16:
    movq %rbp, %rsi
    subq %rdx, %rsi
    leaq -4(%rsi), %rax
    movq %rax, 48(%rsp)
    movq 136(%rsp), %rax
    addq $1, %rax
    movq %rax, 120(%rsp)
    subq %r14, %r10
    movl %r10d, %eax
    movq %rax, 40(%rsp)
    cmpl $15, %r10d
jb .LBB41
.LBB17:
    movq 136(%rsp), %rcx
    movb $240, (%rcx)
    movl 40(%rsp), %r13d
    subl $15, %r13d
    movq 120(%rsp), %r15
.LBB18:
    cmpl $255, %r13d
jb .LBB20
.LBB19:
    leaq 1(%r15), %r11
    movb $255, (%r15)
    subl $255, %r13d
    movq %r11, %r15
    cmpl $255, %r13d
jae .LBB19
.LBB20:
    movq %r15, %rax
    addq $1, %rax
    movq %rax, 120(%rsp)
    movb %r13b, (%r15)
    jmp .LBB21
.LBB41:
    movl 40(%rsp), %r8d
    shll $4, %r8d
    movq 136(%rsp), %rcx
    movb %r8b, (%rcx)
.LBB21:
    movl 40(%rsp), %r11d
    movq %r11, %rdi
    movq 120(%rsp), %rsi
    subq %r14, %rsi
    subq $1, %rdi
    cmpq %rdi, %rsi
    movl %r11d, %r13d
ja .LBB40
.LBB22:
    movq 120(%rsp), %r12
    xorl %r15d, %r15d
.LBB23:
    cmpq %r13, %r15
jb .LBB37
.LBB24:
    movq %r12, 112(%rsp)
    jmp .LBB25
.LBB37:
    leaq 1(%r12), %rax
    movq %rax, 24(%rsp)
    movzbl (%r14, %r15), %eax
    movb %al, (%r12)
    addq $1, %r15
    movq 24(%rsp), %r12
    cmpq %r13, %r15
jb .LBB37
    jmp .LBB24
.LBB25:
    movq %rbx, %r9
    subq %rbp, %r9
    movzwl %r9w, %edi
    movq 112(%rsp), %rsi
    addq $1, %rsi
    movl %edi, %edx
    movq 112(%rsp), %rcx
    movb %dil, (%rcx)
    movq 112(%rsp), %r15
    addq $2, %r15
    sarl $8, %edx
    movb %dl, (%rsi)
    movl 48(%rsp), %r13d
    cmpl $15, %r13d
jb .LBB39
.LBB26:
    movq 136(%rsp), %rcx
    movzbl (%rcx), %edi
    orl $15, %edi
    movb %dil, (%rcx)
    movl %r13d, %r14d
    subl $15, %r14d
.LBB27:
    cmpl $255, %r14d
jb .LBB29
.LBB28:
    leaq 1(%r15), %rdx
    movb $255, (%r15)
    leal -255(%r14), %eax
    movl %eax, 28(%rsp)
    movq %rdx, %r15
    movl %eax, %r14d
    cmpl $255, %eax
jae .LBB28
.LBB29:
    leaq 1(%r15), %r13
    movb %r14b, (%r15)
    jmp .LBB30
.LBB39:
    movzbl 48(%rsp), %edi
    movq 136(%rsp), %rcx
    movzbl (%rcx), %eax
    orl %eax, %edi
    movb %dil, (%rcx)
    movq %r15, %r13
.LBB30:
    movq %rbx, %r15
    movq %rbx, 104(%rsp)
    jmp .LBB31
.LBB42:
    movq %r10, %r8
    subq %r14, %r8
    sarq $6, %r8
    addq $1, %r8
    leaq (%r10, %r8, 1), %r15
    movq 136(%rsp), %r13
    movq %r14, 104(%rsp)
.LBB31:
    movq %r15, %r10
    movq %r13, 136(%rsp)
    movq 104(%rsp), %r14
    cmpq 64(%rsp), %r15
jb .LBB9
.LBB32:
    movq 72(%rsp), %r9
    subq %r14, %r9
    movl %r9d, %ebp
    movq 136(%rsp), %rax
    addq $1, %rax
    movq %rax, 32(%rsp)
    movl %ebp, %esi
    shll $4, %esi
    cmpl $15, %ebp
    movl $240, %edx
    cmovbl %esi, %edx
    movq 136(%rsp), %rcx
    movb %dl, (%rcx)
    movl %ebp, %r10d
    movq %r10, %r8
    movq 32(%rsp), %r9
    subq %r14, %r9
    subq $1, %r8
    cmpq %r8, %r9
    movl %ebp, %r15d
ja .LBB43
.LBB33:
    movq 32(%rsp), %rax
    movq %rax, %rbp
    xorl %r13d, %r13d
.LBB34:
    cmpq %r15, %r13
jb .LBB38
.LBB35:
    movq %rbp, 96(%rsp)
    jmp .LBB36
.LBB38:
    leaq 1(%rbp), %rsi
    movzbl (%r14, %r13), %eax
    movq %rbp, %rcx
    movb %al, (%rcx)
    addq $1, %r13
    movq %rsi, %rbp
    cmpq %r15, %r13
jb .LBB38
    jmp .LBB35
.LBB36:
    movq 96(%rsp), %r9
    subq 88(%rsp), %r9
    movl %r9d, %edi
    movl %r9d, %esi
    shlq $32, %rsi
    movzbl dst_data(%rip), %eax
    xorq %rax, %rsi
    shrl $1, %edi
    leaq dst_data(%rip), %rcx
    movzbl (%rcx, %rdi), %r9d
    shlq $16, %r9
    xorq %r9, %rsi
    addq %rsi, 144(%rsp)
    movl 152(%rsp), %eax
    imull $4099, %eax, %eax
    movl %eax, %edi
    andq $524287, %rdi
    leaq src_data(%rip), %rdx
    addq %rdi, %rdx
    movzbl (%rdx), %r8d
    xorl $85, %r8d
    movb %r8b, (%rdx)
    movl 152(%rsp), %eax
    addl $1, %eax
    movq %rax, 152(%rsp)
    cmpl $2048, %eax
jb .LBB5
.LBB50:
    leaq .Lstr0(%rip), %rdi
    movq 144(%rsp), %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $168, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB40:
    movq 120(%rsp), %rdi
    movq %r14, %rsi
    movq %r11, %rdx
    call memmove@PLT
    movq 120(%rsp), %rcx
    movq 40(%rsp), %rax
    addq %rcx, %rax
    movq %rax, 112(%rsp)
    jmp .LBB25
.LBB43:
    movq 32(%rsp), %rdi
    movq %r14, %rsi
    movq %r10, %rdx
    call memmove@PLT
    movq 32(%rsp), %rcx
    movq %rbp, %rax
    addq %rcx, %rax
    movq %rax, 96(%rsp)
    jmp .LBB36
.LBB6:
    movl $16384, %eax
    subq $0, %rax
    leaq 0(,%rax,4), %r11
    testq %rax, %rax
je .LBB7
.LBB44:
    leaq hash_table(%rip), %rdi
    xorl %esi, %esi
    movq %r11, %rdx
    call memset@PLT
    jmp .LBB7
