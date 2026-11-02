.Lstr0:
.Lstr1:
.Lstr2:
.Lstr3:
.Lstr4:
.Lstr5:
.Lstr6:
.Lstr7:
.Lstr8:
.Lstr9:
.Lstr10:
.Lstr11:
.Lstr12:
.Lstr13:
.Lstr14:
.Lstr15:
.Lstr16:
.Lstr17:
.Lstr18:
.Lstr19:

k_urem7:
    xorl %edx, %edx
.LBB1:
    cmpl %esi, %edx
jae .LBB3
.LBB2:
    movl %edi, %r8d
    imulq $613566757, %r8, %r9
    shrq $32, %r9
    subq %r9, %r8
    shrq $1, %r8
    addq %r9, %r8
    shrq $2, %r8
    movl %r8d, %r11d
    imull $7, %r11d, %r11d
    movl %edi, %r10d
    subl %r11d, %r10d
    leal (%r10, %rdx), %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB2
.LBB3:
    movl %edi, %eax
    ret

k_urem10:
    movl $3435973837, %edx
    xorl %r8d, %r8d
.LBB5:
    cmpl %esi, %r8d
jae .LBB7
.LBB6:
    movl %edi, %r9d
    imulq %rdx, %r9
    shrq $35, %r9
    movl %r9d, %r11d
    leaq (%r11, %r11, 4), %r11
    addl %r11d, %r11d
    movl %edi, %r10d
    subl %r11d, %r10d
    movl %r8d, %r9d
    xorl %edi, %r9d
    leal (%r10, %r9), %edi
    addl $1, %r8d
    cmpl %esi, %r8d
jb .LBB6
.LBB7:
    movl %edi, %eax
    ret

k_udiv7:
    xorl %edx, %edx
.LBB9:
    cmpl %esi, %edx
jae .LBB11
.LBB10:
    movl %edi, %r8d
    imulq $613566757, %r8, %r9
    shrq $32, %r9
    subq %r9, %r8
    shrq $1, %r8
    addq %r9, %r8
    shrq $2, %r8
    movl %r8d, %r11d
    movl %edx, %r10d
    leaq (%r10, %r10, 2), %r10
    leal (%r11, %r10), %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB10
.LBB11:
    movl %edi, %eax
    ret

k_udr10:
    movl $3435973837, %edx
    xorl %r8d, %r8d
.LBB13:
    cmpl %esi, %r8d
jae .LBB15
.LBB14:
    movl %edi, %r9d
    imulq %rdx, %r9
    shrq $35, %r9
    movl %r9d, %r11d
    movl %r9d, %r10d
    leaq (%r10, %r10, 4), %r10
    addl %r10d, %r10d
    movl %edi, %r9d
    subl %r10d, %r9d
    leaq (%r9, %r9, 2), %r9
    addl %r9d, %r11d
    leal (%r11, %r8), %edi
    addl $1, %r8d
    cmpl %esi, %r8d
jb .LBB14
.LBB15:
    movl %edi, %eax
    ret

k_sdr7:
    movl $2454267027, %edx
    xorl %r8d, %r8d
.LBB17:
    cmpl %esi, %r8d
jae .LBB19
.LBB18:
    movslq %edi, %r9
    imulq %rdx, %r9
    sarq $34, %r9
    movq %r9, %r11
    shrq $63, %r11
    addq %r11, %r9
    movslq %r9d, %r10
    leal 0(,%r10,8), %r9d
    subl %r10d, %r9d
    movl %edi, %r11d
    subl %r9d, %r11d
    leaq (%r11, %r11, 4), %r11
    addl %r11d, %r10d
    movl %r10d, %edi
    subl %r8d, %edi
    addl $1, %r8d
    cmpl %esi, %r8d
jb .LBB18
.LBB19:
    movl %edi, %eax
    ret

k_srem7:
    movl $2454267027, %edx
    xorl %r8d, %r8d
.LBB21:
    cmpl %esi, %r8d
