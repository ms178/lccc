.Lstr0:

k:

img:
out:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    leaq img+64(%rip), %rax
    movq %rax, 24(%rsp)
    xorl %r8d, %r8d
    leaq img(%rip), %r9
.LBB1:
    cmpl $64, %r8d
jge .LBB6
.LBB2:
    movl %r8d, %edi
    leaq (%rdi, %rdi, 4), %rdi
    xorl %esi, %esi
.LBB3:
    cmpl $64, %esi
jge .LBB5
.LBB4:
    movslq %esi, %rdx
    movl %esi, %r10d
    leaq (%r10, %r10, 2), %r10
    addl %edi, %r10d
    movl %esi, %r14d
    imull %r8d, %r14d
    addl %r14d, %r10d
    andl $255, %r10d
    movb %r10b, (%r9, %rdx)
    addl $1, %esi
    cmpl $64, %esi
jl .LBB4
.LBB5:
    addl $1, %r8d
    leaq 64(%r9), %r9
    cmpl $64, %r8d
jl .LBB2
.LBB6:
    movl $1, %ebp
    movq 24(%rsp), %r10
.LBB7:
    cmpq $63, %rbp
jge .LBB12
.LBB8:
    movq %rbp, %r8
    shlq $6, %r8
    leaq out(%rip), %r14
    addq %r8, %r14
    leaq 1(%rbp), %rdi
    shlq $6, %rdi
    leaq -1(%rbp), %rsi
    shlq $6, %rsi
    leaq img(%rip), %rdx
    addq %rdi, %rdx
    leaq img(%rip), %r8
    addq %rsi, %r8
    movl $1, %edi
.LBB9:
    cmpq $63, %rdi
jge .LBB11
.LBB10:
    movzbl -1(%r8, %rdi), %r11d
    imull k(%rip), %r11d
    leaq 1(%rdi), %r15
    movzbl (%r8, %rdi), %r13d
    movslq k+4(%rip), %rbx
    imull %r13d, %ebx
    addl %ebx, %r11d
    movzbl (%r8, %r15), %r13d
    imull k+8(%rip), %r13d
    addl %r13d, %r11d
    movzbl -1(%r10, %rdi), %ebx
    imull k+12(%rip), %ebx
    addl %ebx, %r11d
    movzbl (%r10, %rdi), %r13d
    movslq k+16(%rip), %rbx
    imull %r13d, %ebx
    addl %ebx, %r11d
    movzbl (%r10, %r15), %ebx
    imull k+20(%rip), %ebx
    addl %ebx, %r11d
    movzbl -1(%rdx, %rdi), %esi
    imull k+24(%rip), %esi
    addl %esi, %r11d
    movzbl (%rdx, %rdi), %esi
    movslq k+28(%rip), %rbx
    imull %esi, %ebx
    addl %ebx, %r11d
    movzbl (%rdx, %r15), %esi
    imull k+32(%rip), %esi
    addl %esi, %r11d
    movl %r11d, %ebx
    negl %ebx
    testl %r11d, %r11d
    movq %r11, %r9
    cmovll %ebx, %r9d
    cmpl $255, %r9d
    movl $255, %ecx
    movq %r9, %r11
    cmovgl %ecx, %r11d
    movb %r11b, (%r14, %rdi)
    movq %r15, %rdi
    cmpq $63, %r15
jl .LBB10
.LBB11:
    addq $1, %rbp
    leaq 64(%r10), %rax
    movq %rax, %r10
    cmpq $63, %rbp
jl .LBB8
.LBB12:
    xorl %r11d, %r11d
    xorl %edx, %edx
    leaq out(%rip), %r10
.LBB13:
    cmpq $64, %rdx
jge .LBB18
.LBB14:
    xorl %r8d, %r8d
    movq %r11, %r9
.LBB15:
    cmpq $64, %r8
jge .LBB17
.LBB16:
    movq %r9, %rdi
    shlq $5, %rdi
    subq %r9, %rdi
    movzbl (%r10, %r8), %esi
    leaq (%rdi, %rsi, 1), %r9
    addq $1, %r8
    cmpq $64, %r8
jl .LBB16
.LBB17:
    addq $1, %rdx
    leaq 64(%r10), %rax
    movq %r9, %r11
    movq %rax, %r10
    cmpq $64, %rdx
jl .LBB14
.LBB18:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


