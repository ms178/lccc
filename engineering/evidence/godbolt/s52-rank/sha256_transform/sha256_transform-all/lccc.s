.Lstr0:

K:

sha256_transform:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $312, %rsp
    movq %rdi, 296(%rsp)
    leaq 40(%rsp), %rdi
    subq %rsi, %rdi
    subq $4, %rdi
    cmpq $25, %rdi
jb .LBB14
.LBB1:
    xorl %r8d, %r8d
    jmp .LBB2
.LBB14:
    xorl %r9d, %r9d
    jmp .LBB5
.LBB2:
    cmpq $64, %r8
jge .LBB4
.LBB3:
    vmovdqu (%rsi,%r8), %ymm2
    vmovdqu %ymm2, 40(%rsp,%r8)
    addq $32, %r8
    cmpq $64, %r8
jl .LBB3
    jmp .LBB4
.LBB7:
    leaq 104(%rsp), %r11
    movl $16, %r10d
.LBB8:
    cmpq $64, %r10
jge .LBB10
.LBB9:
    movl 32(%rsp, %r10, 4), %edx
    rorxl $17, %edx, %r8d
    rorxl $19, %edx, %r9d
    xorl %r9d, %r8d
    shrl $10, %edx
    xorl %edx, %r8d
    movl 12(%rsp, %r10, 4), %edx
    addl %edx, %r8d
    movl -20(%rsp, %r10, 4), %esi
    rorxl $7, %esi, %edx
    rorxl $18, %esi, %r9d
    xorl %r9d, %edx
    shrl $3, %esi
    xorl %esi, %edx
    addl %edx, %r8d
    movl -24(%rsp, %r10, 4), %edx
    addl %r8d, %edx
    movl %edx, (%r11)
    addq $1, %r10
    leaq 4(%r11), %r11
    cmpq $64, %r10
jl .LBB9
.LBB10:
    movq 296(%rsp), %rcx
    movl (%rcx), %ebx
    movl 4(%rcx), %r11d
    movl 8(%rcx), %edi
    movl 12(%rcx), %r12d
    movl 16(%rcx), %r13d
    movl 20(%rcx), %r9d
    movl 24(%rcx), %r14d
    leaq 28(%rcx), %rsi
    movl (%rsi), %r15d
    leaq K(%rip), %rbp
    movl %r11d, %eax
    andl %edi, %eax
    movq %rax, 32(%rsp)
    xorl %r8d, %r8d
    movq %r15, %rdx
    movq %r14, %r10
.LBB11:
    cmpq $64, %r8
jge .LBB13
.LBB12:
    rorxl $6, %r13d, %esi
    rorxl $11, %r13d, %r14d
    xorl %r14d, %esi
    rorxl $25, %r13d, %r15d
    xorl %r15d, %esi
    leal (%rdx, %rsi), %r14d
    movl %r13d, %esi
    andl %r9d, %esi
    andnl %r10d, %r13d, %r15d
    xorl %r15d, %esi
    addl %esi, %r14d
    movl (%rbp, %r8, 4), %r15d
    addl %r15d, %r14d
    movl 40(%rsp, %r8, 4), %esi
    addl %esi, %r14d
    rorxl $2, %ebx, %esi
    rorxl $13, %ebx, %r15d
    xorl %r15d, %esi
    rorxl $22, %ebx, %r15d
    xorl %r15d, %esi
    movl %ebx, %r15d
    andl %r11d, %r15d
    movl %ebx, %eax
    andl %edi, %eax
    movl %eax, 28(%rsp)
    movl %r15d, %eax
    xorl 28(%rsp), %eax
    xorl 32(%rsp), %eax
    addl %eax, %esi
    addl %r14d, %r12d
    movl %r12d, 28(%rsp)
    addl %esi, %r14d
    leaq 1(%r8), %rax
    movq %rax, %r8
    movq %r10, %rdx
    movq %r9, %r10
    movq %rdi, %r12
    movq %r15, 32(%rsp)
    movq %r13, %r9
    movq %r11, %rdi
    movl 28(%rsp), %esi
    movq %rsi, %r13
    movq %rbx, %r11
    movq %r14, %rbx
    cmpq $64, %rax
