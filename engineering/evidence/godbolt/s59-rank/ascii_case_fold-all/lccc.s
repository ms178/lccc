data:
folded:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    leaq data(%rip), %rdi
    leaq folded(%rip), %rsi
    xorl %edx, %edx
.LBB1:
    cmpl $65536, %edx
jae .LBB3
.LBB2:
    imull $1103515245, %edx, %r8d
    addl $12345, %r8d
    shrl $16, %r8d
    movl %edx, %ebx
    movl %r8d, %r10d
    imulq $1491936009, %r10, %r12
    movq %r12, %r9
    shrq $32, %r9
    subq %r9, %r10
    shrq $1, %r10
    addq %r9, %r10
    shrq $6, %r10
    movl %r10d, %r9d
    imull $95, %r9d, %r9d
    subl %r9d, %r8d
    addl $32, %r8d
    movb %r8b, (%rdi, %rbx)
    addl $1, %edx
    cmpl $65536, %edx
jb .LBB2
.LBB3:
    xorl %edx, %edx
    xorl %r8d, %r8d
.LBB4:
    cmpq $65536, %r8
jae .LBB6
.LBB5:
    leaq 1(%r8), %r13
    leaq 2(%r8), %r14
    leaq 3(%r8), %r15
    leaq 4(%r8), %rbp
    movzbl (%rdi, %r8), %r11d
    movl %r11d, %r9d
    cmpb $65, %r11b
    setae %r12b
    movzbl %r12b, %r12d
    cmpb $90, %r11b
    setbe %bl
    movzbl %bl, %ebx
    movl %r12d, %r11d
    andl %ebx, %r11d
    leal 32(%r9), %r12d
    movl %r9d, %ebx
    cmovnel %r12d, %ebx
    movb %bl, (%rsi, %r8)
    movl %edx, %r10d
    shll $5, %r10d
    addl %edx, %r10d
    leal (%r10, %rbx), %r12d
    movq %r13, %r9
    movzbl (%rdi, %r9), %r10d
    movl %r10d, %r11d
    cmpb $65, %r10b
    setae %r13b
    movzbl %r13b, %r13d
    cmpb $90, %r10b
    setbe %bl
    movzbl %bl, %ebx
    movl %r13d, %r10d
    andl %ebx, %r10d
    leal 32(%r11), %r13d
    movl %r11d, %ebx
    cmovnel %r13d, %ebx
    movb %bl, (%rsi, %r9)
    movl %r12d, %r9d
    shll $5, %r9d
    addl %r12d, %r9d
    leal (%r9, %rbx), %r12d
    movq %r14, %r11
    movzbl (%rdi, %r11), %r9d
    movl %r9d, %r10d
    cmpb $65, %r9b
    setae %r13b
    movzbl %r13b, %r13d
    cmpb $90, %r9b
    setbe %r14b
    movzbl %r14b, %r14d
    movl %r13d, %r9d
    andl %r14d, %r9d
    leal 32(%r10), %ebx
    movl %r10d, %r13d
    cmovnel %ebx, %r13d
    movb %r13b, (%rsi, %r11)
    movl %r12d, %r11d
    shll $5, %r11d
    addl %r12d, %r11d
    leal (%r11, %r13), %r14d
    movq %r15, %r10
    movzbl (%rdi, %r10), %r11d
    movl %r11d, %r9d
    cmpb $65, %r11b
    setae %r15b
    movzbl %r15b, %r15d
    cmpb $90, %r11b
    setbe %bl
    movzbl %bl, %ebx
    andl %ebx, %r15d
    leal 32(%r9), %r12d
    movl %r9d, %r13d
    cmovnel %r12d, %r13d
    movb %r13b, (%rsi, %r10)
    movl %r14d, %r10d
    shll $5, %r10d
    addl %r14d, %r10d
    leal (%r10, %r13), %edx
    movq %rbp, %r8
    cmpq $65536, %r8
jb .LBB5
.LBB6:
    movzbl folded(%rip), %edi
    cmpb $32, %dil
jne .LBB9
.LBB7:
    movzbl folded+1(%rip), %esi
    cmpb $55, %sil
jne .LBB9
.LBB8:
    movzbl folded+2(%rip), %r8d
    cmpb $110, %r8b
je .LBB10
.LBB9:
    movl $1, %eax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB10:
    cmpl $2571300113, %edx
    movl $0, %ecx
    movl $2, %r9d
    cmovel %ecx, %r9d
    movl %r9d, %eax
    jmp .L__lccc_epilogue_1


