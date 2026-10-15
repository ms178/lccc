bytes:
bins:

main:
    pushq %rbx
    pushq %r14
    subq $24, %rsp
    leaq bytes(%rip), %rdi
    xorl %esi, %esi
.LBB1:
    cmpl $262144, %esi
jae .LBB3
.LBB2:
    imull $-1640531535, %esi, %edx
    shrl $24, %edx
    movl %esi, %r8d
    movb %dl, (%rdi, %r8)
    addl $1, %esi
    cmpl $262144, %esi
jb .LBB2
.LBB3:
    leaq bytes(%rip), %rdi
    xorl %esi, %esi
.LBB4:
    cmpq $262144, %rsi
jae .LBB6
.LBB5:
    leaq 1(%rsi), %rdx
    leaq 2(%rsi), %r8
    leaq 3(%rsi), %rbx
    movzbl (%rdi, %rsi), %eax
    leaq bins(%rip), %r9
    addq $1, (%r9, %rax, 8)
    movzbl (%rdi, %rdx), %eax
    addq $1, (%r9, %rax, 8)
    movzbl (%rdi, %r8), %eax
    addq $1, (%r9, %rax, 8)
    movzbl (%rdi, %rbx), %eax
    movq %r9, %rdx
    leaq (%rdx, %rax, 8), %rdx
    movq (%rdx), %r8
    leaq 1(%r8), %rax
    movq %rax, (%rdx)
    addq $4, %rsi
    cmpq $262144, %rsi
jb .LBB5
.LBB6:
    leaq bins(%rip), %r14
    xorl %r11d, %r11d
    xorl %r10d, %r10d
    xorl %edi, %edi
.LBB7:
    cmpq $256, %r10
jae .LBB9
.LBB8:
    movq (%r14, %r10, 8), %r9
    addq %r9, %rdi
    imulq $131, %r11, %rsi
    leaq (%rsi, %r9, 1), %r11
    addq $1, %r10
    cmpq $256, %r10
jb .LBB8
.LBB9:
    cmpq $262144, %rdi
je .LBB11
.LBB10:
    movl $1, %eax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %r14
    popq %rbx
    ret
.LBB11:
    movabsq $-7388273516099151790, %rcx
    cmpq %rcx, %r11
    movl $0, %ecx
    movl $2, %edx
    cmovel %ecx, %edx
    movl %edx, %eax
    jmp .L__lccc_epilogue_1


