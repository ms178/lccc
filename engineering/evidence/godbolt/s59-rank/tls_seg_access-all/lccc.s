.Lstr0:

tls_slots:
tls_indirect:

tls_pass:
    pushq %rbx
    pushq %r12
    pushq %r15
    movq %fs:0, %rax
    leaq tls_indirect@TPOFF(%rax), %rbx
    movq %fs:0, %rax
    leaq tls_slots@TPOFF(%rax), %rsi
    leaq 8(%rsi), %rdx
    movq %rdx, (%rbx)
    movl %edi, %eax
    movq %rax, (%rdx)
    movq 8(%rsi), %r12
    movl %edi, %r11d
    leaq 16(%rsi), %r10
    xorl %edi, %edi
    movl $2, %edx
.LBB1:
    cmpq $64, %rdx
jg .LBB3
.LBB2:
    addq %r11, %r12
    movq %r12, (%r10)
    movq %rdx, %r9
    andq $7, %r9
    movq 8(%rsi, %r9, 8), %r15
    xorq %r12, %r15
    addq %r15, %rdi
    addq $1, %rdx
    leaq 8(%r10), %r10
    cmpq $64, %rdx
jle .LBB2
.LBB3:
    movq (%rbx), %r11
    addq (%r11), %rdi
    movq %rdi, %rax
    popq %r15
    popq %r12
    popq %rbx
    ret

main:
    pushq %rbx
    pushq %r12
    subq $24, %rsp
    xorl %ebx, %ebx
    xorl %r12d, %r12d
.LBB5:
    cmpl $200000, %r12d
jae .LBB7
.LBB6:
    movl %r12d, %r11d
    orl $1, %r11d
    movl %r11d, %edi
    call tls_pass
    addq %rax, %rbx
    addl $1, %r12d
    cmpl $200000, %r12d
jb .LBB6
.LBB7:
    leaq .Lstr0(%rip), %rdi
    movq %rbx, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    popq %r12
    popq %rbx
    ret


