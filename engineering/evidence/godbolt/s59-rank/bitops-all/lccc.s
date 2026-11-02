.Lstr0:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    movl $3735928559, %ebx
    xorl %r9d, %r9d
    xorl %r11d, %r11d
    xorl %r10d, %r10d
    xorl %r12d, %r12d
    xorl %r13d, %r13d
.LBB1:
    cmpl $50000000, %r9d
jge .LBB6
.LBB2:
    imull $1664525, %ebx, %r8d
    leal 1013904223(%r8), %edi
    xorl %eax, %eax
    popcntl %edi, %eax
    movslq %eax, %rsi
    leaq (%r12, %rsi, 1), %rdx
    testl %edi, %edi
jne .LBB4
.LBB3:
    movl $32, %esi
    jmp .LBB5
.LBB4:
    xorl %esi, %esi
    lzcntl %edi, %esi
.LBB5:
    movslq %esi, %r14
    addq %r14, %r11
    movl %edi, %esi
    shrl $1, %esi
    andl $1431655765, %esi
    movl %edi, %r15d
    andl $1431655765, %r15d
    shll $1, %r15d
    orl %r15d, %esi
    movq %rsi, %rbp
    shrl $2, %ebp
    andl $858993459, %ebp
    andl $858993459, %esi
    shll $2, %esi
    movl %ebp, %r8d
    orl %esi, %r8d
    movl %r8d, %esi
    shrl $4, %esi
    andl $252645135, %esi
    andl $252645135, %r8d
    shll $4, %r8d
    orl %r8d, %esi
    movq %rsi, %r8
    bswapl %r8d
    movzbl %r8b, %eax
    addq %rax, %r13
    movzwl %di, %r8d
    subl $1, %r8d
    movl %r8d, %esi
    shrl $1, %esi
    orl %esi, %r8d
    movl %r8d, %esi
    shrl $2, %esi
    orl %esi, %r8d
    movl %r8d, %esi
    shrl $4, %esi
    orl %esi, %r8d
    movl %r8d, %esi
    shrl $8, %esi
    orl %esi, %r8d
    movl %r8d, %esi
    shrl $16, %esi
    orl %esi, %r8d
    leal 1(%r8), %r8d
    addq %r8, %r10
    addl $1, %r9d
    movq %rdi, %rbx
    movq %rdx, %r12
    cmpl $50000000, %r9d
jl .LBB2
.LBB6:
    leaq .Lstr0(%rip), %rdi
    movq %r12, %rsi
    movq %r11, %rdx
    movq %r13, %rcx
    movq %r10, %r8
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