jae .LBB23
.LBB22:
    movslq %edi, %r9
    imulq %rdx, %r9
    sarq $34, %r9
    movq %r9, %r11
    shrq $63, %r11
    addq %r11, %r9
    movl %r9d, %r10d
    imull $7, %r10d, %r10d
    movl %edi, %r9d
    subl %r10d, %r9d
    movzwl %r8w, %eax
    addl %eax, %r9d
    leal -30000(%r9), %edi
    addl $1, %r8d
    cmpl %esi, %r8d
jb .LBB22
.LBB23:
    movl %edi, %eax
    ret

k_srem16:
    xorl %edx, %edx
.LBB25:
    cmpl %esi, %edx
jae .LBB27
.LBB26:
    movl %edi, %r8d
    sarl $31, %r8d
    shrl $28, %r8d
    leal (%rdi, %r8), %r9d
    sarl $4, %r9d
    shll $4, %r9d
    movl %edi, %r11d
    subl %r9d, %r11d
    leaq (%r11, %r11, 2), %r11
    movzbl %dl, %eax
    addl %eax, %r11d
    leal -100(%r11), %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB26
.LBB27:
    movl %edi, %eax
    ret

k_sdr16:
    xorl %edx, %edx
.LBB29:
    cmpl %esi, %edx
jae .LBB31
.LBB30:
    movl %edi, %r8d
    sarl $31, %r8d
    shrl $28, %r8d
    leal (%rdi, %r8), %r9d
    sarl $4, %r9d
    movl %r9d, %r11d
    shll $4, %r11d
    movl %edi, %r10d
    subl %r11d, %r10d
    shll $3, %r10d
    xorl %r10d, %r9d
    movl %r9d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB30
.LBB31:
    movl %edi, %eax
    ret

k_mul7:
    xorl %edx, %edx
.LBB33:
    cmpl %esi, %edx
jae .LBB35
.LBB34:
    leal 0(,%rdi,8), %r8d
    subl %edi, %r8d
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB34
.LBB35:
    movl %edi, %eax
    ret

k_mul11:
    xorl %edx, %edx
.LBB37:
    cmpl %esi, %edx
jae .LBB39
.LBB38:
    imull $11, %edi, %r8d
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB38
.LBB39:
    movl %edi, %eax
    ret

k_mul17:
    xorl %edx, %edx
.LBB41:
    cmpl %esi, %edx
jae .LBB43
.LBB42:
    movl %edi, %r8d
    shll $4, %r8d
    addl %edi, %r8d
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB42
.LBB43:
    movl %edi, %eax
    ret

k_mul24:
    xorl %edx, %edx
.LBB45:
    cmpl %esi, %edx
jae .LBB47
.LBB46:
    movl %edi, %r8d
    leaq (%r8, %r8, 2), %r8
    shll $3, %r8d
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB46
.LBB47:
    movl %edi, %eax
    ret

k_mul45:
    xorl %edx, %edx
.LBB49:
    cmpl %esi, %edx
jae .LBB51
.LBB50:
    movl %edi, %r8d
    leaq (%r8, %r8, 4), %r8
    leaq (%r8, %r8, 8), %r8
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB50
.LBB51:
    movl %edi, %eax
    ret

k_mulm3:
    xorl %edx, %edx
.LBB53:
    cmpl %esi, %edx
jae .LBB55
.LBB54:
    movl %edi, %r8d
    leaq (%r8, %r8, 2), %r8
    negl %r8d
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB54
.LBB55:
    movl %edi, %eax
    ret

k_mul1000:
    xorl %edx, %edx
.LBB57:
    cmpl %esi, %edx
jae .LBB59
.LBB58:
    imull $1000, %edi, %r8d
    movl %r8d, %edi
    xorl %edx, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB58
.LBB59:
    movl %edi, %eax
    ret

k_fnv:
    xorl %edx, %edx
.LBB61:
    cmpl %esi, %edx
jae .LBB63
.LBB62:
    movzbl %dl, %r8d
    xorl %edi, %r8d
    imull $16777619, %r8d, %edi
    addl $1, %edx
    cmpl %esi, %edx
jb .LBB62
.LBB63:
    movl %edi, %eax
    ret

k_digits:
    pushq %rbx
    pushq %r12
    pushq %r13
    movl $3435973837, %edx
    xorl %ebx, %ebx
    xorl %r12d, %r12d
.LBB65:
    cmpl %esi, %r12d
