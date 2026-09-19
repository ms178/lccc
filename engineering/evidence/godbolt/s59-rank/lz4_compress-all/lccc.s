.Lstr0:

src_data:
dst_data:
hash_table:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $104, %rsp
    leaq dst_data(%rip), %rax
    movq %rax, 56(%rsp)
    leaq src_data(%rip), %r13
    leaq 524288(%r13), %r15
    leaq 524276(%r13), %rax
    movq %rax, 48(%rsp)
    leaq 1(%r13), %rax
    movq %rax, 40(%rsp)
    xorl %ebx, %ebx
    movl $1511734397, %r11d
.LBB1:
    cmpl $524288, %ebx
jb .LBB2
.LBB34:
    movq $0, 88(%rsp)
    movq $0, 80(%rsp)
    jmp .LBB35
.LBB2:
    imull $1664525, %r11d, %r11d
    leal 1013904223(%r11), %r12d
    movl %r12d, %r10d
    andl $15, %r10d
    cmpl $6, %r10d
jae .LBB33
    cmpl $128, %ebx
jb .LBB33
.LBB3:
    leal -128(%rbx), %edi
    movl %r12d, %esi
    andl $63, %esi
    addl %esi, %edi
    leaq src_data(%rip), %rcx
    movzbl (%rcx, %rdi), %r10d
    jmp .LBB4
.LBB33:
    movl %r12d, %r8d
    shrl $24, %r8d
    movzbl %r8b, %r10d
.LBB4:
    movl %ebx, %r9d
    leaq src_data(%rip), %rcx
    movb %r10b, (%rcx, %r9)
    addl $1, %ebx
    movq %r12, %r11
    cmpl $524288, %ebx
jb .LBB2
    jmp .LBB34
.LBB35:
    movq 88(%rsp), %rax
    cmpl $24, %eax
jae .LBB36
.LBB5:
    jmp .LBB6
.LBB7:
    movq 40(%rsp), %rbx
    movq 56(%rsp), %rax
    movq %rax, 72(%rsp)
    movq %r13, 64(%rsp)
.LBB8:
    cmpq 48(%rsp), %rbx
jae .LBB28
.LBB9:
    movl (%rbx), %esi
    imull $-1640531535, %esi, %esi
    shrl $18, %esi
    shlq $2, %rsi
    leaq hash_table(%rip), %r11
    addq %rsi, %r11
    movl (%r11), %r10d
    leaq (%r13, %r10, 1), %r12
    movq %rbx, %r8
    subq %r13, %r8
    movl %r8d, (%r11)
    cmpq %rbx, %r12
jae .LBB31
.LBB10:
    cmpq %r13, %r12
jb .LBB31
.LBB11:
    movl (%r12), %edi
    cmpl (%rbx), %edi
jne .LBB31
.LBB12:
    leaq 4(%r12), %r14
    leaq 4(%rbx), %rdx
    movq %rdx, %rbp
.LBB13:
    cmpq %r15, %rbp
jae .LBB16
.LBB14:
    movzbl (%rbp), %r11d
    movzbl (%r14), %r10d
    cmpl %r10d, %r11d
jne .LBB16
.LBB15:
    addq $1, %rbp
    addq $1, %r14
    cmpq %r15, %rbp
jb .LBB14
.LBB16:
    movq %r14, %r8
    subq %r12, %r8
    leaq -4(%r8), %rax
    movq %rax, 32(%rsp)
    movq 72(%rsp), %r9
    addq $1, %r9
    subq 64(%rsp), %rbx
    movl %ebx, 28(%rsp)
    cmpl $15, %ebx
jb .LBB30
.LBB17:
    movq 72(%rsp), %rcx
    movb $240, (%rcx)
    movl 28(%rsp), %ebx
    subl $15, %ebx
    movq %r9, %r12
.LBB18:
    cmpl $255, %ebx
jb .LBB20
.LBB19:
    leaq 1(%r12), %rsi
    movb $255, (%r12)
    leal -255(%rbx), %eax
    movl %eax, 20(%rsp)
    movq %rsi, %r12
    movl %eax, %ebx
    cmpl $255, %ebx
jae .LBB19
.LBB20:
    movq %r12, %rax
    addq $1, %rax
    movq %rax, 16(%rsp)
    movb %bl, (%r12)
    movq 16(%rsp), %r12
    jmp .LBB21
