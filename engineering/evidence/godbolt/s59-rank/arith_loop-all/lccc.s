.Lstr0:

arith_loop:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $280, %rsp
    movq %rdi, %rbx
    movl $1, %esi
    movl $32, %edx
    movl $29, %r8d
    movl $26, %r9d
    movl $23, %r11d
    movl $20, %r10d
    movl $17, %r12d
    movl $14, %r13d
    movl $11, %r14d
    movl $8, %r15d
    movl $5, %ebp
    movq $2, 264(%rsp)
    movq $0, 256(%rsp)
    movq $30, 248(%rsp)
    movq $27, 240(%rsp)
    movq $24, 232(%rsp)
    movq $21, 224(%rsp)
    movq $18, 216(%rsp)
    movq $15, 208(%rsp)
    movq $12, 200(%rsp)
    movq $9, 192(%rsp)
    movq $6, 184(%rsp)
    movq $3, 176(%rsp)
    movq $31, 168(%rsp)
    movq $28, 160(%rsp)
    movq $25, 152(%rsp)
    movq $22, 144(%rsp)
    movq $19, 136(%rsp)
    movq $16, 128(%rsp)
    movq $13, 120(%rsp)
    movq $10, 112(%rsp)
    movq $7, 104(%rsp)
    movq $4, 96(%rsp)
.LBB1:
    movl $0, %eax
    cmpl %ebx, %eax
jge .LBB3
.LBB2:
    movl 264(%rsp), %edi
    imull 176(%rsp), %edi
    addl %edi, %esi
    movl 176(%rsp), %edi
    imull 96(%rsp), %edi
    addl %edi, 264(%rsp)
    movl 96(%rsp), %eax
    movq %rbp, %rcx
    imull %ecx, %eax
    addl %eax, 176(%rsp)
    movl %ebp, %edi
    imull 184(%rsp), %edi
    addl %edi, 96(%rsp)
    movl 184(%rsp), %edi
    imull 104(%rsp), %edi
    addl %edi, %ebp
    movl 104(%rsp), %eax
    imull %r15d, %eax
    addl %eax, 184(%rsp)
    movl %r15d, %edi
    imull 192(%rsp), %edi
    addl %edi, 104(%rsp)
    movl 192(%rsp), %edi
    imull 112(%rsp), %edi
    addl %edi, %r15d
    movl 112(%rsp), %eax
    imull %r14d, %eax
    addl %eax, 192(%rsp)
    movl %r14d, %edi
    imull 200(%rsp), %edi
    addl %edi, 112(%rsp)
    movl 200(%rsp), %edi
    imull 120(%rsp), %edi
    addl %edi, %r14d
    movl 120(%rsp), %eax
    imull %r13d, %eax
    addl %eax, 200(%rsp)
    movl %r13d, %edi
    imull 208(%rsp), %edi
    addl %edi, 120(%rsp)
    movl 208(%rsp), %edi
    imull 128(%rsp), %edi
    addl %edi, %r13d
    movl 128(%rsp), %eax
    imull %r12d, %eax
    addl %eax, 208(%rsp)
    movl %r12d, %edi
    imull 216(%rsp), %edi
    addl %edi, 128(%rsp)
    movl 216(%rsp), %edi
    imull 136(%rsp), %edi
    addl %edi, %r12d
    movl 136(%rsp), %eax
    imull %r10d, %eax
    addl %eax, 216(%rsp)
    movl %r10d, %edi
    imull 224(%rsp), %edi
    addl %edi, 136(%rsp)
    movl 224(%rsp), %edi
    imull 144(%rsp), %edi
    addl %edi, %r10d
    movl 144(%rsp), %eax
    imull %r11d, %eax
    addl %eax, 224(%rsp)
    movl %r11d, %edi
    imull 232(%rsp), %edi
    addl %edi, 144(%rsp)
    movl 232(%rsp), %edi
    imull 152(%rsp), %edi
    addl %edi, %r11d
    movl 152(%rsp), %eax
    imull %r9d, %eax
    addl %eax, 232(%rsp)
    movl %r9d, %edi
    imull 240(%rsp), %edi
    addl %edi, 152(%rsp)
    movl 240(%rsp), %edi
    imull 160(%rsp), %edi
    addl %edi, %r9d
    movl 160(%rsp), %eax
    imull %r8d, %eax
    addl %eax, 240(%rsp)
    movl %r8d, %edi
    imull 248(%rsp), %edi
    addl %edi, 160(%rsp)
    movl 248(%rsp), %edi
    imull 168(%rsp), %edi
    addl %edi, %r8d
    movl 168(%rsp), %eax
    imull %edx, %eax
    addl %eax, 248(%rsp)
    movl %edx, %edi
    imull %esi, %edi
    addl %edi, 168(%rsp)
    movl %esi, %edi
    imull 264(%rsp), %edi
    addl %edi, %edx
    movl 256(%rsp), %eax
    addl $1, %eax
    movq %rax, 256(%rsp)
    cmpl %ebx, %eax
jl .LBB2
.LBB3:
    xorl 264(%rsp), %esi
    xorl 176(%rsp), %esi
    xorl 96(%rsp), %esi
    xorl %ebp, %esi
    xorl 184(%rsp), %esi
    xorl 104(%rsp), %esi
    xorl %r15d, %esi
    xorl 192(%rsp), %esi
    xorl 112(%rsp), %esi
    xorl %r14d, %esi
    xorl 200(%rsp), %esi
    xorl 120(%rsp), %esi
    xorl %r13d, %esi
    xorl 208(%rsp), %esi
    xorl 128(%rsp), %esi
    xorl %r12d, %esi
    xorl 216(%rsp), %esi
    xorl 136(%rsp), %esi
    xorl %r10d, %esi
    xorl 224(%rsp), %esi
    xorl 144(%rsp), %esi
    xorl %r11d, %esi
    xorl 232(%rsp), %esi
    xorl 152(%rsp), %esi
    xorl %r9d, %esi
    xorl 240(%rsp), %esi
    xorl 160(%rsp), %esi
    xorl %r8d, %esi
    xorl 248(%rsp), %esi
    xorl 168(%rsp), %esi
    xorl %edx, %esi
    movl %esi, %eax
    addq $280, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

main:
    subq $24, %rsp
    movl $10000000, %edi
    call arith_loop@PLT
    movl %eax, 16(%rsp)
    movl 16(%rsp), %r11d
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    ret


