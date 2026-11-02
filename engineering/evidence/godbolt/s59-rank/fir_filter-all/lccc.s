.Lstr0:

main.a.0:
main.c.1:
main.d.2:

main:
    subq $8, %rsp
    leaq main.a.0(%rip), %r11
    xorl %r10d, %r10d
.LBB1:
    cmpq $264, %r10
jge .LBB3
.LBB2:
    movq %r10, %r8
    shlq $8, %r8
    addq %r10, %r8
    leaq 13(%r8), %r9
    andq $1023, %r9
    movswq %r9w, %rsi
    subl $512, %esi
    movw %si, (%r11, %r10, 2)
    addq $1, %r10
    cmpq $264, %r10
jl .LBB2
.LBB3:
    movw $7, main.c.1(%rip)
    movw $5, main.c.1+2(%rip)
    movw $5, main.c.1+4(%rip)
    movw $7, main.c.1+6(%rip)
    movw $11, main.c.1+8(%rip)
    movw $17, main.c.1+10(%rip)
    movw $25, main.c.1+12(%rip)
    movw $35, main.c.1+14(%rip)
    leaq main.d.2(%rip), %rdi
    xorl %esi, %esi
.LBB4:
    movabsq $1099511628211, %r10
    cmpq $256, %rsi
jge .LBB6
.LBB5:
    movswq (%r11, %rsi, 2), %rax
    movswq %ax, %rdx
    movswq main.c.1(%rip), %rax
    movswq %ax, %r10
    imull %r10d, %edx
    leaq 1(%rsi), %r8
    movswq (%r11, %r8, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+2(%rip), %rax
    movswq %ax, %r10
    imull %r10d, %r9d
    addl %r9d, %edx
    movswq 4(%r11, %rsi, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+4(%rip), %rax
    movswq %ax, %r10
    imull %r9d, %r10d
    addl %r10d, %edx
    movswq 6(%r11, %rsi, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+6(%rip), %rax
    movswq %ax, %r10
    imull %r10d, %r9d
    addl %r9d, %edx
    movswq 8(%r11, %rsi, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+8(%rip), %rax
    movswq %ax, %r10
    imull %r10d, %r9d
    addl %r9d, %edx
    movswq 10(%r11, %rsi, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+10(%rip), %rax
    movswq %ax, %r10
    imull %r9d, %r10d
    addl %r10d, %edx
    movswq 12(%r11, %rsi, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+12(%rip), %rax
    movswq %ax, %r10
    imull %r10d, %r9d
    addl %r9d, %edx
    movswq 14(%r11, %rsi, 2), %rax
    movswq %ax, %r9
    movswq main.c.1+14(%rip), %rax
    movswq %ax, %r10
    imull %r10d, %r9d
    addl %r9d, %edx
    movq %rsi, %r9
    shlq $2, %r9
    movl %edx, (%rdi, %rsi, 4)
    movq %r8, %rsi
    jmp .LBB4
.LBB6:
    movl $146959, %r11d
    xorl %r8d, %r8d
.LBB7:
    cmpq $256, %r8
jge .LBB9
.LBB8:
    movl (%rdi, %r8, 4), %edx
    movzwl %dx, %eax
    movq %r11, %rsi
    xorq %rax, %rsi
    imulq %r10, %rsi
    sarl $16, %edx
    movzwl %dx, %eax
    xorq %rax, %rsi
    movq %rsi, %r11
    imulq %r10, %r11
    addq $1, %r8
    cmpq $256, %r8
jl .LBB8
.LBB9:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $8, %rsp
    ret


