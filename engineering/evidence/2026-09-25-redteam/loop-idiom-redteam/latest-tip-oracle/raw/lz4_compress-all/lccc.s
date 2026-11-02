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
    xorl %r11d, %r11d
    movl $1511734397, %r10d
.LBB1:
    cmpl $524288, %r11d
jae .LBB46
.LBB2:
    imull $1664525, %r10d, %r10d
    leal 1013904223(%r10), %r8d
    movl %r8d, %r9d
    andl $15, %r9d
    cmpl $6, %r9d
jae .LBB45
    cmpl $128, %r11d
jb .LBB45
.LBB3:
    leal -128(%r11), %edx
    movl %r8d, %r10d
    andl $63, %r10d
    addl %r10d, %edx
    leaq src_data(%rip), %rcx
    movzbl (%rcx, %rdx), %edi
.LBB4:
    movl %r11d, %esi
    leaq src_data(%rip), %rcx
    movb %dil, (%rcx, %rsi)
    addl $1, %r11d
    movq %r8, %r10
    cmpl $524288, %r11d
jb .LBB2
    jmp .LBB46
.LBB5:
.LBB6:
    movl $16384, %eax
    subq $0, %rax
    leaq 0(,%rax,4), %r13
    testq %rax, %rax
jne .LBB44
.LBB7:
    movq 56(%rsp), %r8
    movq 88(%rsp), %rax
    movq %rax, 152(%rsp)
    movq 80(%rsp), %r14
.LBB8:
    cmpq 64(%rsp), %r8
jae .LBB32
.LBB9:
    movl (%r8), %r9d
    imull $-1640531535, %r9d, %r9d
    shrl $18, %r9d
    shlq $2, %r9
    leaq hash_table(%rip), %rsi
    addq %r9, %rsi
    movl (%rsi), %edx
    leaq src_data(%rip), %rcx
    leaq (%rcx, %rdx), %rbp
    movq %r8, %r11
    subq 80(%rsp), %r11
    movl %r11d, (%rsi)
    cmpq %r8, %rbp
jae .LBB42
.LBB10:
    cmpq 80(%rsp), %rbp
jb .LBB42
.LBB11:
    movl (%rbp), %r9d
    cmpl (%r8), %r9d
jne .LBB42
.LBB12:
    leaq 4(%rbp), %r13
    leaq 4(%r8), %rbx
.LBB13:
    cmpq 72(%rsp), %rbx
jae .LBB16
.LBB14:
    movzbl (%rbx), %esi
    movzbl (%r13), %edx
    cmpl %edx, %esi
jne .LBB16
.LBB15:
    leaq 1(%rbx), %rax
    movq %rax, 24(%rsp)
    leaq 1(%r13), %rax
    movq %rax, 16(%rsp)
    movq 24(%rsp), %rbx
    movq %rax, %r13
    cmpq 72(%rsp), %rbx
jb .LBB14
.LBB16:
    movq %r13, %r11
    subq %rbp, %r11
    leaq -4(%r11), %rax
    movq %rax, 48(%rsp)
    movq 152(%rsp), %rax
    addq $1, %rax
    movq %rax, 136(%rsp)
    subq %r14, %r8
    movl %r8d, %eax
    movq %rax, 40(%rsp)
    cmpl $15, %r8d
jb .LBB41
.LBB17:
    movq 152(%rsp), %rcx
    movb $240, (%rcx)
    movq 40(%rsp), %rbp
    subl $15, %ebp
    movq 136(%rsp), %r15
.LBB18:
    cmpl $255, %ebp
jb .LBB20
.LBB19:
    leaq 1(%r15), %r8
    movb $255, (%r15)
    subl $255, %ebp
    movq %r8, %r15
    cmpl $255, %ebp
jae .LBB19
.LBB20:
    movq %r15, %rax
    addq $1, %rax
    movq %rax, 136(%rsp)
    movzbl %bpl, %r9d
    movb %r9b, (%r15)
.LBB21:
    movl 40(%rsp), %r11d
    movq %r11, %rdi
    movq 136(%rsp), %rsi
    subq %r14, %rsi
    subq $1, %rdi
    cmpq %rdi, %rsi
    movl %r11d, %ebp
ja .LBB40
.LBB22:
    movq 136(%rsp), %r12
    xorl %r15d, %r15d
.LBB23:
    cmpq %rbp, %r15
jb .LBB37
.LBB24:
    movq %r12, 128(%rsp)
.LBB25:
    movq %rbx, %r10
    subq %r13, %r10
    movzwl %r10w, %r8d
    movq 128(%rsp), %r9
    addq $1, %r9
    movl %r8d, %edi
    movq 128(%rsp), %rcx
    movb %r8b, (%rcx)
    movq 128(%rsp), %r13
    addq $2, %r13
    sarl $8, %edi
    movb %dil, (%r9)
    movl 48(%rsp), %r14d
    cmpl $15, %r14d
jb .LBB39
.LBB26:
    movq 152(%rsp), %rcx
    movzbl (%rcx), %r8d
    orl $15, %r8d
    movb %r8b, (%rcx)
    movl %r14d, %r15d
    subl $15, %r15d
    movq %r13, %rbp
.LBB27:
    cmpl $255, %r15d
