.Lstr0:

tab:

main.a.0:
main.d.1:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    leaq main.a.0(%rip), %rbx
    xorl %r10d, %r10d
.LBB1:
    cmpq $300, %r10
jge .LBB3
.LBB2:
    imulq $37, %r10, %r9
    leaq 11(%r9), %rdi
    movb %dil, (%rbx, %r10)
    addq $1, %r10
    cmpq $300, %r10
jl .LBB2
.LBB3:
    leaq main.d.1(%rip), %rdx
    leaq tab(%rip), %r10
    xorl %r8d, %r8d
    xorl %r9d, %r9d
.LBB4:
    cmpq $300, %r9
jge .LBB18
.LBB5:
    movzbl (%rbx, %r9), %edi
    shll $16, %edi
    movl $300, %esi
    subq %r9, %rsi
    cmpq $1, %rsi
jg .LBB7
.LBB6:
    movq %rdi, %r12
    jmp .LBB8
.LBB7:
    movzbl 1(%rbx, %r9), %r15d
    shll $8, %r15d
    movl %edi, %r12d
    orl %r15d, %r12d
.LBB8:
    cmpq $2, %rsi
jg .LBB10
.LBB9:
    movq %r12, %rdi
    jmp .LBB11
.LBB10:
    movzbl 2(%rbx, %r9), %eax
    movl %r12d, %edi
    orl %eax, %edi
.LBB11:
    leal 1(%r8), %r15d
    movslq %r8d, %rbp
    movl %edi, %r13d
    shrl $18, %r13d
    movl %r13d, %r11d
    andl $63, %r11d
    movsbq (%r10, %r11), %rax
    movb %al, (%rdx, %rbp)
    movl %r15d, %ebp
    movl %edi, %r13d
    shrl $12, %r13d
    movl %r13d, %r11d
    andl $63, %r11d
    movsbq (%r10, %r11), %rax
    movslq %r8d, %r8
    movb %al, 1(%rdx, %r8)
    cmpq $1, %rsi
jg .LBB13
.LBB12:
    movl $61, %ebp
    jmp .LBB14
.LBB13:
    movl %edi, %r12d
    shrl $6, %r12d
    movl %r12d, %r11d
    andl $63, %r11d
    movsbq (%r10, %r11), %rax
    movsbq %al, %rbp
.LBB14:
    movsbl %bpl, %r11d
    movslq %r8d, %r8
    movb %r11b, 2(%rdx, %r8)
    leal 4(%r8), %r11d
    cmpq $2, %rsi
jg .LBB16
.LBB15:
    movl $61, %esi
    jmp .LBB17
.LBB16:
    andl $63, %edi
    movsbq (%r10, %rdi), %rax
    movsbq %al, %rsi
.LBB17:
    movslq %r8d, %r8
    movb %sil, 3(%rdx, %r8)
    addq $3, %r9
    movq %r11, %r8
    cmpq $300, %r9
jl .LBB5
.LBB18:
    movl %r8d, %r11d
    movslq %r8d, %r10
    xorl %r8d, %r8d
.LBB19:
    cmpq %r10, %r8
jge .LBB21
.LBB20:
    movq %r11, %r9
    shlq $5, %r9
    addq %r11, %r9
    movsbq (%rdx, %r8), %rax
    movzbl %al, %eax
    movl %eax, %esi
    leaq (%r9, %rsi, 1), %r11
    addq $1, %r8
    cmpq %r10, %r8
jl .LBB20
.LBB21:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


