.Lstr0:

main:
    subq $24, %rsp
    movabsq $50000005000000, %rax
    movq %rax, 16(%rsp)
    movq 16(%rsp), %r11
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    ret