jb .LBB29
.LBB28:
    leaq 1(%rbp), %rdi
    movb $255, (%rbp)
    leal -255(%r15), %eax
    movl %eax, 28(%rsp)
    movq %rdi, %rbp
    movl 28(%rsp), %r15d
    cmpl $255, %r15d
jae .LBB28
.LBB29:
    leaq 1(%rbp), %r14
    movb %r15b, (%rbp)
.LBB30:
    movq %rbx, %r15
    movq %r14, %rbp
    movq %rbx, 120(%rsp)
.LBB31:
    movq %r15, %r8
    movq %rbp, 152(%rsp)
    movq 120(%rsp), %r14
    cmpq 64(%rsp), %r15
jb .LBB9
.LBB32:
    movq 72(%rsp), %r10
    subq %r14, %r10
    movl %r10d, %r13d
    movq 152(%rsp), %rax
    addq $1, %rax
    movq %rax, 32(%rsp)
    movl %r10d, %r9d
    shll $4, %r9d
    cmpl $15, %r10d
    movl $240, %edi
    cmovbl %r9d, %edi
    movq 152(%rsp), %rcx
    movb %dil, (%rcx)
    movl %r13d, %r10d
    movq %r10, %rdx
    movq 32(%rsp), %r8
    subq %r14, %r8
    subq $1, %rdx
    cmpq %rdx, %r8
    movl %r13d, %ebp
ja .LBB43
.LBB33:
    movq 32(%rsp), %r13
    xorl %r15d, %r15d
.LBB34:
    cmpq %rbp, %r15
jb .LBB38
.LBB35:
    movq %r13, 112(%rsp)
.LBB36:
    movq 112(%rsp), %rdi
    subq 88(%rsp), %rdi
    movl %edi, %esi
    movl %edi, %edx
    shlq $32, %rdx
    movzbl dst_data(%rip), %eax
    xorq %rax, %rdx
    shrl $1, %esi
    leaq dst_data(%rip), %rcx
    movzbl (%rcx, %rsi), %edi
    shlq $16, %rdi
    xorq %rdi, %rdx
    addq %rdx, 96(%rsp)
    movl 104(%rsp), %eax
    imull $4099, %eax, %eax
    movl %eax, %esi
    andq $524287, %rsi
    leaq src_data(%rip), %r8
    addq %rsi, %r8
    movzbl (%r8), %r9d
    xorl $85, %r9d
    movb %r9b, (%r8)
    movl 104(%rsp), %eax
    addl $1, %eax
    movq %rax, 104(%rsp)
    cmpl $24, %eax
jb .LBB5
    jmp .LBB48
.LBB37:
    leaq 1(%r12), %rax
    movq %rax, 24(%rsp)
    movzbl (%r14, %r15), %eax
    movb %al, (%r12)
    addq $1, %r15
    movq %r15, 16(%rsp)
    movq 24(%rsp), %r12
    cmpq %rbp, %r15
jb .LBB37
    jmp .LBB24
.LBB38:
    leaq 1(%r13), %r8
    movzbl (%r14, %r15), %eax
    movb %al, (%r13)
    addq $1, %r15
    movq %r15, 24(%rsp)
    movq %r8, %r13
    cmpq %rbp, %r15
jb .LBB38
    jmp .LBB35
.LBB39:
    movzbl 48(%rsp), %edx
    movq 152(%rsp), %rcx
    movzbl (%rcx), %eax
    orl %eax, %edx
    movb %dl, (%rcx)
    movq %r13, %r14
    jmp .LBB30
.LBB40:
    movq 136(%rsp), %rdi
    movq %r14, %rsi
    movq %r11, %rdx
    call memmove@PLT
    movq 136(%rsp), %rcx
    movq 40(%rsp), %rax
    addq %rcx, %rax
    movq %rax, 128(%rsp)
    jmp .LBB25
.LBB41:
    movl 40(%rsp), %edi
    shll $4, %edi
    movq 152(%rsp), %rcx
    movb %dil, (%rcx)
    jmp .LBB21
.LBB42:
    movq %r8, %rdx
    subq %r14, %rdx
    sarq $6, %rdx
    addq $1, %rdx
    leaq (%r8, %rdx, 1), %r15
    movq 152(%rsp), %rax
    movq %rax, %rbp
    movq %r14, 120(%rsp)
    jmp .LBB31
.LBB43:
    movq 32(%rsp), %rdi
    movq %r14, %rsi
    movq %r10, %rdx
    call memmove@PLT
    movq 32(%rsp), %rcx
    addq %rcx, %r13
    movq %r13, 112(%rsp)
    jmp .LBB36
.LBB44:
    leaq hash_table(%rip), %rdi
    xorl %esi, %esi
    movq %r13, %rdx
    call memset@PLT
    jmp .LBB7
.LBB45:
    movl %r8d, %r10d
    shrl $24, %r10d
    movzbl %r10b, %edi
    jmp .LBB4
.LBB46:
    movq $0, 104(%rsp)
    movq $0, 96(%rsp)
.LBB47:
    movl $0, %eax
    cmpl $24, %eax
jb .LBB5
.LBB48:
    leaq .Lstr0(%rip), %rdi
    movq 96(%rsp), %rsi
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
