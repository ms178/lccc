bytes:
bins:

main:
    pushq %rbx
    pushq %r12
    pushq %r14
    pushq %rbp
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
    cmpl $262144, %esi
jae .LBB6
.LBB5:
    leal 4(%rsi), %ebx
    movl %esi, %r12d
    movzbl (%rdi, %r12), %r14d
    shlq $3, %r14
    leaq bins(%rip), %r11
    addq %r14, %r11
    movq (%r11), %rax
    addq $1, %rax
    movq %rax, (%r11)
    movzbl 1(%rdi, %rsi), %r11d
    shlq $3, %r11
    leaq bins(%rip), %rdx
    addq %r11, %rdx
    movq (%rdx), %rbp
    leaq 1(%rbp), %rax
    movq %rax, (%rdx)
    movzbl 2(%rdi, %rsi), %r8d
    shlq $3, %r8
    leaq bins(%rip), %r11
    addq %r8, %r11
    movq (%r11), %rax
    addq $1, %rax
    movq %rax, (%r11)
    movzbl 3(%rdi, %rsi), %r11d
    shlq $3, %r11
    leaq bins(%rip), %rdx
    addq %r11, %rdx
    movq (%rdx), %rax
    addq $1, %rax
    movq %rax, (%rdx)
    movl %ebx, %esi
    cmpl $262144, %esi
jb .LBB5
.LBB6:
    leaq bins(%rip), %r9
    xorl %r11d, %r11d
    xorl %r10d, %r10d
    xorl %edi, %edi
.LBB7:
    cmpq $256, %r10
jae .LBB9
.LBB8:
    movq (%r9, %r10, 8), %r8
    addq %r8, %rdi
    imulq $131, %r11, %rsi
    leaq (%rsi, %r8, 1), %r11
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
    popq %rbp
    popq %r14
    popq %r12
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


