.Lstr0:

table:

insert:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $32, %rsp
    movq %rdi, %rbx
    movq %rsi, %r12
    movl %edi, %r11d
    shrl $16, %r11d
    movl %edi, %r10d
    xorl %r11d, %r10d
    imull $73244475, %r10d, %r10d
    movl %r10d, %r8d
    shrl $16, %r8d
    xorl %r8d, %r10d
    imull $73244475, %r10d, %r10d
    movl %r10d, %r9d
    shrl $16, %r9d
    xorl %r9d, %r10d
    movzwl %r10w, %r13d
    leaq table(%rip), %rcx
    movq (%rcx, %r13, 8), %rdx
.LBB1:
    testq %rdx, %rdx
je .LBB4
.LBB2:
    cmpl %ebx, (%rdx)
jne .LBB3
.LBB5:
    movl %r12d, 4(%rdx)
.L__lccc_epilogue_1:
    addq $32, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB3:
    movq 8(%rdx), %rdx
    testq %rdx, %rdx
jne .LBB2
.LBB4:
    movl $16, %edi
    call malloc@PLT
    movq %rax, %r9
    movl %ebx, (%r9)
    movl %r12d, 4(%r9)
    movl %r13d, %edx
    shlq $3, %rdx
    leaq table(%rip), %r11
    addq %rdx, %r11
    movq (%r11), %rax
    movq %rax, 8(%r9)
    movq %r9, (%r11)
    jmp .L__lccc_epilogue_1

lookup:
    movl %edi, %esi
    shrl $16, %esi
    movl %edi, %edx
    xorl %esi, %edx
    imull $73244475, %edx, %edx
    movl %edx, %r8d
    shrl $16, %r8d
    xorl %r8d, %edx
    imull $73244475, %edx, %edx
    movl %edx, %r9d
    shrl $16, %r9d
    xorl %r9d, %edx
    andl $65535, %edx
    leaq table(%rip), %rcx
    movq (%rcx, %rdx, 8), %rsi
.LBB7:
    testq %rsi, %rsi
je .LBB10
.LBB8:
    cmpl %edi, (%rsi)
jne .LBB9
.LBB11:
    movslq 4(%rsi), %r9
    movl %r9d, %eax
    ret
.LBB9:
    movq 8(%rsi), %rsi
    testq %rsi, %rsi
jne .LBB8
.LBB10:
    movl $-1, %eax
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    movl $12345, %ebx
    xorl %r12d, %r12d
.LBB13:
    cmpl $2000000, %r12d
jge .LBB15
.LBB14:
    imull $1664525, %ebx, %r11d
    leal 1013904223(%r11), %ebx
    movq %rbx, %rdi
    movq %r12, %rsi
    call insert
    addl $1, %r12d
    cmpl $2000000, %r12d
jl .LBB14
.LBB15:
    movl $12345, %r14d
    xorl %r15d, %r15d
    xorl %ebp, %ebp
.LBB16:
    cmpl $2000000, %ebp
jge .LBB18
.LBB17:
    imull $1664525, %r14d, %r10d
    leal 1013904223(%r10), %r14d
    movl %r14d, %edi
    call lookup
    movl %eax, %r8d
    movslq %r8d, %r9
    addq %r9, %r15
    addl $1, %ebp
    cmpl $2000000, %ebp
jl .LBB17
.LBB18:
    movq %r14, %rdi
    movq %r15, %r12
    xorl %r13d, %r13d
.LBB19:
    cmpl $2000000, %r13d
jge .LBB24
.LBB20:
    imull $1664525, %edi, %edi
    leal 1013904223(%rdi), %r14d
    movl %r13d, %esi
    andl $1, %esi
je .LBB23
.LBB21:
    movq %r14, %rdi
    movq %r13, %rsi
    call insert
    movq %r12, %r15
.LBB22:
    addl $1, %r13d
    movq %r14, %rdi
    movq %r15, %r12
    cmpl $2000000, %r13d
jl .LBB20
    jmp .LBB24
.LBB23:
    movl %r14d, %edi
    call lookup
    movl %eax, %edx
    movslq %edx, %r11
    leaq (%r12, %r11, 1), %r15
    jmp .LBB22
.LBB24:
    leaq .Lstr0(%rip), %rdi
    movq %r12, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