jl .LBB12
.LBB13:
    movq 296(%rsp), %rax
    vmovdqu (%rax), %xmm15
    vmovd %ebx, %xmm0
    movq %r11, %rcx
    vmovd %r11d, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %rdi, %rax
    shlq $32, %r12
    orq %r12, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vmovdqa %xmm0, %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    movq 296(%rsp), %rax
    vmovdqu %xmm15, (%rax)
    movq 296(%rsp), %r8
    leaq 16(%r8), %r8
    vmovdqu (%r8), %xmm15
    vmovd %r13d, %xmm0
    movq %r9, %rcx
    vmovd %r9d, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %r10, %rax
    shlq $32, %rdx
    orq %rdx, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vmovdqa %xmm0, %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovdqu %xmm15, (%r8)
    vzeroupper
    addq $312, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB4:
    shrq $2, %r8
    movslq %r8d, %r9
.LBB5:
    cmpl $16, %r9d
jge .LBB7
.LBB6:
    movslq %r9d, %r11
    movl (%rsi, %r11, 4), %eax
    movl %eax, 40(%rsp, %r11, 4)
    addl $1, %r9d
    cmpl $16, %r9d
jl .LBB6
    jmp .LBB7

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $600, %rsp
    movl $1779033703, 464(%rsp)
    movl $3144134277, 468(%rsp)
    movl $1013904242, 472(%rsp)
    movl $2773480762, 476(%rsp)
    movl $1359893119, 480(%rsp)
    movl $2600822924, 484(%rsp)
    movl $528734635, 488(%rsp)
    movl $1541459225, 492(%rsp)
    movl $1633837952, 400(%rsp)
    movl $0, 404(%rsp)
    movl $0, 408(%rsp)
    movl $0, 412(%rsp)
    pxor %xmm0, %xmm0
    movdqu %xmm0, 416(%rsp)
    movdqu %xmm0, 432(%rsp)
    movl $0, 448(%rsp)
    movl $0, 452(%rsp)
    movl $0, 456(%rsp)
    movl $24, 460(%rsp)
    leaq 464(%rsp), %rdi
    leaq 400(%rsp), %rsi
    call sha256_transform
    movl 464(%rsp), %edi
    cmpl $3128432319, %edi
je .LBB26
.LBB16:
    xorl %esi, %esi
    jmp .LBB17
.LBB26:
    movl 468(%rsp), %r10d
    cmpl $2399260650, %r10d
je .LBB28
.LBB27:
    xorl %esi, %esi
    jmp .LBB17
.LBB28:
    movl 492(%rsp), %r9d
    cmpl $4060091821, %r9d
    movl $0, %ecx
    movl $1, %esi
    cmovnel %ecx, %esi
.LBB17:
    movl $1664525, %eax
    movd %eax, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqu %xmm0, 304(%rsp)
    movl $1664525, %eax
    movd %eax, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqu %xmm0, 272(%rsp)
    movl $1664525, %eax
    movd %eax, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqu %xmm0, 240(%rsp)
    movl $1664525, %eax
    movd %eax, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqu %xmm0, 208(%rsp)
    xorl %eax, %eax
    movd %eax, %xmm0
    movl $1013904223, %ecx
    movd %ecx, %xmm1
    punpckldq %xmm1, %xmm0
    movl $2027808446, %eax
    movl $3041712669, %ecx
    shlq $32, %rcx
    orq %rcx, %rax
    movq %rax, %xmm1
    punpcklqdq %xmm1, %xmm0
    movdqu %xmm0, 176(%rsp)
    leaq 512(%rsp), %rbx
    leaq 576(%rsp), %r12
    movl $4055616892, %eax
    movd %eax, %xmm0
    movl $774553819, %ecx
    movd %ecx, %xmm1
    punpckldq %xmm1, %xmm0
    movl $1788458042, %eax
    movl $2802362265, %ecx
    shlq $32, %rcx
    orq %rcx, %rax
    movq %rax, %xmm1
    punpcklqdq %xmm1, %xmm0
    movdqu %xmm0, 144(%rsp)
    leaq 528(%rsp), %r13
    movl $3816266488, %eax
    movd %eax, %xmm0
    movl $535203415, %ecx
    movd %ecx, %xmm1
    punpckldq %xmm1, %xmm0
    movl $1549107638, %eax
    movl $2563011861, %ecx
    shlq $32, %rcx
    orq %rcx, %rax
    movq %rax, %xmm1
    punpcklqdq %xmm1, %xmm0
    movdqu %xmm0, 112(%rsp)
    leaq 544(%rsp), %r14
    movl $3576916084, %eax
    movd %eax, %xmm0
    movl $295853011, %ecx
    movd %ecx, %xmm1
    punpckldq %xmm1, %xmm0
    movl $1309757234, %eax
    movl $2323661457, %ecx
    shlq $32, %rcx
    orq %rcx, %rax
    movq %rax, %xmm1
    punpcklqdq %xmm1, %xmm0
    movdqu %xmm0, 80(%rsp)
    testl %esi, %esi
