.Lstr0:

sieve:

count_primes:
    subq $8, %rsp
    leaq sieve(%rip), %rdi
    movl $1, %esi
    movl $10000001, %edx
    call memset@PLT
    leaq sieve(%rip), %r11
    movb $0, sieve+1(%rip)
    movb $0, (%r11)
    movl $2, %r10d
.LBB1:
    movl %r10d, %r8d
    imull %r10d, %r8d
    cmpl $10000000, %r8d
jg .LBB7
.LBB2:
    movslq %r10d, %r9
    movsbq (%r11, %r9), %rsi
    testb %sil, %sil
je .LBB6
.LBB3:
    movl %r10d, %eax
    imull %r10d, %eax
    movslq %eax, %rdx
    movslq %r10d, %r8
    movq %rdx, %r9
.LBB4:
    cmpq $10000000, %r9
jg .LBB6
.LBB5:
    movb $0, (%r11, %r9)
    leaq (%r9, %r8, 1), %r9
    cmpq $10000000, %r9
jle .LBB5
.LBB6:
    addl $1, %r10d
    movl %r10d, %edx
    imull %r10d, %edx
    cmpl $10000000, %edx
jle .LBB2
.LBB7:
    xorl %r10d, %r10d
    movl $2, %r8d
.LBB8:
    cmpq $10000000, %r8
jg .LBB10
.LBB9:
    movsbq (%r11, %r8), %rdi
    leal 1(%r10), %esi
    testb %dil, %dil
    cmovnel %esi, %r10d
    addq $1, %r8
    cmpq $10000000, %r8
jle .LBB9
.LBB10:
    movl %r10d, %eax
    addq $8, %rsp
    ret

main:
    subq $24, %rsp
    call count_primes@PLT
    movl %eax, 16(%rsp)
    movl 16(%rsp), %r11d
    leaq .Lstr0(%rip), %rdi
    movl $10000000, %esi
    movq %r11, %rdx
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    ret


