.Lstr0:

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
    leaq main.a.0(%rip), %r11
    xorl %r10d, %r10d
.LBB1:
    cmpq $1024, %r10
jge .LBB3
.LBB2:
    movq %r10, %r8
    shlq $5, %r8
    subq %r10, %r8
    leaq 7(%r8), %r9
    movw %r9w, (%r11, %r10, 2)
    addq $1, %r10
    cmpq $1024, %r10
jl .LBB2
.LBB3:
    leaq main.d.1(%rip), %r10
    xorl %r8d, %r8d
    xorl %r9d, %r9d
.LBB4:
    cmpl $1024, %r9d
jge .LBB6
.LBB5:
    leal 1(%r9), %edi
    leal 2(%r9), %ebx
    leal 3(%r9), %r12d
    leal 4(%r9), %r13d
    movslq %r9d, %rdx
    movzwl (%r11, %rdx, 2), %eax
    movl %eax, %ebp
    leal (%r8, %rbp), %esi
    movl %esi, (%r10, %rdx, 4)
    movslq %edi, %rdx
    movslq %r9d, %r9
    movzwl 2(%r11, %r9, 2), %eax
    addl %eax, %esi
    shlq $2, %rdx
    movl %esi, 4(%r10, %r9, 4)
    movslq %ebx, %rdx
    movzwl 4(%r11, %r9, 2), %eax
    addl %eax, %esi
    shlq $2, %rdx
    movl %esi, 8(%r10, %r9, 4)
    movslq %r12d, %rdx
    movzwl 6(%r11, %r9, 2), %edi
    leal (%rsi, %rdi), %r8d
    shlq $2, %rdx
    movl %r8d, 12(%r10, %r9, 4)
    movl %r13d, %r9d
    cmpl $1024, %r9d
jl .LBB5
.LBB6:
    movl (%r10), %esi
    addl main.d.1+2044(%rip), %esi
    addl main.d.1+4092(%rip), %esi
    movl %esi, %r11d
    xorl %r8d, %r8d
.LBB7:
    cmpq $1024, %r8
jge .LBB9
.LBB8:
    movq %r11, %r9
    shlq $5, %r9
    addq %r11, %r9
    movl (%r10), %edi
    leaq (%r9, %rdi, 1), %r11
    addq $7, %r8
    leaq 28(%r10), %r10
    cmpq $1024, %r8
jl .LBB8
.LBB9:
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


