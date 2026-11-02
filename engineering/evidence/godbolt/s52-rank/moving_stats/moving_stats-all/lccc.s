.Lstr0:

main.a.0:
main.mns.1:
main.mxs.2:
main.sums.3:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    leaq main.a.0(%rip), %r11
    xorl %r10d, %r10d
.LBB1:
    cmpl $1024, %r10d
jge .LBB3
.LBB2:
    imull $717, %r10d, %r8d
    addl $41, %r8d
    movslq %r8d, %r9
    imulq $1098962147, %r9, %r9
    sarq $41, %r9
    movq %r9, %rdi
    shrq $63, %rdi
    addq %rdi, %r9
    movl %r9d, %esi
    imull $2001, %esi, %esi
    subl %esi, %r8d
    subl $1000, %r8d
    movswl %r8w, %edx
    leaq 0(,%r10,2), %r8
    movslq %r10d, %r10
    movw %dx, (%r11, %r10, 2)
    addl $1, %r10d
    cmpl $1024, %r10d
jl .LBB2
.LBB3:
    leaq main.mns.1(%rip), %rbx
    leaq main.mxs.2(%rip), %r12
    leaq main.sums.3(%rip), %r13
    xorl %r10d, %r10d
    xorl %r14d, %r14d
.LBB4:
    leaq 16(%r10), %r8
    cmpq $1024, %r8
