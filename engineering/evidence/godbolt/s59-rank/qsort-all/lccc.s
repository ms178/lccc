.Lstr0:

arr:

cmp:
    movl (%rdi), %edx
    subl (%rsi), %edx
    movl %edx, %eax
    ret

main:
    subq $8, %rsp
    leaq arr(%rip), %r11
    movl $42, %r10d
    xorl %r8d, %r8d
.LBB2:
    cmpq $1000000, %r8
jge .LBB4
.LBB3:
    imull $1664525, %r10d, %r9d
    leal 1013904223(%r9), %r10d
    movq %r10, %rax
    andq $2147483647, %rax
    movl %eax, (%r11, %r8, 4)
    addq $1, %r8
    cmpq $1000000, %r8
jl .LBB3
.LBB4:
    leaq arr(%rip), %rdi
    movl $1000000, %esi
    movl $4, %edx
    leaq cmp(%rip), %rcx
    call qsort@PLT
    movslq arr+2000000(%rip), %r11
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $8, %rsp
    ret


