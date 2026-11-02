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
    movl 24(%rsp), %eax
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
    movl 16(%rsp), %r9d
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
    movq %rdi, %rsi
    subq $7, %rsi
    movq %rdx, %r8
.LBB8:
    cmpq %rsi, %r8
jb .LBB16
.LBB9:
    movq %r8, %rsi
    movq %rbx, %r9
    jmp .LBB10
.LBB16:
    movq (%rbx), %r9
    xorq (%r8), %r9
jne .LBB18
.LBB17:
    addq $8, %r8
    addq $8, %rbx
    cmpq %rsi, %r8
jb .LBB16
    jmp .LBB9
.LBB18:
    xorl %edi, %edi
    tzcntq %r9, %rdi
    movq %rdi, %rax
    sarl $3, %eax
    cltq
    movslq %eax, %rsi
    addq %rsi, %r8
    subq %rdx, %r8
    movl %r8d, %r10d
    jmp .LBB14
.LBB10:
    cmpq %rdi, %rsi
jae .LBB13
.LBB11:
    movzbl (%rsi), %r8d
    movzbl (%r9), %r10d
    cmpl %r10d, %r8d
jne .LBB13
.LBB12:
    addq $1, %rsi
    addq $1, %r9
    cmpq %rdi, %rsi
jb .LBB11
.LBB13:
    subq %rdx, %rsi
    movl %esi, %r10d
.LBB14:
    movl %r10d, %r9d
    movl %r14d, %edi
    andl $7, %edi
    shll $3, %edi
    shlxq %rdi, %r9, %r9
    movl %r12d, %esi
    xorq %rsi, %r9
    addq %r9, %r13
    addl $1, %r14d
    movq %rbp, 16(%rsp)
    cmpl $131072, %r14d
jb .LBB7
.LBB15:
    movl 24(%rsp), %eax
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
