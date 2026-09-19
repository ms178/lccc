.Lstr0:

fib:
    movslq %edi, %rsi
    cmpl $1, %edi
jle .LBB5
.LBB1:
    movl $2, %edx
    xorl %r8d, %r8d
    movl $1, %r9d
.LBB2:
    cmpq %rsi, %rdx
jg .LBB4
.LBB3:
    leaq (%r8, %r9, 1), %r11
    addq $1, %rdx
    movq %r9, %r8
    movq %r11, %r9
    cmpq %rsi, %rdx
jle .LBB3
.LBB4:
    movq %r9, %rax
    ret
.LBB5:
    movq %rsi, %rax
    ret

main:
    subq $24, %rsp
    movl $40, %edi
    call fib@PLT
    movq %rax, 16(%rsp)
    movq 16(%rsp), %r11
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    ret


