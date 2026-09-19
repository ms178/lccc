.Lstr0:

gzip_crc32_table:
main.check.0:

gzip_crc_data:

gzip_crc32_update:
    movl %edi, %r8d
    xorl $-1, %r8d
    leaq gzip_crc32_table(%rip), %r9
    xorl %r11d, %r11d
.LBB1:
    cmpq %rdx, %r11
jae .LBB3
.LBB2:
    movzbl (%rsi, %r11), %eax
    movzbl %r8b, %edi
    xorl %eax, %edi
    shrl $8, %r8d
    xorl (%r9, %rdi, 4), %r8d
    addq $1, %r11
    cmpq %rdx, %r11
jb .LBB2
.LBB3:
    xorl $-1, %r8d
    movl %r8d, %eax
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    subq $24, %rsp
    xorl %edi, %edi
    leaq main.check.0(%rip), %rsi
    movl $9, %edx
    call gzip_crc32_update
    movq %rax, %r11
    cmpl $3421780262, %eax
je .LBB6
.LBB5:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB6:
    leaq gzip_crc_data(%rip), %rbx
    xorl %r12d, %r12d
    movl $305419896, %r10d
.LBB7:
    cmpq $1048576, %r12
jae .LBB9
.LBB8:
    imull $1664525, %r10d, %r8d
    leal 1013904223(%r8), %r10d
    movl %r10d, %esi
    shrl $24, %esi
    movb %sil, (%rbx, %r12)
    addq $1, %r12
    cmpq $1048576, %r12
jb .LBB8
.LBB9:
    xorl %r13d, %r13d
    xorl %r14d, %r14d
.LBB10:
    cmpl $64, %r14d
jae .LBB12
.LBB11:
    movq %r13, %rdi
    leaq gzip_crc_data(%rip), %rsi
    movl $1048576, %edx
    call gzip_crc32_update
    movq %rax, %r13
    movzbl %al, %r10d
    imull $8191, %r14d, %eax
    movl %eax, %r8d
    andq $1048575, %r8
    movzbl (%rbx, %r8), %eax
    xorl %eax, %r10d
    movb %r10b, (%rbx, %r8)
    addl $1, %r14d
    cmpl $64, %r14d
jb .LBB11
.LBB12:
    leaq .Lstr0(%rip), %rdi
    movq %r13, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1


