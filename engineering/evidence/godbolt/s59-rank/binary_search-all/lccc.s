table:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $32, %rsp
    leaq table(%rip), %rdi
    leaq table(%rip), %rbx
    leaq table(%rip), %r12
    xorl %r8d, %r8d
.LBB1:
    cmpl $4096, %r8d
jl .LBB2
.LBB3:
    xorl %r13d, %r13d
    movq $-1000, %r11
    jmp .LBB4
.LBB2:
    movl %r8d, %r10d
    leaq (%r10, %r10, 2), %r10
    addl $7, %r10d
    movslq %r8d, %r8
    movl %r10d, (%rdi, %r8, 4)
    addl $1, %r8d
    cmpl $4096, %r8d
jl .LBB2
    jmp .LBB3
.LBB4:
    cmpl $13288, %r11d
jge .LBB16
.LBB5:
    xorl %r10d, %r10d
    movl $4095, %edi
.LBB6:
    cmpl %edi, %r10d
jle .LBB11
.LBB7:
    movq $-1, %r8
    jmp .LBB8
.LBB11:
    movl %edi, %r8d
    subl %r10d, %r8d
    sarl $1, %r8d
    leal (%r10, %r8), %edx
    movslq %edx, %rdx
    movl (%rbx, %rdx, 4), %esi
    cmpl %r11d, %esi
jne .LBB12
.LBB13:
    movq %rdx, %r8
    jmp .LBB8
.LBB12:
    leal 1(%rdx), %r8d
    subl $1, %edx
    cmpl %r11d, %esi
    cmovll %r8d, %r10d
    cmpl %r11d, %esi
    movq %rdi, %rcx
    movl %edx, %edi
    cmovll %ecx, %edi
    cmpl %edi, %r10d
jle .LBB11
    jmp .LBB7
.LBB8:
    testl %r8d, %r8d
jge .LBB14
.LBB9:
    movq %r13, %r10
    jmp .LBB10
.LBB14:
    movslq %r8d, %rdi
    cmpl %r11d, (%r12, %rdi, 4)
je .LBB15
.LBB17:
    movl $1, %eax
.L__lccc_epilogue_1:
    addq $32, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB15:
    movl %r8d, %edi
    leal (%r13, %rdi), %r10d
.LBB10:
    addl $7, %r11d
    movq %r10, %r13
    cmpl $13288, %r11d
jl .LBB5
.LBB16:
    cmpl $1198665, %r13d
    movl $0, %ecx
    movl $2, %esi
    cmovel %ecx, %esi
    movl %esi, %eax
    jmp .L__lccc_epilogue_1


