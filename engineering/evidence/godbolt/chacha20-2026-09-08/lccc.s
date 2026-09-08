.section .rodata
.Lstr0:
    .byte 37, 48, 49, 54, 108, 108, 120, 10, 0

.section .rodata
.align 16
.type check_known_vector.test_in.0, @object
.size check_known_vector.test_in.0, 64
check_known_vector.test_in.0:
    .long 1634760805
    .long 857760878
    .long 2036477234
    .long 1797285236
    .long 50462976
    .long 117835012
    .long 185207048
    .long 252579084
    .long 319951120
    .long 387323156
    .long 454695192
    .long 522067228
    .long 1
    .long 150994944
    .long 1241513984
    .zero 4

.text
.p2align 4
.type chacha20_core, @function
chacha20_core:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $232, %rsp
    .cfi_def_cfa_offset 288
    movq %rdi, 216(%rsp)
    movq %rsi, 208(%rsp)
    movl (%rsi), %eax
    movq %rax, 200(%rsp)
    movq 208(%rsp), %rax
    leaq 4(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    movq %rax, 120(%rsp)
    movq 208(%rsp), %rax
    leaq 8(%rax), %rax
    movq %rax, %rsi
    movl (%rax), %eax
    movq %rax, 160(%rsp)
    movq 208(%rsp), %rax
    leaq 12(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    movq %rax, 80(%rsp)
    movq 208(%rsp), %rax
    leaq 16(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    movq %rax, 128(%rsp)
    movq 208(%rsp), %rax
    leaq 20(%rax), %rax
    movq %rax, %r9
    movl (%rax), %eax
    movq %rax, 168(%rsp)
    movq 208(%rsp), %rax
    leaq 24(%rax), %rax
    movq %rax, %r11
    movl (%rax), %eax
    movq %rax, 88(%rsp)
    movq 208(%rsp), %rax
    leaq 28(%rax), %rax
    movq %rax, %r10
    movl (%rax), %eax
    movq %rax, 136(%rsp)
    movq 208(%rsp), %rax
    leaq 32(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    movq %rax, 176(%rsp)
    movq 208(%rsp), %rax
    leaq 36(%rax), %rax
    movq %rax, %rsi
    movl (%rax), %eax
    movq %rax, 96(%rsp)
    movq 208(%rsp), %rax
    leaq 40(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    movq %rax, 144(%rsp)
    movq 208(%rsp), %rax
    leaq 44(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    movq %rax, 184(%rsp)
    movq 208(%rsp), %rax
    leaq 48(%rax), %rax
    movq %rax, %r9
    movl (%rax), %eax
    movq %rax, 104(%rsp)
    movq 208(%rsp), %rax
    leaq 52(%rax), %rax
    movq %rax, %r11
    movl (%rax), %eax
    movq %rax, 152(%rsp)
    movq 208(%rsp), %rax
    leaq 56(%rax), %rax
    movq %rax, %r10
    movl (%rax), %eax
    movq %rax, 192(%rsp)
    movq 208(%rsp), %rax
    leaq 60(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    movq %rax, 112(%rsp)
    movq $0, 72(%rsp)
.LBB1:
    movl 72(%rsp), %eax
    cmpl $10, %eax
jge .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    movl 200(%rsp), %esi
    addl 128(%rsp), %esi
    movl 104(%rsp), %edx
    xorl %esi, %edx
    roll $16, %edx
    movl 176(%rsp), %r8d
    addl %edx, %r8d
    movl 128(%rsp), %r9d
    xorl %r8d, %r9d
    roll $12, %r9d
    addl %r9d, %esi
    xorl %esi, %edx
    roll $8, %edx
    addl %edx, %r8d
    xorl %r8d, %r9d
    roll $7, %r9d
    movl 120(%rsp), %r11d
    addl 168(%rsp), %r11d
    movl 152(%rsp), %r10d
    xorl %r11d, %r10d
    roll $16, %r10d
    movl 96(%rsp), %edi
    addl %r10d, %edi
    movl 168(%rsp), %ebx
    xorl %edi, %ebx
    roll $12, %ebx
    addl %ebx, %r11d
    xorl %r11d, %r10d
    roll $8, %r10d
    addl %r10d, %edi
    xorl %edi, %ebx
    roll $7, %ebx
    movl 160(%rsp), %r12d
    addl 88(%rsp), %r12d
    movl 192(%rsp), %r13d
    xorl %r12d, %r13d
    roll $16, %r13d
    movl 144(%rsp), %r14d
    addl %r13d, %r14d
    movl 88(%rsp), %r15d
    xorl %r14d, %r15d
    roll $12, %r15d
    addl %r15d, %r12d
    xorl %r12d, %r13d
    roll $8, %r13d
    addl %r13d, %r14d
    xorl %r14d, %r15d
    roll $7, %r15d
    movl 80(%rsp), %ebp
    addl 136(%rsp), %ebp
    movl %ebp, %eax
    xorl 112(%rsp), %eax
    roll $16, %eax
    movl %eax, 68(%rsp)
    movl 184(%rsp), %eax
    addl 68(%rsp), %eax
    movl %eax, 64(%rsp)
    movl 136(%rsp), %eax
    xorl 64(%rsp), %eax
    roll $12, %eax
    movl %eax, 60(%rsp)
    addl %eax, %ebp
    movl %ebp, %eax
    xorl 68(%rsp), %eax
    roll $8, %eax
    movl %eax, 68(%rsp)
    movl 64(%rsp), %eax
    addl 68(%rsp), %eax
    movl %eax, 56(%rsp)
    movl 60(%rsp), %eax
    xorl 56(%rsp), %eax
    roll $7, %eax
    movl %eax, 64(%rsp)
    addl %ebx, %esi
    movl %esi, %eax
    xorl 68(%rsp), %eax
    roll $16, %eax
    movl %eax, 68(%rsp)
    addl 68(%rsp), %r14d
    xorl %r14d, %ebx
    roll $12, %ebx
    addl %ebx, %esi
    movl %esi, %eax
    xorl 68(%rsp), %eax
    roll $8, %eax
    movq %rax, 112(%rsp)
    addl 112(%rsp), %r14d
    xorl %r14d, %ebx
    movl %ebx, %eax
    roll $7, %eax
    movq %rax, 168(%rsp)
    addl %r15d, %r11d
    xorl %r11d, %edx
    roll $16, %edx
    movl 56(%rsp), %ebx
    addl %edx, %ebx
    xorl %ebx, %r15d
    roll $12, %r15d
    addl %r15d, %r11d
    xorl %r11d, %edx
    roll $8, %edx
    addl %edx, %ebx
    xorl %ebx, %r15d
    movl %r15d, %eax
    roll $7, %eax
    movq %rax, 88(%rsp)
    addl 64(%rsp), %r12d
    xorl %r12d, %r10d
    roll $16, %r10d
    addl %r10d, %r8d
    movl 64(%rsp), %r15d
    xorl %r8d, %r15d
    roll $12, %r15d
    addl %r15d, %r12d
    xorl %r12d, %r10d
    roll $8, %r10d
    addl %r10d, %r8d
    xorl %r8d, %r15d
    movl %r15d, %eax
    roll $7, %eax
    movq %rax, 136(%rsp)
    addl %r9d, %ebp
    xorl %ebp, %r13d
    roll $16, %r13d
    addl %r13d, %edi
    xorl %edi, %r9d
    roll $12, %r9d
    addl %r9d, %ebp
    xorl %ebp, %r13d
    roll $8, %r13d
    addl %r13d, %edi
    xorl %edi, %r9d
    movl %r9d, %eax
    roll $7, %eax
    movq %rax, 128(%rsp)
    movl 72(%rsp), %eax
    addl $1, %eax
    cltq
    movq %rax, 72(%rsp)
    movq %rsi, 200(%rsp)
    movq %r13, 192(%rsp)
    movq %rbx, 184(%rsp)
    movq %r8, 176(%rsp)
    movq %r12, 160(%rsp)
    movq %r10, 152(%rsp)
    movq %r14, 144(%rsp)
    movq %r11, 120(%rsp)
    movq %rdx, 104(%rsp)
    movq %rdi, 96(%rsp)
    movq %rbp, 80(%rsp)
    movl %eax, %eax
    cmpl $10, %eax
jl .LBB2
.LBB3:
    movq 208(%rsp), %rcx
    movl (%rcx), %eax
    addl 200(%rsp), %eax
    movq 216(%rsp), %rcx
    movl %eax, (%rcx)
    movq 216(%rsp), %rdx
    leaq 4(%rdx), %rdx
    movq 208(%rsp), %rax
    leaq 4(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    addl 120(%rsp), %eax
    movl %eax, (%rdx)
    movq 216(%rsp), %r11
    leaq 8(%r11), %r11
    movq 208(%rsp), %rax
    leaq 8(%rax), %rax
    movq %rax, %r10
    movl (%rax), %eax
    addl 160(%rsp), %eax
    movl %eax, (%r11)
    movq 216(%rsp), %rsi
    leaq 12(%rsi), %rsi
    movq 208(%rsp), %rax
    leaq 12(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    addl 80(%rsp), %eax
    movl %eax, (%rsi)
    movq 216(%rsp), %r9
    leaq 16(%r9), %r9
    movq 208(%rsp), %rax
    leaq 16(%rax), %rax
    movq %rax, %r11
    movl (%rax), %eax
    addl 128(%rsp), %eax
    movl %eax, (%r9)
    movq 216(%rsp), %rdi
    leaq 20(%rdi), %rdi
    movq 208(%rsp), %rax
    leaq 20(%rax), %rax
    movq %rax, %rsi
    movl (%rax), %eax
    addl 168(%rsp), %eax
    movl %eax, (%rdi)
    movq 216(%rsp), %r8
    leaq 24(%r8), %r8
    movq 208(%rsp), %rax
    leaq 24(%rax), %rax
    movq %rax, %r9
    movl (%rax), %eax
    addl 88(%rsp), %eax
    movl %eax, (%r8)
    movq 216(%rsp), %r10
    leaq 28(%r10), %r10
    movq 208(%rsp), %rax
    leaq 28(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    addl 136(%rsp), %eax
    movl %eax, (%r10)
    movq 216(%rsp), %rdx
    leaq 32(%rdx), %rdx
    movq 208(%rsp), %rax
    leaq 32(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    addl 176(%rsp), %eax
    movl %eax, (%rdx)
    movq 216(%rsp), %r11
    leaq 36(%r11), %r11
    movq 208(%rsp), %rax
    leaq 36(%rax), %rax
    movq %rax, %r10
    movl (%rax), %eax
    addl 96(%rsp), %eax
    movl %eax, (%r11)
    movq 216(%rsp), %rsi
    leaq 40(%rsi), %rsi
    movq 208(%rsp), %rax
    leaq 40(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    addl 144(%rsp), %eax
    movl %eax, (%rsi)
    movq 216(%rsp), %r9
    leaq 44(%r9), %r9
    movq 208(%rsp), %rax
    leaq 44(%rax), %rax
    movq %rax, %r11
    movl (%rax), %eax
    addl 184(%rsp), %eax
    movl %eax, (%r9)
    movq 216(%rsp), %rdi
    leaq 48(%rdi), %rdi
    movq 208(%rsp), %rax
    leaq 48(%rax), %rax
    movq %rax, %rsi
    movl (%rax), %eax
    addl 104(%rsp), %eax
    movl %eax, (%rdi)
    movq 216(%rsp), %r8
    leaq 52(%r8), %r8
    movq 208(%rsp), %rax
    leaq 52(%rax), %rax
    movq %rax, %r9
    movl (%rax), %eax
    addl 152(%rsp), %eax
    movl %eax, (%r8)
    movq 216(%rsp), %r10
    leaq 56(%r10), %r10
    movq 208(%rsp), %rax
    leaq 56(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    addl 192(%rsp), %eax
    movl %eax, (%r10)
    movq 216(%rsp), %rdx
    leaq 60(%rdx), %rdx
    movq 208(%rsp), %rax
    leaq 60(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    addl 112(%rsp), %eax
    movl %eax, (%rdx)
    addq $232, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.cfi_endproc
.size chacha20_core, .-chacha20_core

.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $232, %rsp
    .cfi_def_cfa_offset 288
    leaq check_known_vector.test_in.0(%rip), %r11
    leaq 32(%rsp), %rdi
    movq %r11, %rax
    movq %r11, %rsi
    call chacha20_core
    movl 32(%rsp), %r11d
    cmpl $3840405776, %r11d
jne .LBB8
.LBB5:
    movl 36(%rsp), %r8d
    cmpl $358169553, %r8d
jne .LBB8
.LBB6:
    movl 88(%rsp), %edi
    cmpl $3900952779, %edi
jne .LBB8
.LBB7:
    movl 92(%rsp), %edx
    cmpl $1312575650, %edx
    movl $0, %ecx
    movl $1, %r11d
    cmovnel %ecx, %r11d
    testl %r11d, %r11d
jne .LBB10
    jmp .LBB9
.LBB8:
.LBB9:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $232, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB10:
    movl $1634760805, 160(%rsp)
    movl $857760878, 164(%rsp)
    movl $2036477234, 168(%rsp)
    movl $1797285236, 172(%rsp)
    movl $67372036, 176(%rsp)
    movl $84215045, 180(%rsp)
    movl $101058054, 184(%rsp)
    movl $117901063, 188(%rsp)
    movl $134744072, 192(%rsp)
    movl $151587081, 196(%rsp)
    movl $168430090, 200(%rsp)
    movl $185273099, 204(%rsp)
    movl $0, 208(%rsp)
    movl $3735928559, 212(%rsp)
    movl $19088743, 216(%rsp)
    movl $2309737967, 220(%rsp)
    xorl %r10d, %r10d
    xorl %r15d, %r15d
.LBB11:
    cmpl $16, %r15d
jae .LBB16
.p2align 4,,10
.p2align 3
.LBB12:
    movq %r10, %rbp
    movq $0, 24(%rsp)
.LBB13:
    movq 24(%rsp), %rax
    cmpl $131072, %eax
jae .LBB15
.p2align 4,,10
.p2align 3
.LBB14:
    movl %r15d, %eax
    xorl 24(%rsp), %eax
    movl %eax, 208(%rsp)
    leaq 96(%rsp), %rdi
    leaq 160(%rsp), %rsi
    call chacha20_core
    movl 96(%rsp), %r11d
    movl %r11d, %r8d
    shlq $32, %r8
    movl 156(%rsp), %r9d
    xorq %r9, %r8
    movl 124(%rsp), %edi
    shlq $16, %rdi
    xorq %rdi, %r8
    addq %r8, %rbp
    andl $255, %r11d
    movl 176(%rsp), %eax
    xorl %r11d, %eax
    movl %eax, 176(%rsp)
    movq 24(%rsp), %rax
    addl $1, %eax
    movq %rax, 24(%rsp)
    cmpl $131072, %eax
jb .LBB14
.LBB15:
    addl $1, %r15d
    movq %rbp, %r10
    cmpl $16, %r15d
jb .LBB12
.LBB16:
    leaq .Lstr0(%rip), %rdx
    movq %rdx, %rdi
    movq %r10, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
