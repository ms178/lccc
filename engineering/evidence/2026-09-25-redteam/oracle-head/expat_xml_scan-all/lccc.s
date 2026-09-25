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
jae .LBB27
.LBB2:
    movzbl (%r9), %r11d
    cmpb $-128, %r11b
jae .LBB13
.LBB3:
    movzbl %r11b, %r10d
    andl $-33, %r10d
    subl $65, %r10d
    cmpl $25, %r10d
jbe .LBB6
.LBB4:
    cmpb $95, %r11b
je .LBB6
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
    cmpb $45, %r11b
je .LBB7
.LBB12:
    cmpb $46, %r11b
je .LBB8
    jmp .LBB27
.LBB7:
.LBB8:
    leaq 1(%r9), %rdi
    jmp .LBB9
.LBB13:
    leaq 62(%r11), %r10
    cmpb $29, %r10b
ja .LBB22
.LBB14:
    movl $2, %r11d
    jmp .LBB15
.LBB22:
    leaq 32(%r11), %r10
    cmpb $15, %r10b
ja .LBB25
.LBB23:
    movl $3, %r10d
    jmp .LBB24
.LBB25:
    subq $-16, %r11
    cmpb $4, %r11b
ja .LBB27
.LBB26:
    movl $4, %r10d
.LBB24:
    movq %r10, %r11
.LBB15:
    movq %rsi, %rax
    subq %r9, %rax
    cmpq %r11, %rax
jb .LBB27
.LBB16:
    movzbl 1(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB27
.LBB17:
    cmpq $2, %r11
jbe .LBB19
.LBB18:
    movzbl 2(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB27
.LBB19:
    cmpq $3, %r11
jbe .LBB21
.LBB20:
    movzbl 3(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB27
.LBB21:
    leaq (%r9, %r11, 1), %rdi
.LBB9:
    movq %rdi, %r9
    cmpq %rsi, %rdi
jb .LBB2
.LBB27:
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
jne .LBB30
.LBB29:
    leaq check_name_kernel.utf8_name.2+8(%rip), %r10
    leaq check_name_kernel.utf8_name.2(%rip), %rdi
    movq %r10, %rsi
    call expat_utf8_name_length
    cmpq $8, %rax
    movl $0, %ecx
    movl $1, %r9d
    cmovnel %ecx, %r9d
    testl %r9d, %r9d
jne .LBB31
.LBB30:
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
.LBB31:
    leaq expat_xml_data(%rip), %rbx
    leaq make_xml_corpus.fragment.0(%rip), %r12
    xorl %r13d, %r13d
.LBB32:
    leaq 61(%r13), %rdi
    cmpq $1048576, %rdi
ja .LBB39
.LBB33:
    movq %r13, %r14
    xorl %r15d, %r15d
.LBB34:
    leaq 1(%r15), %rsi
    cmpq $61, %rsi
jb .LBB38
.LBB35:
    movq %r14, %r13
    leaq 61(%r13), %rdx
    cmpq $1048576, %rdx
jbe .LBB33
    jmp .LBB39
.LBB38:
    leaq 1(%r14), %r11
    movzbl (%r12, %r15), %eax
    movb %al, (%rbx, %r14)
    addq $1, %r15
    movq %r11, %r14
    leaq 1(%r15), %rsi
    cmpq $61, %rsi
jb .LBB38
    jmp .LBB35
.LBB39:
    leaq expat_xml_data+1048576(%rip), %rbp
    movq %r13, %r12
.LBB36:
    movabsq $1099511628211, %rax
    movq %rax, 32(%rsp)
    cmpq $1048576, %r12
jb .LBB37
.LBB40:
    movq $0, 56(%rsp)
    movq $0, 48(%rsp)
    jmp .LBB41
.LBB37:
    leaq 1(%r12), %r13
    movb $32, (%rbx, %r12)
    movq %r13, %r12
    jmp .LBB36
.LBB41:
    movl 56(%rsp), %eax
    cmpl $64, %eax
jae .LBB62
.LBB42:
    leaq expat_xml_data(%rip), %r12
    movabsq $1469598103934665603, %rax
    movq %rax, 40(%rsp)
.LBB43:
    cmpq %rbp, %r12
jae .LBB52
.LBB44:
    movzbl (%r12), %r15d
    cmpb $34, %r15b
je .LBB46
.LBB45:
    cmpb $39, %r15b
jne .LBB53
.LBB46:
    leaq 1(%r12), %r14
    movzbl %r15b, %r13d
.LBB47:
    cmpq %rbp, %r14
jae .LBB50
.LBB48:
    movzbl (%r14), %r8d
    cmpl %r13d, %r8d
je .LBB50
.LBB49:
    addq $1, %r14
    cmpq %rbp, %r14
jb .LBB48
.LBB50:
    leaq 1(%r14), %rdi
    cmpq %rbp, %r14
    movq %r14, %r13
    cmovbq %rdi, %r13
    movq 40(%rsp), %r14
    jmp .LBB51
.LBB53:
    movzbl %r15b, %esi
    andl $-33, %esi
    subl $65, %esi
    cmpl $25, %esi
jbe .LBB61
.LBB54:
    cmpb $95, %r15b
je .LBB61
.LBB55:
    cmpb $58, %r15b
je .LBB61
.LBB56:
    cmpb $-62, %r15b
jae .LBB57
    jmp .LBB60
.LBB61:
.LBB57:
    movq %r12, %rdi
    movq %rbp, %rsi
    call expat_utf8_name_length
    movq %rax, %r13
    testq %rax, %rax
je .LBB52
.LBB58:
    movzbl %r15b, %edx
    leaq (%r13, %rdx, 1), %r11
    movq 40(%rsp), %r10
    xorq %r11, %r10
    movq %r10, %rdi
    imulq 32(%rsp), %rdi
    leaq (%r12, %r13, 1), %r9
    jmp .LBB59
.LBB60:
    leaq 1(%r12), %r9
    movq 40(%rsp), %rdi
.LBB59:
    movq %r9, %r13
    movq %rdi, %r14
.LBB51:
    movq %r13, %r12
    movq %r14, 40(%rsp)
    cmpq %rbp, %r13
jb .LBB44
.LBB52:
    movq 48(%rsp), %rax
    xorq 40(%rsp), %rax
    movq %rax, 48(%rsp)
    movl 56(%rsp), %eax
    imull $8191, %eax, %eax
    movl %eax, %edx
    andq $1048575, %rdx
    movzbl (%rbx, %rdx), %r8d
    xorl $1, %r8d
    movb %r8b, (%rbx, %rdx)
    movl 56(%rsp), %eax
    addl $1, %eax
    movq %rax, 56(%rsp)
    cmpl $64, %eax
jb .LBB42
.LBB62:
    leaq .Lstr0(%rip), %rdi
    movq 48(%rsp), %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1
