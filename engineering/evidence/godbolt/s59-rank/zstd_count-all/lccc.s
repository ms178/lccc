.Lstr0:

buffer:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    leaq buffer(%rip), %r10
    xorl %r8d, %r8d
    movl $2235986729, %r9d
.LBB1:
    cmpl $1048576, %r8d
jb .LBB2
.LBB20:
    movq $0, 24(%rsp)
    xorl %r11d, %r11d
    movl $324508639, %esi
    jmp .LBB21
.LBB2:
    imull $1664525, %r9d, %r9d
    leal 1013904223(%r9), %edx
    testb $7, %dl
jne .LBB19
    cmpl $64, %r8d
jb .LBB19
.LBB3:
    leal -64(%r8), %r9d
    movl %edx, %edi
    shrl $28, %edi
    addl %edi, %r9d
    movzbl (%r10, %r9), %edi
    jmp .LBB4
.LBB19:
    movl %edx, %esi
    shrl $24, %esi
    movzbl %sil, %edi
.LBB4:
    movl %r8d, %esi
    movb %dil, (%r10, %rsi)
    addl $1, %r8d
    movq %rdx, %r9
    cmpl $1048576, %r8d
jb .LBB2
    jmp .LBB20
.LBB21:
    movq 24(%rsp), %rax
    cmpl $16, %eax
jae .LBB22
.LBB5:
    movq %r11, %r13
    xorl %r14d, %r14d
    movq %rsi, 16(%rsp)
.LBB6:
    cmpl $131072, %r14d
jae .LBB15
.LBB7:
    movq 16(%rsp), %r9
    imull $1664525, %r9d, %r9d
    movq %r9, %rbp
    addl $1013904223, %ebp
    movl %ebp, %r12d
    andl $1048319, %r12d
    movl %ebp, %r8d
    shrl $12, %r8d
    andl $1048319, %r8d
    movl %ebp, %edi
    andl $127, %edi
    addl $16, %edi
    leaq buffer(%rip), %rdx
    addq %r12, %rdx
    leaq buffer(%rip), %rbx
    addq %r8, %rbx
    movl %edi, %r15d
    leaq (%rdx, %r15, 1), %rdi
    movq %rdi, %r8
    subq $7, %r8
    movq %rdx, %rsi
.LBB8:
    cmpq %r8, %rsi
jb .LBB16
.LBB9:
    movq %rsi, %r8
    movq %rbx, %r9
    jmp .LBB10
.LBB16:
    movq (%rbx), %r9
    xorq (%rsi), %r9
jne .LBB18
.LBB17:
    addq $8, %rsi
    addq $8, %rbx
    cmpq %r8, %rsi
jb .LBB16
    jmp .LBB9
.LBB18:
    tzcntq %r9, %rdi
    movq %rdi, %rax
    sarl $3, %eax
    cltq
    movslq %eax, %r10
    addq %r10, %rsi
    subq %rdx, %rsi
    movl %esi, %r9d
    jmp .LBB14
.LBB10:
    cmpq %rdi, %r8
jae .LBB13
.LBB11:
    movzbl (%r8), %esi
    movzbl (%r9), %r10d
    cmpl %r10d, %esi
jne .LBB13
.LBB12:
    addq $1, %r8
    leaq 1(%r9), %rax
    movq %rax, %r9
    cmpq %rdi, %r8
jb .LBB11
.LBB13:
    subq %rdx, %r8
    movl %r8d, %r9d
.LBB14:
    movl %r9d, %edi
    movl %r14d, %esi
    andl $7, %esi
    shll $3, %esi
    shlxq %rsi, %rdi, %rdi
    movl %r12d, %edx
    xorq %rdx, %rdi
    addq %rdi, %r13
    addl $1, %r14d
    movq %rbp, 16(%rsp)
    cmpl $131072, %r14d
jb .LBB7
.LBB15:
    movq 24(%rsp), %rax
    addl $1, %eax
    movq %rax, 24(%rsp)
    movq %r13, %r11
    movq 16(%rsp), %rsi
    cmpl $16, %eax
jb .LBB5
.LBB22:
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


