.text
.global _start
_start:
    xorl %ebp, %ebp
    andl $-16, %esp
    call test
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
.section .note.GNU-stack,"",@progbits