jae .LBB70
.LBB66:
    leal (%rdi, %r12), %r10d
    movq %rbx, %r11
.LBB67:
    testl %r10d, %r10d
je .LBB69
.LBB68:
    movl %r10d, %r8d
    imulq %rdx, %r8
    shrq $35, %r8
    movl %r8d, %r8d
    movl %r8d, %r9d
    leaq (%r9, %r9, 4), %r9
    addl %r9d, %r9d
    movl %r10d, %r13d
    subl %r9d, %r13d
    addl %r13d, %r11d
    movl %r8d, %r10d
    testl %r10d, %r10d
jne .LBB68
.LBB69:
    addl $1, %r12d
    movq %r11, %rbx
    cmpl %esi, %r12d
jb .LBB66
.LBB70:
    movq %rbx, %rax
    popq %r13
    popq %r12
    popq %rbx
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $504, %rsp
    movq %rdi, %rbx
    movq %rsi, %r12
    cmpl $1, %edi
jg .LBB73
.LBB72:
    movl $20000000, %ebx
    jmp .LBB74
.LBB73:
    movq 8(%r12), %rdi
    xorl %esi, %esi
    xorl %edx, %edx
    call strtoul@PLT
    movq %rax, %r8
    movl %eax, %ebx
.LBB74:
    leaq 88(%rsp), %rdi
    vpxor %xmm0, %xmm0, %xmm0
    movl $6, %ecx
.Lmset_loop_0:
    vmovdqu %ymm0, (%rdi)
    vmovdqu %ymm0, 32(%rdi)
    addq $64, %rdi
    decq %rcx
    jne .Lmset_loop_0
    vmovdqu %xmm0, (%rdi)
    vmovdqu %xmm0, 8(%rdi)
    leaq .Lstr0(%rip), %rax
    movq %rax, 88(%rsp)
    leaq k_urem7(%rip), %r9
    movq %r9, 96(%rsp)
    movl $123456789, 104(%rsp)
    leaq .Lstr1(%rip), %rax
    movq %rax, 112(%rsp)
    leaq k_urem10(%rip), %r10
    movq %r10, 120(%rsp)
    movl $987654321, 128(%rsp)
    leaq .Lstr2(%rip), %rax
    movq %rax, 136(%rsp)
    leaq k_udiv7(%rip), %rsi
    movq %rsi, 144(%rsp)
    movl $3735928559, 152(%rsp)
    leaq .Lstr3(%rip), %rax
    movq %rax, 160(%rsp)
    leaq k_udr10(%rip), %r9
    movq %r9, 168(%rsp)
    movl $305419896, 176(%rsp)
    leaq .Lstr4(%rip), %rax
    movq %rax, 184(%rsp)
    leaq k_sdr7(%rip), %r10
    movq %r10, 192(%rsp)
    movl $2147422772, 200(%rsp)
    leaq .Lstr5(%rip), %rax
    movq %rax, 208(%rsp)
    leaq k_srem7(%rip), %rsi
    movq %rsi, 216(%rsp)
    movl $2147488308, 224(%rsp)
    leaq .Lstr6(%rip), %rax
    movq %rax, 232(%rsp)
    leaq k_srem16(%rip), %r9
    movq %r9, 240(%rsp)
    movl $4294967280, 248(%rsp)
    leaq .Lstr7(%rip), %rax
    movq %rax, 256(%rsp)
    leaq k_sdr16(%rip), %r10
    movq %r10, 264(%rsp)
    movl $267242409, 272(%rsp)
    leaq .Lstr8(%rip), %rax
    movq %rax, 280(%rsp)
    leaq k_mul7(%rip), %rsi
    movq %rsi, 288(%rsp)
    movl $1, 296(%rsp)
    leaq .Lstr9(%rip), %rax
    movq %rax, 304(%rsp)
    leaq k_mul11(%rip), %r9
    movq %r9, 312(%rsp)
    movl $3, 320(%rsp)
    leaq .Lstr10(%rip), %rax
    movq %rax, 328(%rsp)
    leaq k_mul17(%rip), %r10
    movq %r10, 336(%rsp)
    movl $5, 344(%rsp)
    leaq .Lstr11(%rip), %rax
    movq %rax, 352(%rsp)
    leaq k_mul24(%rip), %rsi
    movq %rsi, 360(%rsp)
    movl $7, 368(%rsp)
    leaq .Lstr12(%rip), %rax
    movq %rax, 376(%rsp)
    leaq k_mul45(%rip), %r9
    movq %r9, 384(%rsp)
    movl $9, 392(%rsp)
    leaq .Lstr13(%rip), %rax
    movq %rax, 400(%rsp)
    leaq k_mulm3(%rip), %r10
    movq %r10, 408(%rsp)
    movl $11, 416(%rsp)
    leaq .Lstr14(%rip), %rax
    movq %rax, 424(%rsp)
    leaq k_mul1000(%rip), %rsi
    movq %rsi, 432(%rsp)
    movl $13, 440(%rsp)
    leaq .Lstr15(%rip), %rax
    movq %rax, 448(%rsp)
    leaq k_fnv(%rip), %r9
    movq %r9, 456(%rsp)
    movl $2166136261, 464(%rsp)
    leaq .Lstr16(%rip), %rax
    movq %rax, 472(%rsp)
    leaq k_digits(%rip), %r12
    movq %r12, 480(%rsp)
    movl $1234567, 488(%rsp)
    movq %rbx, %rax
    shrl $4, %eax
    movl %eax, 44(%rsp)
    movabsq $4741671816366391296, %rax
    movq %rax, 48(%rsp)
    xorl %r14d, %r14d
    xorl %r15d, %r15d
    leaq 88(%rsp), %rax
    movq %rax, %rbp
