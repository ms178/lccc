.Lstr0:
.Lstr1:
.Lstr2:

make:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $16, %rsp
    movq %rdi, %rbx
    movl $16, %edi
    call malloc@PLT
    movq %rax, %r12
    testl %ebx, %ebx
jle .LBB2
.LBB1:
    subl $1, %ebx
    movl %ebx, %edi
    call make
    movq %rax, (%r12)
    movl %ebx, %edi
    call make
    movq %rax, 8(%r12)
    jmp .LBB3
.LBB2:
    movq $0, 8(%r12)
    movq $0, (%r12)
.LBB3:
    movq %r12, %rax
    addq $16, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret

check:
    pushq %rbx
    pushq %r12
    subq $24, %rsp
    movq %rdi, %rbx
    movq (%rbx), %r11
    testq %r11, %r11
jne .LBB6
.LBB5:
    movl $1, %eax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %r12
    popq %rbx
    ret
.LBB6:
    movq %r11, %rdi
    call check
    movl %eax, %r10d
    leal 1(%r10), %r12d
    movq 8(%rbx), %r9
    movq %r9, %rdi
    call check
    movl %eax, %edi
    leal (%r12, %rdi), %eax
    jmp .L__lccc_epilogue_1

destroy:
    pushq %rbx
    subq $16, %rsp
    movq %rdi, %rbx
    movq (%rbx), %r11
    testq %r11, %r11
je .LBB9
.LBB8:
    movq %r11, %rdi
    call destroy
    movq 8(%rbx), %rdi
    call destroy
.LBB9:
    movq %rbx, %rdi
    call free@PLT
    addq $16, %rsp
    popq %rbx
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    movl $19, %edi
    call make
    movq %rax, %rbx
    movq %rax, %rdi
    call check
    movl %eax, %r11d
    leaq .Lstr0(%rip), %rdi
    movl $19, %esi
    movq %r11, %rdx
    xorl %eax, %eax
    call printf@PLT
    movq %rbx, %rdi
    call destroy
    movl $18, %edi
    call make
    movq %rax, 8(%rsp)
    movl $4, %r12d
.LBB11:
    cmpl $18, %r12d
jg .LBB19
.LBB12:
    movl $18, %r11d
    subl %r12d, %r11d
    leal 4(%r11), %r13d
    movl $1, %r14d
    xorl %r15d, %r15d
.LBB13:
    cmpl %r13d, %r15d
jl .LBB18
.LBB14:
    xorl %ebp, %ebp
    xorl %ebx, %ebx
    jmp .LBB15
.LBB18:
    addl %r14d, %r14d
    addl $1, %r15d
    cmpl %r13d, %r15d
jl .LBB18
    jmp .LBB14
.LBB15:
    cmpl %r14d, %ebx
jge .LBB17
.LBB16:
    movl %r12d, %edi
    call make
    movq %rax, %r13
    movq %rax, %rdi
    call check
    addl %eax, %ebp
    movq %r13, %rdi
    call destroy
    addl $1, %ebx
    cmpl %r14d, %ebx
jl .LBB16
.LBB17:
    leaq .Lstr1(%rip), %rdi
    movq %r14, %rsi
    movq %r12, %rdx
    movq %rbp, %rcx
    xorl %eax, %eax
    call printf@PLT
    addl $2, %r12d
    cmpl $18, %r12d
jle .LBB12
.LBB19:
    movq 8(%rsp), %rdi
    call check
    movl %eax, %r10d
    leaq .Lstr2(%rip), %rdi
    movl $18, %esi
    movq %r10, %rdx
    xorl %eax, %eax
    call printf@PLT
    movq 8(%rsp), %rdi
    call destroy
    xorl %eax, %eax
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


