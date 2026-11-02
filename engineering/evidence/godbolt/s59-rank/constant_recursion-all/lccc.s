.Lstr0:

main:
    subq $24, %rsp
    movl $16381, 16(%rsp)
    movl 16(%rsp), %r11d
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    ret