.LBB30:
    movl 28(%rsp), %r11d
    shll $4, %r11d
    movq 72(%rsp), %rcx
    movb %r11b, (%rcx)
    movq %r9, %r12
.LBB21:
    movl 28(%rsp), %edx
    movq %r12, %rdi
    movq 64(%rsp), %rsi
    call memmove@PLT
    movl 28(%rsp), %eax
    addq %r12, %rax
    movq %rbp, %r8
    subq %r14, %r8
    movzwl %r8w, %esi
    movb %sil, (%rax)
    leaq 2(%rax), %r14
    sarl $8, %esi
    movb %sil, 1(%rax)
    movl 32(%rsp), %ebx
    cmpl $15, %ebx
jb .LBB29
.LBB22:
    movq 72(%rsp), %rcx
    movzbl (%rcx), %r8d
    orl $15, %r8d
    movb %r8b, (%rcx)
    movl %ebx, %r12d
    subl $15, %r12d
    movq %r14, %rbx
.LBB23:
    cmpl $255, %r12d
jb .LBB25
.LBB24:
    leaq 1(%rbx), %rdi
    movb $255, (%rbx)
    subl $255, %r12d
    movq %rdi, %rbx
    cmpl $255, %r12d
jae .LBB24
.LBB25:
    leaq 1(%rbx), %rsi
    movb %r12b, (%rbx)
    movq %rsi, %rbx
    jmp .LBB26
.LBB29:
    movzbl 32(%rsp), %r10d
    movq 72(%rsp), %rcx
    movzbl (%rcx), %eax
    orl %eax, %r10d
    movb %r10b, (%rcx)
    movq %r14, %rbx
.LBB26:
    movq %rbp, %r12
    movq %rbx, %r14
    jmp .LBB27
.LBB31:
    movq %rbx, %rdi
    subq 64(%rsp), %rdi
    sarq $6, %rdi
    addq $1, %rdi
    leaq (%rbx, %rdi, 1), %r12
    movq 72(%rsp), %r14
    movq 64(%rsp), %rax
    movq %rax, %rbp
.LBB27:
    movq %r12, %rbx
    movq %r14, 72(%rsp)
    movq %rbp, 64(%rsp)
    cmpq 48(%rsp), %r12
jb .LBB9
.LBB28:
    movq %r15, %rsi
    subq 64(%rsp), %rsi
    movl %esi, %ebp
    movq 72(%rsp), %rbx
    addq $1, %rbx
    movl %ebp, %r11d
    shll $4, %r11d
    cmpl $15, %ebp
    movl $240, %r10d
    cmovbl %r11d, %r10d
    movq 72(%rsp), %rcx
    movb %r10b, (%rcx)
    movl %ebp, %r10d
    movq %rbx, %rdi
    movq 64(%rsp), %rsi
    movq %r10, %rdx
    call memmove@PLT
    leaq (%rbx, %rbp), %r9
    subq 56(%rsp), %r9
    movl %r9d, %edi
    movl %r9d, %esi
    shlq $32, %rsi
    movzbl dst_data(%rip), %eax
    xorq %rax, %rsi
    shrl $1, %edi
    leaq dst_data(%rip), %rcx
    movzbl (%rcx, %rdi), %r10d
    shlq $16, %r10
    xorq %r10, %rsi
    addq %rsi, 80(%rsp)
    movq 88(%rsp), %rax
    imull $4099, %eax, %eax
    movl %eax, %r8d
    andq $524287, %r8
    leaq src_data(%rip), %rdi
    addq %r8, %rdi
    movzbl (%rdi), %esi
    xorl $85, %esi
    movb %sil, (%rdi)
    movq 88(%rsp), %rax
    addl $1, %eax
    movq %rax, 88(%rsp)
    cmpl $24, %eax
jb .LBB5
.LBB36:
    leaq .Lstr0(%rip), %rdi
    movq 80(%rsp), %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $104, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB6:
    movl $16384, %eax
    subq $0, %rax
    movq %rax, %r10
    leaq 0(,%rax,4), %r11
    testq %rax, %rax
je .LBB7
.LBB32:
    leaq hash_table(%rip), %rdi
    xorl %esi, %esi
    movq %r11, %rdx
    call memset@PLT
    jmp .LBB7


