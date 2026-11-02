.Lstr0:

linux_bitmap_a:
linux_bitmap_b:

linux_find_next_andnot_bit:
    pushq %r12
    pushq %rbp
    subq $24, %rsp
    cmpq %rdx, %rcx
jb .LBB2
.LBB1:
    movq %rdx, %rax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %rbp
    popq %r12
    ret
.LBB2:
    movq %rcx, %r9
    andq $63, %r9
    movq $-1, %r11
    shlxq %r9, %r11, %r11
    movq %rcx, %r10
    shrq $6, %r10
    movq (%rdi, %r10, 8), %r12
    movq (%rsi, %r10, 8), %rax
    andnq %r12, %rax, %r9
    movq %r9, %rcx
    andq %r11, %rcx
.LBB3:
    testq %rcx, %rcx
jne .LBB6
.LBB4:
    leaq 1(%r10), %r9
    shlq $6, %r9
    cmpq %rdx, %r9
jb .LBB5
.LBB7:
    movq %rdx, %rax
    jmp .L__lccc_epilogue_1
.LBB5:
    addq $1, %r10
    movq (%rdi, %r10, 8), %rbp
    movq (%rsi, %r10, 8), %rax
    andnq %rbp, %rax, %rcx
    testq %rcx, %rcx
je .LBB4
.LBB6:
    shlq $6, %r10
    xorl %r11d, %r11d
    tzcntq %rcx, %r11
    movl %r11d, %edi
    addq %rdi, %r10
    cmpq %rdx, %r10
    movq %rdx, %rcx
    cmovbq %r10, %rcx
    movq %rcx, %rax
    jmp .L__lccc_epilogue_1

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $72, %rsp
    movq $32, 40(%rsp)
    movq $4, 48(%rsp)
    movq $0, 56(%rsp)
    movq $0, 16(%rsp)
    movq $-1, 24(%rsp)
    movq $-1, 32(%rsp)
    leaq 40(%rsp), %rdi
    leaq 16(%rsp), %rsi
    movl $192, %edx
    xorl %ecx, %ecx
    call linux_find_next_andnot_bit
    movq %rax, %rdi
    cmpq $5, %rax
jne .LBB10
.LBB9:
    leaq 40(%rsp), %rdi
    leaq 16(%rsp), %rsi
    movl $192, %edx
    movl $6, %ecx
    call linux_find_next_andnot_bit
    movq %rax, %rsi
    cmpq $192, %rax
    movl $0, %ecx
    movl $1, %edx
    cmovnel %ecx, %edx
    testl %edx, %edx
jne .LBB11
.LBB10:
    movl $2, %eax
.L__lccc_epilogue_2:
    addq $72, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB11:
    leaq linux_bitmap_b(%rip), %rbx
    leaq linux_bitmap_a(%rip), %r12
    xorl %r13d, %r13d
.LBB12:
    cmpq $16384, %r13
jae .LBB16
.LBB13:
    movq $0, (%r12, %r13, 8)
    movq $-1, (%rbx, %r13, 8)
    movq %r13, %r9
    andq $63, %r9
    cmpq $5, %r9
jne .LBB15
.LBB14:
    imulq $13, %r13, %rdi
    leaq 7(%rdi), %rsi
    andq $63, %rsi
    movl $1, %edx
    shlxq %rsi, %rdx, %rdx
    movq %r13, %r11
    shlq $3, %r11
    movq %rdx, (%r12, %r13, 8)
    movq $0, (%rbx, %r13, 8)
.LBB15:
    addq $1, %r13
    cmpq $16384, %r13
jb .LBB13
.LBB16:
    xorl %r14d, %r14d
    xorl %r11d, %r11d
.LBB17:
    cmpl $1024, %r14d
jae .LBB21
.LBB18:
    movq %r14, %rax
    andq $63, %rax
    movl %eax, %r15d
    movl %r14d, %r9d
    movq %r9, %rbp
    shlq $19, %rbp
    movq %r11, %r12
.LBB19:
    leaq linux_bitmap_a(%rip), %rdi
    leaq linux_bitmap_b(%rip), %rsi
    movl $1048576, %edx
    movq %r15, %rcx
    call linux_find_next_andnot_bit
    leaq (%rax, %rbp, 1), %rdx
    movq %r12, %r10
    xorq %rdx, %r10
    leaq 1(%rax), %r8
    cmpq $1048576, %rax
    movq %r12, %r11
    cmovbq %r10, %r11
    cmpq $1048576, %rax
    cmovbq %r8, %r15
    movq %r11, %r12
    cmpq $1048576, %rax
jb .LBB19
.LBB20:
    movl %r14d, %r9d
    imulq $13, %r9, %r9
    andq $255, %r9
    shlq $6, %r9
    addq $5, %r9
    leal 0(,%r14,8), %edi
    subl %r14d, %edi
    andl $63, %edi
    movl $1, %esi
    shlxq %rdi, %rsi, %rsi
    xorq (%rbx, %r9, 8), %rsi
    movq %rsi, (%rbx, %r9, 8)
    addl $1, %r14d
    cmpl $1024, %r14d
jb .LBB18
.LBB21:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_2


