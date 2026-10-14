.Lstr0:

make_xml_corpus.fragment.0:
check_name_kernel.ascii_name.1:
check_name_kernel.utf8_name.2:

expat_xml_data:

expat_utf8_name_length:
    movq %rdi, %rdx
    movq %rdi, %r9
.LBB1:
    cmpq %rsi, %r9
jae .LBB26
.LBB2:
    movzbl (%r9), %r11d
    cmpb $-128, %r11b
jae .LBB12
.LBB3:
    movzbl %r11b, %r10d
    andl $-33, %r10d
    subl $65, %r10d
    cmpl $25, %r10d
    jbe .LBB7
.LBB4:
    cmpb $95, %r11b
    je .LBB7
.LBB5:
    cmpb $58, %r11b
jne .LBB10
.LBB6:
    jmp .LBB7
.LBB10:
    movzbl %r11b, %edi
    subl $48, %edi
    cmpb $9, %dil
jbe .LBB7
.LBB11:
    leaq -45(%r11), %r10
    cmpb $1, %r10b
jbe .LBB8
    jmp .LBB26
.LBB7:
.LBB8:
    leaq 1(%r9), %rdi
    jmp .LBB9
.LBB12:
    leaq 62(%r11), %r10
    cmpb $29, %r10b
ja .LBB21
.LBB13:
    movl $2, %r11d
    jmp .LBB14
.LBB21:
    leaq 32(%r11), %r10
    cmpb $15, %r10b
ja .LBB24
.LBB22:
    movl $3, %r10d
    jmp .LBB23
.LBB24:
    subq $-16, %r11
    cmpb $4, %r11b
ja .LBB26
.LBB25:
    movl $4, %r10d
.LBB23:
    movq %r10, %r11
.LBB14:
    movq %rsi, %rax
    subq %r9, %rax
    cmpq %r11, %rax
jb .LBB26
.LBB15:
    movzbl 1(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB26
.LBB16:
    cmpq $2, %r11
jbe .LBB18
.LBB17:
    movzbl 2(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB26
.LBB18:
    cmpq $3, %r11
jbe .LBB20
.LBB19:
    movzbl 3(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB26
.LBB20:
    leaq (%r9, %r11, 1), %rdi
.LBB9:
    movq %rdi, %r9
    cmpq %rsi, %rdi
jb .LBB2
.LBB26:
    movq %r9, %rax
    subq %rdx, %rax
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $72, %rsp
    leaq check_name_kernel.ascii_name.1+7(%rip), %r11
    leaq check_name_kernel.ascii_name.1(%rip), %rdi
    movq %r11, %rsi
    call expat_utf8_name_length
    cmpq $7, %rax
jne .LBB29
.LBB28:
    leaq check_name_kernel.utf8_name.2+8(%rip), %r10
    leaq check_name_kernel.utf8_name.2(%rip), %rdi
    movq %r10, %rsi
    call expat_utf8_name_length
    cmpq $8, %rax
    movl $0, %ecx
    movl $1, %r9d
    cmovnel %ecx, %r9d
    testl %r9d, %r9d
jne .LBB30
.LBB29:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $72, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB30:
    leaq expat_xml_data(%rip), %rbx
    leaq make_xml_corpus.fragment.0(%rip), %r12
    xorl %r13d, %r13d
.LBB31:
    leaq 61(%r13), %rdi
    cmpq $1048576, %rdi
ja .LBB38
.LBB32:
    movq %r13, %r14
    xorl %r15d, %r15d
.LBB33:
    leaq 1(%r15), %rsi
    cmpq $61, %rsi
jb .LBB37
.LBB34:
    movq %r14, %r13
    leaq 61(%r13), %rdx
    cmpq $1048576, %rdx
jbe .LBB32
    jmp .LBB38
.LBB37:
    movzbl (%r12, %r15), %eax
    movb %al, (%rbx, %r14)
    addq $1, %r15
    addq $1, %r14
    leaq 1(%r15), %rdi
    cmpq $61, %rdi
jb .LBB37
    jmp .LBB34
.LBB38:
    leaq expat_xml_data+1048576(%rip), %rbp
.LBB35:
    movabsq $1099511628211, %rax
    movq %rax, 32(%rsp)
    cmpq $1048576, %r13
jb .LBB36
.LBB39:
    movq $0, 56(%rsp)
    movq $0, 48(%rsp)
    jmp .LBB40
.LBB36:
    movb $32, (%rbx, %r13)
    addq $1, %r13
    jmp .LBB35
.LBB40:
    movl 56(%rsp), %eax
    cmpl $64, %eax
jae .LBB61
.LBB41:
    leaq expat_xml_data(%rip), %r15
    movabsq $1469598103934665603, %rax
    movq %rax, 40(%rsp)
.LBB42:
    cmpq %rbp, %r15
jae .LBB51
.LBB43:
    movzbl (%r15), %r14d
    cmpb $34, %r14b
je .LBB45
.LBB44:
    cmpb $39, %r14b
jne .LBB52
.LBB45:
    movzbl %r14b, %r13d
    leaq 1(%r15), %r12
.LBB46:
    cmpq %rbp, %r12
jae .LBB49
.LBB47:
    movzbl (%r12), %r10d
    cmpl %r13d, %r10d
je .LBB49
.LBB48:
    addq $1, %r12
    cmpq %rbp, %r12
jb .LBB47
.LBB49:
    leaq 1(%r12), %rcx
    cmpq %rbp, %r12
    movq %r12, %rax
    cmovbq %rcx, %rax
    movq %rax, %r12
    movq 40(%rsp), %r13
    jmp .LBB50
.LBB52:
    movzbl %r14b, %edi
    andl $-33, %edi
    subl $65, %edi
    cmpl $25, %edi
jbe .LBB60
.LBB53:
    cmpb $95, %r14b
je .LBB60
.LBB54:
    cmpb $58, %r14b
je .LBB60
.LBB55:
    cmpb $-62, %r14b
jae .LBB56
    jmp .LBB59
.LBB60:
.LBB56:
    movq %r15, %rdi
    movq %rbp, %rsi
    call expat_utf8_name_length
    movq %rax, %r12
    testq %rax, %rax
je .LBB51
.LBB57:
    movzbl %r14b, %esi
    leaq (%r12, %rsi, 1), %rdx
    movq 40(%rsp), %r11
    xorq %rdx, %r11
    movq %r11, %r9
    imulq 32(%rsp), %r9
    leaq (%r15, %r12, 1), %r8
    jmp .LBB58
.LBB59:
    leaq 1(%r15), %r8
    movq 40(%rsp), %r9
.LBB58:
    movq %r8, %r12
    movq %r9, %r13
.LBB50:
    movq %r12, %r15
    movq %r13, 40(%rsp)
    cmpq %rbp, %r12
jb .LBB43
.LBB51:
    movq 48(%rsp), %rax
    xorq 40(%rsp), %rax
    movq %rax, 48(%rsp)
    movl 56(%rsp), %eax
    imull $8191, %eax, %eax
    movl %eax, %esi
    andq $1048575, %rsi
    movzbl (%rbx, %rsi), %r10d
    xorl $1, %r10d
    movb %r10b, (%rbx, %rsi)
    movl 56(%rsp), %eax
    addl $1, %eax
    movq %rax, 56(%rsp)
    cmpl $64, %eax
jb .LBB41
.LBB61:
    leaq .Lstr0(%rip), %rdi
    movq 48(%rsp), %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1


