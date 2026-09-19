.Lstr0:

main:
    subq $8, %rsp
    xorl %r11d, %r11d
    xorl %r10d, %r10d
    movl $777, %r8d
.LBB1:
    cmpl $50000000, %r10d
jge .LBB21
.LBB2:
    imull $1664525, %r8d, %r8d
    leal 1013904223(%r8), %r9d
    movl %r9d, %edi
    shrl $16, %edi
    andq $15, %rdi
    movzwl %r9w, %r8d
    movslq %edi, %rax
    cmpq $16, %rax
    jae .LBB3
    leaq .Ljt_0(%rip), %rcx
    movslq (%rcx,%rax,4), %rdx
    addq %rcx, %rdx
    jmpq *%rdx
.Ljt_0:
.LBB3:
    xorl %edi, %edi
    jmp .LBB20
.LBB4:
    leal (%r10, %r8), %edi
    jmp .LBB20
.LBB5:
    movl %r10d, %edi
    subl %r8d, %edi
    jmp .LBB20
.LBB6:
    movl %r10d, %edi
    imull %r8d, %edi
    jmp .LBB20
.LBB7:
    movl %r10d, %edi
    xorl %r8d, %edi
    jmp .LBB20
.LBB8:
    movl %r10d, %edi
    orl %r8d, %edi
    jmp .LBB20
.LBB9:
    movl %r10d, %edi
    andl %r8d, %edi
    jmp .LBB20
.LBB10:
    movl %r8d, %esi
    andl $7, %esi
    shlxl %esi, %r10d, %edi
    jmp .LBB20
.LBB11:
    movl %r8d, %esi
    andl $7, %esi
    sarxl %esi, %r10d, %edi
    jmp .LBB20
.LBB12:
    movl %r10d, %esi
    notl %esi
    leal (%rsi, %r8), %edi
    jmp .LBB20
.LBB13:
    movl %r8d, %esi
    notl %esi
    movl %r10d, %edi
    subl %esi, %edi
    jmp .LBB20
.LBB14:
    leal (%r10, %r8), %edi
    leaq (%rdi, %rdi, 2), %rdi
    jmp .LBB20
.LBB15:
    movl %r10d, %esi
    subl %r8d, %esi
    movl %esi, %edi
    leaq (%rdi, %rdi, 4), %rdi
    jmp .LBB20
.LBB16:
    xorl %r10d, %r8d
    leal 1(%r8), %edi
    jmp .LBB20
.LBB17:
    orl %r10d, %r8d
    movl %r8d, %edi
    subl $1, %edi
    jmp .LBB20
.LBB18:
    andl %r10d, %r8d
    leal 2(%r8), %edi
    jmp .LBB20
.LBB19:
    leal (%r10, %r8), %esi
    leal 1(%rsi), %edi
.LBB20:
    movslq %edi, %rsi
    addq %rsi, %r11
    addl $1, %r10d
    movq %r9, %r8
    cmpl $50000000, %r10d
jl .LBB2
.LBB21:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $8, %rsp
    ret


