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
jae .LBB30
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
    jmp .LBB30
.LBB7:
.LBB8:
    leaq 1(%r9), %rdi
    jmp .LBB9
.LBB13:
    cmpb $-62, %r11b
jb .LBB23
.LBB14:
    cmpb $-33, %r11b
ja .LBB23
.LBB15:
    movl $2, %r11d
    jmp .LBB16
.LBB23:
    cmpb $-32, %r11b
jb .LBB27
.LBB24:
    cmpb $-17, %r11b
ja .LBB27
.LBB25:
    movl $3, %r10d
    jmp .LBB26
.LBB27:
    cmpb $-16, %r11b
jb .LBB30
.LBB28:
    cmpb $-12, %r11b
ja .LBB30
.LBB29:
    movl $4, %r10d
.LBB26:
    movq %r10, %r11
.LBB16:
    movq %rsi, %rax
    subq %r9, %rax
    movq %rax, %r10
    cmpq %r11, %rax
jb .LBB30
.LBB17:
    movzbl 1(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB30
.LBB18:
    cmpq $2, %r11
jbe .LBB20
.LBB19:
    movzbl 2(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB30
.LBB20:
    cmpq $3, %r11
jbe .LBB22
.LBB21:
    movzbl 3(%r9), %r10d
    andl $192, %r10d
    cmpl $128, %r10d
jne .LBB30
.LBB22:
    leaq (%r9, %r11, 1), %rdi
.LBB9:
    movq %rdi, %r9
    cmpq %rsi, %rdi
jb .LBB2
.LBB30:
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
    movq %rax, %r10
    cmpq $7, %rax
jne .LBB33
.LBB32:
    leaq check_name_kernel.utf8_name.2+8(%rip), %r10
    leaq check_name_kernel.utf8_name.2(%rip), %rdi
    movq %r10, %rsi
    call expat_utf8_name_length
    movq %rax, %r8
    cmpq $8, %rax
    movl $0, %ecx
    movl $1, %r9d
    cmovnel %ecx, %r9d
    testl %r9d, %r9d
jne .LBB34
.LBB33:
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
.LBB34:
    leaq expat_xml_data(%rip), %rbx
    leaq make_xml_corpus.fragment.0(%rip), %r12
    xorl %r13d, %r13d
.LBB35:
    leaq 61(%r13), %rdi
    cmpq $1048576, %rdi
ja .LBB42
.LBB36:
    movq %r13, %r14
    xorl %r15d, %r15d
.LBB37:
    leaq 1(%r15), %rsi
    cmpq $61, %rsi
jb .LBB41
.LBB38:
    movq %r14, %r13
    leaq 61(%r13), %rdx
    cmpq $1048576, %rdx
jbe .LBB36
    jmp .LBB42
.LBB41:
    leaq 1(%r14), %r11
    movzbl (%r12, %r15), %eax
    movb %al, (%rbx, %r14)
    addq $1, %r15
    movq %r11, %r14
    leaq 1(%r15), %rsi
    cmpq $61, %rsi
jb .LBB41
    jmp .LBB38
.LBB42:
    leaq expat_xml_data+1048576(%rip), %rbp
    movq %r13, %r12
.LBB39:
    movabsq $1099511628211, %rax
    movq %rax, 32(%rsp)
    cmpq $1048576, %r12
jb .LBB40
.LBB43:
    movq $0, 56(%rsp)
    movq $0, 48(%rsp)
    jmp .LBB44
.LBB40:
    leaq 1(%r12), %r13
    movb $32, (%rbx, %r12)
    movq %r13, %r12
    jmp .LBB39
.LBB44:
    movq 56(%rsp), %rax
    cmpl $64, %eax
jae .LBB65
.LBB45:
    leaq expat_xml_data(%rip), %r12
    movabsq $1469598103934665603, %rax
    movq %rax, 40(%rsp)
.LBB46:
    cmpq %rbp, %r12
jae .LBB55
.LBB47:
    movzbl (%r12), %r15d
    cmpb $34, %r15b
je .LBB49
.LBB48:
    cmpb $39, %r15b
jne .LBB56
.LBB49:
    leaq 1(%r12), %r14
    movzbl %r15b, %r13d
.LBB50:
    cmpq %rbp, %r14
jae .LBB53
.LBB51:
    movzbl (%r14), %r8d
    cmpl %r13d, %r8d
je .LBB53
.LBB52:
    addq $1, %r14
    cmpq %rbp, %r14
jb .LBB51
.LBB53:
    leaq 1(%r14), %rdi
    cmpq %rbp, %r14
    movq %r14, %r13
    cmovbq %rdi, %r13
    movq 40(%rsp), %r14
    jmp .LBB54
.LBB56:
    movzbl %r15b, %esi
    andl $-33, %esi
    subl $65, %esi
    cmpl $25, %esi
jbe .LBB64
.LBB57:
    cmpb $95, %r15b
je .LBB64
.LBB58:
    cmpb $58, %r15b
je .LBB64
.LBB59:
    cmpb $-62, %r15b
jae .LBB60
    jmp .LBB63
.LBB64:
.LBB60:
    movq %r12, %rdi
    movq %rbp, %rsi
    call expat_utf8_name_length
    movq %rax, %r13
    testq %rax, %rax
je .LBB55
.LBB61:
    movzbl %r15b, %edx
    leaq (%r13, %rdx, 1), %r11
    movq 40(%rsp), %r10
    xorq %r11, %r10
    movq %r10, %rdi
    imulq 32(%rsp), %rdi
    leaq (%r12, %r13, 1), %r9
    jmp .LBB62
.LBB63:
    leaq 1(%r12), %r9
    movq 40(%rsp), %rdi
.LBB62:
    movq %r9, %r13
    movq %rdi, %r14
.LBB54:
    movq %r13, %r12
    movq %r14, 40(%rsp)
    cmpq %rbp, %r13
jb .LBB47
.LBB55:
    movq 48(%rsp), %rax
    xorq 40(%rsp), %rax
    movq %rax, 48(%rsp)
    movq 56(%rsp), %rax
    imull $8191, %eax, %eax
    movl %eax, %edx
    andq $1048575, %rdx
    movzbl (%rbx, %rdx), %r8d
    xorl $1, %r8d
    movb %r8b, (%rbx, %rdx)
    movq 56(%rsp), %rax
    addl $1, %eax
    movq %rax, 56(%rsp)
    cmpl $64, %eax
jb .LBB45
.LBB65:
    leaq .Lstr0(%rip), %rdi
    movq 48(%rsp), %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1