.LBB75:
    cmpq $17, %r15
jae .LBB77
.LBB76:
    leaq 8(%rbp), %r13
    movq (%r13), %r9
    cmpq %r12, %r9
    movq %rbx, %rax
    movl 44(%rsp), %ecx
    cmovel %ecx, %eax
    movl %eax, 32(%rsp)
    movl $1, %edi
    leaq 72(%rsp), %rsi
    call clock_gettime@PLT
    movq 72(%rsp), %rax
    vcvtsi2sdq %rax, %xmm2, %xmm2
    vmulsd 48(%rsp), %xmm2, %xmm2
    movq 80(%rsp), %rax
    vcvtsi2sdq %rax, %xmm3, %xmm3
    vaddsd %xmm3, %xmm2, %xmm0
    vmovsd %xmm0, 24(%rsp)
    movl 16(%rbp), %r11d
    movq (%r13), %rax
    movq %rax, 16(%rsp)
    movl %r11d, %edi
    movl 32(%rsp), %esi
    movq %rax, %r10
    call *%r10
    movl %eax, %r13d
    movl $1, %edi
    leaq 56(%rsp), %rsi
    call clock_gettime@PLT
    movq 56(%rsp), %rax
    vcvtsi2sdq %rax, %xmm4, %xmm4
    vmulsd 48(%rsp), %xmm4, %xmm4
    movq 64(%rsp), %rax
    vcvtsi2sdq %rax, %xmm5, %xmm5
    vaddsd %xmm5, %xmm4, %xmm4
    movl %r14d, %r10d
    shll $5, %r10d
    subl %r14d, %r10d
    leal (%r10, %r13), %r14d
    movq stderr(%rip), %r8
    movq (%rbp), %r11
    vsubsd 24(%rsp), %xmm4, %xmm4
    movl 32(%rsp), %eax
    vcvtsi2sdq %rax, %xmm6, %xmm6
    vdivsd %xmm6, %xmm4, %xmm4
    movq %r8, %rdi
    leaq .Lstr17(%rip), %rsi
    movq %r11, %rdx
    vmovsd %xmm4, %xmm0, %xmm0
    movb $1, %al
    call fprintf@PLT
    movq (%rbp), %r10
    leaq .Lstr18(%rip), %rdi
    movq %r10, %rsi
    movq %r13, %rdx
    xorl %eax, %eax
    call printf@PLT
    addq $1, %r15
    leaq 24(%rbp), %rbp
    cmpq $17, %r15
jb .LBB76
.LBB77:
    leaq .Lstr19(%rip), %rdi
    movq %r14, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    vzeroupper
    addq $504, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


