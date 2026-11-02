ring:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    leaq ring(%rip), %rdi
    xorl %esi, %esi
    xorl %edx, %edx
    xorl %r8d, %r8d
    xorl %r9d, %r9d
    movl $1, %r11d
.LBB1:
    cmpl $20000, %r8d
jge .LBB9
.LBB2:
    movl %esi, %r10d
    subl %r9d, %r10d
    andl $1023, %r10d
    cmpl $1023, %r10d
jne .LBB4
.LBB3:
    movq %rsi, %rbx
    movq %r11, %r12
    jmp .LBB5
.LBB4:
    imull $1664525, %r11d, %r11d
    leal 1013904223(%r11), %r12d
    movl %esi, %r10d
    andl $1023, %r10d
    movl %r10d, %r13d
    shlq $2, %r13
    movl %r12d, (%rdi, %r10, 4)
    leal 1(%rsi), %ebx
.LBB5:
    cmpl %r9d, %ebx
jne .LBB7
.LBB6:
    movq %r9, %r10
    movq %rdx, %r15
    jmp .LBB8
.LBB7:
    movl %r9d, %esi
    andl $1023, %esi
    movl (%rdi, %rsi, 4), %r13d
    leal (%rdx, %r13), %r15d
    leal 1(%r9), %r10d
.LBB8:
    addl $1, %r8d
    movq %rbx, %rsi
    movq %r15, %rdx
    movq %r10, %r9
    movq %r12, %r11
    cmpl $20000, %r8d
jl .LBB2
.LBB9:
    cmpl $20000, %esi
jne .LBB11
.LBB10:
    cmpl $20000, %r9d
je .LBB12
.LBB11:
    movl $1, %eax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB12:
    cmpl $1920339856, %edx
    movl $0, %ecx
    movl $2, %r10d
    cmovel %ecx, %r10d
    movl %r10d, %eax
    jmp .L__lccc_epilogue_1