je .LBB25
.LBB18:
    xorl %r11d, %r11d
    movq $0, 72(%rsp)
    jmp .LBB19
.LBB25:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $600, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB19:
    movq 72(%rsp), %rax
    cmpl $8, %eax
jae .LBB24
.LBB20:
    movq 72(%rsp), %rax
    xorq $1779033703, %rax
    movl %eax, 560(%rsp)
    movl $3144134277, 564(%rsp)
    movl $1013904242, 568(%rsp)
    movl $2773480762, 572(%rsp)
    movl $1359893119, (%r12)
    movl $2600822924, 580(%rsp)
    movl $528734635, 584(%rsp)
    movl $1541459225, 588(%rsp)
    movq %r11, %rbp
    xorl %r15d, %r15d
.LBB21:
    cmpl $131072, %r15d
jb .LBB23
.LBB22:
    movq 72(%rsp), %rax
    addl $1, %eax
    movq %rax, 72(%rsp)
    movq %rbp, %r11
    movq 72(%rsp), %rax
    cmpl $8, %eax
jb .LBB20
.LBB24:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1
.LBB23:
    movd %r15d, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqa %xmm0, %xmm15
    movd %r15d, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqa %xmm0, %xmm14
    movd %r15d, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    movq %r15, %rax
    movd %r15d, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqa %xmm0, %xmm12
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm12, %xmm12
    movdqu 176(%rsp), %xmm1
    vpaddd %xmm1, %xmm12, %xmm12
    movdqu 560(%rsp), %xmm11
    pxor %xmm11, %xmm12
    movdqu %xmm12, 496(%rsp)
    movdqu 240(%rsp), %xmm1
    vpmulld %xmm1, %xmm13, %xmm13
    movdqu 144(%rsp), %xmm1
    vpaddd %xmm1, %xmm13, %xmm13
    vpxor (%r12), %xmm13, %xmm13
    movdqu %xmm13, (%rbx)
    movdqu 272(%rsp), %xmm1
    vpmulld %xmm1, %xmm14, %xmm14
    movdqu 112(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    leaq 560(%rsp), %rax
    movdqu (%rax), %xmm13
    pxor %xmm13, %xmm14
    movdqu %xmm14, (%r13)
    movdqu 304(%rsp), %xmm1
    vpmulld %xmm1, %xmm15, %xmm15
    movdqu 80(%rsp), %xmm1
    vpaddd %xmm1, %xmm15, %xmm15
    vpxor (%r12), %xmm15, %xmm15
    movdqu %xmm15, (%r14)
    movq %rax, %rdi
    leaq 496(%rsp), %rsi
    call sha256_transform
    movl 560(%rsp), %edi
    shlq $32, %rdi
    movl 588(%rsp), %eax
    xorq %rax, %rdi
    movl 572(%rsp), %r11d
    shlq $16, %r11
    xorq %r11, %rdi
    addq %rdi, %rbp
    addl $1, %r15d
    cmpl $131072, %r15d
jb .LBB23
    jmp .LBB22