jg .LBB6
.LBB5:
    movswq (%r11, %r10, 2), %rsi
    movswq %si, %r9
    leaq 1(%r10), %r15
    movq %r15, %rax
    addq %r15, %rax
    movq %rax, 24(%rsp)
    movswq (%r11, %r15, 2), %r8
    movswq %r8w, %rbp
    addl %ebp, %r9d
    cmpw %si, %r8w
    movq %rsi, %rbp
    cmovll %r8d, %ebp
    cmpw %si, %r8w
    movq %rsi, %rax
    movq %r8, %rcx
    cmovgl %r8d, %eax
    movl %eax, 16(%rsp)
    movswq 4(%r11, %r10, 2), %rdi
    movswq %di, %rsi
    addl %esi, %r9d
    cmpw %bp, %di
    movq %rbp, %r8
    cmovll %edi, %r8d
    cmpw 16(%rsp), %di
    movswq 16(%rsp), %rsi
    cmovgl %edi, %esi
    movswq 6(%r11, %r10, 2), %rdx
    movswq %dx, %rdi
    addl %edi, %r9d
    cmpw %r8w, %dx
    movq %r8, %rdi
    cmovll %edx, %edi
    cmpw %si, %dx
    movq %rsi, %r8
    cmovgl %edx, %r8d
    movswq 8(%r11, %r10, 2), %rbp
    movswq %bp, %rsi
    addl %esi, %r9d
    cmpw %di, %bp
    movq %rdi, %rdx
    cmovll %ebp, %edx
    cmpw %r8w, %bp
    movq %r8, %rdi
    cmovgl %ebp, %edi
    movswq 10(%r11, %r10, 2), %rbp
    movswq %bp, %rsi
    addl %esi, %r9d
    cmpw %dx, %bp
    movq %rdx, %r8
    cmovll %ebp, %r8d
    cmpw %di, %bp
    movq %rdi, %rsi
    cmovgl %ebp, %esi
    movswq 12(%r11, %r10, 2), %rbp
    movswq %bp, %rdx
    addl %edx, %r9d
    cmpw %r8w, %bp
    movq %r8, %rdi
    cmovll %ebp, %edi
    cmpw %si, %bp
    movq %rsi, %rdx
    cmovgl %ebp, %edx
    movswq 14(%r11, %r10, 2), %rbp
    movswq %bp, %r8
    addl %r8d, %r9d
    cmpw %di, %bp
    movq %rdi, %rsi
    cmovll %ebp, %esi
    cmpw %dx, %bp
    movq %rdx, %r8
    cmovgl %ebp, %r8d
    movswq 16(%r11, %r10, 2), %rbp
    movswq %bp, %rdi
    addl %edi, %r9d
    cmpw %si, %bp
    movq %rsi, %rdx
    cmovll %ebp, %edx
    cmpw %r8w, %bp
    movq %r8, %rdi
    cmovgl %ebp, %edi
    movswq 18(%r11, %r10, 2), %rbp
    movswq %bp, %rsi
    addl %esi, %r9d
    cmpw %dx, %bp
    movq %rdx, %r8
    cmovll %ebp, %r8d
    cmpw %di, %bp
    movq %rdi, %rsi
    cmovgl %ebp, %esi
    movswq 20(%r11, %r10, 2), %rbp
    movswq %bp, %rdx
    addl %edx, %r9d
    cmpw %r8w, %bp
    movq %r8, %rdi
    cmovll %ebp, %edi
    cmpw %si, %bp
    movq %rsi, %rdx
    cmovgl %ebp, %edx
    movswq 22(%r11, %r10, 2), %rbp
    movswq %bp, %r8
    addl %r8d, %r9d
    cmpw %di, %bp
    movq %rdi, %rsi
    cmovll %ebp, %esi
    cmpw %dx, %bp
    movq %rdx, %r8
    cmovgl %ebp, %r8d
    movswq 24(%r11, %r10, 2), %rbp
    movswq %bp, %rdi
    addl %edi, %r9d
    cmpw %si, %bp
    movq %rsi, %rdx
    cmovll %ebp, %edx
    cmpw %r8w, %bp
    movq %r8, %rdi
    cmovgl %ebp, %edi
    movswq 26(%r11, %r10, 2), %rbp
    movswq %bp, %rsi
    addl %esi, %r9d
    cmpw %dx, %bp
    movq %rdx, %r8
    cmovll %ebp, %r8d
    cmpw %di, %bp
    movq %rdi, %rsi
    cmovgl %ebp, %esi
    movswq 28(%r11, %r10, 2), %rbp
    movswq %bp, %rdx
    addl %edx, %r9d
    cmpw %r8w, %bp
    movq %r8, %rdi
    cmovll %ebp, %edi
    cmpw %si, %bp
    movq %rsi, %rdx
    cmovgl %ebp, %edx
    movswq 30(%r11, %r10, 2), %rbp
    movswq %bp, %r8
    addl %r8d, %r9d
    cmpw %di, %bp
    movq %rdi, %rsi
    cmovll %ebp, %esi
    cmpw %dx, %bp
    movq %rdx, %r8
    cmovgl %ebp, %r8d
    movl %r9d, (%r13, %r10, 4)
    movw %si, (%rbx, %r14)
    movw %r8w, (%r12, %r14)
    movq %r15, %r10
    movq 24(%rsp), %r14
    leaq 16(%r10), %rsi
    cmpq $1024, %rsi
jle .LBB5
.LBB6:
    xorl %r11d, %r11d
    xorl %edx, %edx
.LBB7:
    leaq 16(%rdx), %r10
    cmpq $1024, %r10
jg .LBB9
.LBB8:
    movq %r11, %r8
    shlq $5, %r8
    subq %r11, %r8
    movl (%r13, %rdx, 4), %eax
    addq %rax, %r8
    imulq $31, %r8, %r8
    movswq (%rbx, %rdx, 2), %rax
    movswq %ax, %rdi
    leal 1000(%rdi), %eax
    addq %rax, %r8
    imulq $31, %r8, %r8
    movswq (%r12, %rdx, 2), %rax
    movswq %ax, %rdi
    leal 1000(%rdi), %esi
    leaq (%r8, %rsi, 1), %r11
    addq $1, %rdx
    leaq 16(%rdx), %r10
    cmpq $1024, %r10
jle .LBB8
.LBB9:
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


