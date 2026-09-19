.Lstr0:

perm:
perm1:
count:

mix:
    pushq %rbx
    xorl %r8d, %r8d
    xorl %r9d, %r9d
.LBB1:
    cmpq $128, %r9
jge .LBB3
.LBB2:
    movl (%rdi, %r9, 4), %ebx
    addl (%rsi, %r9, 4), %ebx
    addl (%rdx, %r9, 4), %ebx
    addl %ebx, %r8d
    movl %r8d, (%rdi, %r9, 4)
    addq $1, %r9
    cmpq $128, %r9
jl .LBB2
.LBB3:
    movl %r8d, %eax
    popq %rbx
    ret

kernel:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    subq $24, %rsp
    movq %rdi, %rbx
    xorl %r12d, %r12d
    xorl %r13d, %r13d
.LBB5:
    cmpl %ebx, %r13d
jge .LBB7
.LBB6:
    leaq perm(%rip), %rdi
    leaq perm1(%rip), %rsi
    leaq count(%rip), %rdx
    movl $128, %ecx
    call mix
    leal (%r12, %rax), %r14d
    leaq perm1(%rip), %rdi
    leaq count(%rip), %rsi
    leaq perm(%rip), %rdx
    movl $128, %ecx
    call mix
    addl %eax, %r14d
    leaq count(%rip), %rdi
    leaq perm(%rip), %rsi
    leaq perm1(%rip), %rdx
    movl $128, %ecx
    call mix
    leal (%r14, %rax), %r12d
    addl $1, %r13d
    cmpl %ebx, %r13d
jl .LBB6
.LBB7:
    movl %r12d, %eax
    addq $24, %rsp
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $16, %rsp
    movq %rdi, %r12
    movq %rsi, %r13
    cmpl $1, %edi
jg .LBB10
.LBB9:
    movl $2000, %ebx
    jmp .LBB11
.LBB10:
    movq 8(%r13), %rdi
    xorl %esi, %esi
    movl $10, %edx
    call strtol@PLT
    movq %rax, %r8
    movslq %eax, %rbx
.LBB11:
    leaq count(%rip), %r9
    leaq perm1(%rip), %rdi
    leaq perm(%rip), %rsi
    xorl %edx, %edx
.LBB12:
    cmpl $128, %edx
jge .LBB14
.LBB13:
    movslq %edx, %rdx
    movl %edx, (%rsi, %rdx, 4)
    movl $128, %r8d
    subl %edx, %r8d
    movl %r8d, (%rdi, %rdx, 4)
    movl %edx, %r8d
    imull %edx, %r8d
    movl %r8d, (%r9, %rdx, 4)
    addl $1, %edx
    cmpl $128, %edx
jl .LBB13
.LBB14:
    movl %ebx, %edi
    call kernel@PLT
    movl %eax, %r11d
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $16, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret


