.Lstr0:

sat_kernel:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    subq $192, %rsp
    movq %rdi, %r12
    movq %rsi, %r13
    movq %rdx, %r14
    movq %rcx, %rbx
    movq %r8, %r15
    xorl %r11d, %r11d
.LBB1:
    cmpl %r15d, %r11d
jae .LBB3
.LBB2:
    movdqu (%r12), %xmm0
    movdqu (%r13), %xmm1
    vpaddusb %xmm1, %xmm0, %xmm3
    movdqu (%r14), %xmm1
    vpaddusb %xmm1, %xmm3, %xmm4
    vpavgb %xmm4, %xmm3, %xmm0
    vpsubq %xmm3, %xmm0, %xmm0
    vpxor %xmm4, %xmm0, %xmm0
    movdqu %xmm0, 16(%rsp)
    movdqu %xmm0, (%rbx)
    addl $1, %r11d
    cmpl %r15d, %r11d
jb .LBB2
.LBB3:
    movq (%rbx), %rax
    movq %rax, 56(%rsp)
    movq 8(%rbx), %rax
    movq 56(%rsp), %r8
    movq %rax, %r9
    xorq %rax, %r8
    movq %r8, %rax
    addq $192, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

main:
    pushq %rbx
    pushq %r12
    subq $88, %rsp
    movq %rdi, %rbx
    movq %rsi, %r12
    cmpl $1, %edi
jg .LBB6
.LBB5:
    movl $1000, %r11d
    jmp .LBB10
.LBB6:
    movq 8(%r12), %r8
    movq %r8, %rdi
    xorl %esi, %esi
    xorl %edx, %edx
    call strtoul@PLT
    movq %rax, %r9
    testq %rax, %rax
je .LBB8
.LBB7:
    movl $4294967295, %ecx
    cmpq %rcx, %r9
jbe .LBB9
.LBB8:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $88, %rsp
    popq %r12
    popq %rbx
    ret
.LBB9:
    movl %r9d, %r11d
.LBB10:
    movb $3, 64(%rsp)
    movb $5, 48(%rsp)
    movb $9, 32(%rsp)
    movb $20, 65(%rsp)
    movb $16, 49(%rsp)
    movb $16, 33(%rsp)
    movb $37, 66(%rsp)
    movb $27, 50(%rsp)
    movb $23, 34(%rsp)
    movb $54, 67(%rsp)
    movb $38, 51(%rsp)
    movb $30, 35(%rsp)
    movb $71, 68(%rsp)
    movb $49, 52(%rsp)
    movb $37, 36(%rsp)
    movb $88, 69(%rsp)
    movb $60, 53(%rsp)
    movb $44, 37(%rsp)
    movb $105, 70(%rsp)
    movb $71, 54(%rsp)
    movb $51, 38(%rsp)
    movb $122, 71(%rsp)
    movb $82, 55(%rsp)
    movb $58, 39(%rsp)
    movb $139, 72(%rsp)
    movb $93, 56(%rsp)
    movb $65, 40(%rsp)
    movb $156, 73(%rsp)
    movb $104, 57(%rsp)
    movb $72, 41(%rsp)
    movb $173, 74(%rsp)
    movb $115, 58(%rsp)
    movb $79, 42(%rsp)
    movb $190, 75(%rsp)
    movb $126, 59(%rsp)
    movb $86, 43(%rsp)
    movb $207, 76(%rsp)
    movb $137, 60(%rsp)
    movb $93, 44(%rsp)
    movb $224, 77(%rsp)
    movb $148, 61(%rsp)
    movb $100, 45(%rsp)
    movb $241, 78(%rsp)
    movb $159, 62(%rsp)
    movb $107, 46(%rsp)
    movb $2, 79(%rsp)
    movb $170, 63(%rsp)
    movb $114, 47(%rsp)
    leaq 64(%rsp), %rdi
    leaq 48(%rsp), %rsi
    leaq 32(%rsp), %rdx
    leaq 16(%rsp), %rcx
    movl %r11d, %r8d
    call sat_kernel
    movq %rax, %r10
    leaq .Lstr0(%rip), %rdi
    movq %rax, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1


