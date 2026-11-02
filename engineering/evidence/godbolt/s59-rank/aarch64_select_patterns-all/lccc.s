.Lstr0:

conditional_increment:
    leal 1(%rdi), %edx
    testl %esi, %esi
    cmovnel %edx, %edi
    movl %edi, %eax
    ret

narrow_high_constant:
    cmpw $-2, %di
    sete %sil
    movzbl %sil, %esi
    movl %esi, %eax
    ret

select_pressure:
    testl %edi, %edi
    movq %rdx, %r10
    cmovnel %esi, %r10d
    testl %edi, %edi
    movq %r8, %rdx
    cmovnel %ecx, %edx
    testl %edi, %edi
    cmovnel %r9d, %esi
    testl %edi, %edi
    movl %esi, %r8d
    cmovnel %edx, %r8d
    addl %r10d, %r8d
    addl %r9d, %r8d
    movl %r8d, %eax
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    subq $24, %rsp
    movl $1, %ebx
    xorl %r12d, %r12d
    xorl %r13d, %r13d
    xorl %r14d, %r14d
.LBB4:
    cmpl $50000000, %r14d
jae .LBB6
.LBB5:
    imull $1664525, %ebx, %r11d
    leal 1013904223(%r11), %ebx
    movl %ebx, %r11d
    andl $1, %r11d
    movl %r13d, %edi
    movl %r11d, %esi
    call conditional_increment@PLT
    movl %eax, %r13d
    movl %ebx, %r8d
    andl $8, %r8d
    movl %ebx, %r10d
    shrl $3, %r10d
    movl %r8d, %edi
    movl %ebx, %esi
    movl %r14d, %edx
    movl %eax, %ecx
    movl %r10d, %r8d
    movl %r12d, %r9d
    call select_pressure@PLT
    movl %eax, %r9d
    addl %r9d, %r12d
    addl $1, %r14d
    cmpl $50000000, %r14d
jb .LBB5
.LBB6:
    movl $65534, %edi
    call narrow_high_constant@PLT
    xorl %eax, %r12d
    leaq .Lstr0(%rip), %rdi
    movq %r13, %rsi
    movq %r12, %rdx
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


