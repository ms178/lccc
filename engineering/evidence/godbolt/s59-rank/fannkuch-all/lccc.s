.Lstr0:

perm:
perm1:
count:
maxflips:
checksum:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    movl $0, maxflips(%rip)
    movl $0, checksum(%rip)
    leaq perm1(%rip), %r11
    movl $0, (%r11)
    movl $1, perm1+4(%rip)
    movl $2, perm1+8(%rip)
    movl $3, perm1+12(%rip)
    movl $4, perm1+16(%rip)
    movl $5, perm1+20(%rip)
    movl $6, perm1+24(%rip)
    movl $7, perm1+28(%rip)
    movl $8, perm1+32(%rip)
    movl $9, perm1+36(%rip)
    movl $10, perm1+40(%rip)
    leaq perm(%rip), %r8
    movl $11, %r12d
    movq $0, 24(%rsp)
.LBB1:
.LBB2:
    cmpl $1, %r12d
jg .LBB15
.LBB3:
    xorl %edi, %edi
.LBB4:
    cmpq $32, %rdi
jge .LBB6
.LBB5:
    vmovdqu (%r11,%rdi), %ymm2
    vmovdqu %ymm2, (%r8,%rdi)
    addq $32, %rdi
    cmpq $32, %rdi
jl .LBB5
.LBB6:
    shrq $2, %rdi
    movslq %edi, %rdx
    movq %rdx, %rdi
    shlq $2, %rdi
    leaq perm(%rip), %rcx
    leaq (%rcx, %rdi), %rsi
    leaq perm1(%rip), %rcx
    leaq (%rcx, %rdi), %r10
.LBB7:
    cmpq $11, %rdx
jl .LBB14
.LBB8:
    xorl %r14d, %r14d
.LBB9:
    movl (%r8), %r10d
    testl %r10d, %r10d
je .LBB16
.LBB10:
    leal 1(%r10), %r15d
    sarl $1, %r15d
    xorl %esi, %esi
.LBB11:
    cmpl %r15d, %esi
jge .LBB13
.LBB12:
    movslq %esi, %rsi
    movl (%r8, %rsi, 4), %ebp
    movl %r10d, %r9d
    subl %esi, %r9d
    movslq %r9d, %r13
    shlq $2, %r13
    movslq %r9d, %r9
    movl (%r8, %r9, 4), %eax
    movl %eax, (%r8, %rsi, 4)
    movl %ebp, %eax
    movl %eax, (%r8, %r9, 4)
    leal 1(%rsi), %eax
    movl %eax, 20(%rsp)
    movl %eax, %esi
    cmpl %r15d, %esi
jl .LBB12
.LBB13:
    addl $1, %r14d
    jmp .LBB9
.LBB14:
    movl (%r10), %eax
    movl %eax, (%rsi)
    addq $1, %rdx
    leaq 4(%rsi), %rsi
    leaq 4(%r10), %r10
    cmpq $11, %rdx
jl .LBB14
    jmp .LBB8
.LBB15:
    leal -1(%r12), %edx
    movslq %edx, %rdx
    leaq count(%rip), %rcx
    movl %r12d, (%rcx, %rdx, 4)
    movq %rdx, %r12
    cmpl $1, %edx
jg .LBB15
    jmp .LBB3
.LBB16:
    cmpl maxflips(%rip), %r14d
jle .LBB18
.LBB17:
    movl %r14d, maxflips(%rip)
.LBB18:
    movl 24(%rsp), %esi
    andl $1, %esi
    movl %r14d, %edx
    negl %edx
    testl %esi, %esi
    movq %r14, %r10
    cmovnel %edx, %r10d
    addl checksum(%rip), %r10d
    movl %r10d, checksum(%rip)
    movl 24(%rsp), %ebx
    addl $1, %ebx
    movslq %r12d, %rsi
    shlq $2, %rsi
    leaq perm1(%rip), %rcx
    leaq (%rcx, %rsi), %r13
    leaq count(%rip), %r14
    addq %rsi, %r14
.LBB19:
    cmpl $11, %r12d
je .LBB26
.LBB20:
    movl (%r11), %r15d
    movslq %r12d, %rbp
    xorl %esi, %esi
.LBB21:
    cmpq %rbp, %rsi
jge .LBB23
.LBB22:
    leaq 1(%rsi), %rdi
    movl (%r11, %rdi, 4), %r9d
    movl %r9d, (%r11, %rsi, 4)
    movq %rdi, %rsi
    cmpq %rbp, %rdi
jl .LBB22
.LBB23:
    movl %r15d, (%r13)
    movl (%r14), %r9d
    subl $1, %r9d
    movl %r9d, (%r14)
    testl %r9d, %r9d
jg .LBB25
.LBB24:
    leal 1(%r12), %eax
    movl %eax, 20(%rsp)
    leaq 4(%r13), %r13
    leaq 4(%r14), %r14
    movl 20(%rsp), %r12d
    cmpl $11, %r12d
je .LBB26
    jmp .LBB20
.LBB25:
    movq %rbx, 24(%rsp)
    jmp .LBB1
.LBB26:
    movslq checksum(%rip), %r11
    movslq maxflips(%rip), %r10
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    movl $11, %edx
    movq %r10, %rcx
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    vzeroupper
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


