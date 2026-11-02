.Lstr0:

K:

sha256_transform:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $280, %rsp
    movq %rdi, %rbx
    leaq 16(%rsp), %rdi
    subq %rsi, %rdi
    subq $4, %rdi
    cmpq $25, %rdi
jb .LBB14
.LBB1:
    xorl %r8d, %r8d
.LBB2:
    cmpq $64, %r8
jge .LBB4
.LBB3:
    vmovdqu (%rsi,%r8), %ymm2
    vmovdqu %ymm2, 16(%rsp,%r8)
    addq $32, %r8
    cmpq $64, %r8
jl .LBB3
.LBB4:
    shrq $2, %r8
    movslq %r8d, %r9
.LBB5:
    cmpl $16, %r9d
jge .LBB7
.LBB6:
    movslq %r9d, %r11
    movl (%rsi, %r11, 4), %eax
    movl %eax, 16(%rsp, %r11, 4)
    addl $1, %r9d
    cmpl $16, %r9d
jl .LBB6
.LBB7:
    movl $16, %r8d
.LBB8:
    cmpl $64, %r8d
jge .LBB10
.LBB9:
    movslq %r8d, %r9
    shlq $2, %r9
    addl $2, %r8d
    leaq 16(%rsp), %rcx
    addq %r9, %rcx
    leaq 16(%rsp), %rax
    vmovq -8(%rax,%r9), %xmm3
    vpslld $15, %xmm3, %xmm15
    vpsrld $17, %xmm3, %xmm14
    vpor %xmm14, %xmm15, %xmm15
    vpslld $13, %xmm3, %xmm14
    vpsrld $19, %xmm3, %xmm13
    vpor %xmm13, %xmm14, %xmm14
    vpxor %xmm14, %xmm15, %xmm15
    vpsrld $10, %xmm3, %xmm14
    vpxor %xmm14, %xmm15, %xmm15
    vmovq -28(%rax,%r9), %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq -60(%rax,%r9), %xmm4
    vpslld $25, %xmm4, %xmm14
    vpsrld $7, %xmm4, %xmm13
    vpor %xmm13, %xmm14, %xmm14
    vpslld $14, %xmm4, %xmm13
    vpsrld $18, %xmm4, %xmm12
    vpor %xmm12, %xmm13, %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    vpsrld $3, %xmm4, %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq -64(%rax,%r9), %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq %xmm15, (%rax,%r9)
    cmpl $64, %r8d
jl .LBB9
.LBB10:
    movl (%rbx), %r12d
    movl 4(%rbx), %esi
    movl 8(%rbx), %r8d
    movl 12(%rbx), %r13d
    movl 16(%rbx), %r14d
    movl 20(%rbx), %edi
    movl 24(%rbx), %r15d
    movl 28(%rbx), %ebp
    xorl %r10d, %r10d
    movq %rbp, %r11
    movq %r15, %rdx
.LBB11:
    cmpq $64, %r10
jge .LBB13
.LBB12:
    rorxl $6, %r14d, %r9d
    rorxl $11, %r14d, %r15d
    xorl %r15d, %r9d
    movq %r14, %rbp
    rorxl $25, %ebp, %ebp
    xorl %ebp, %r9d
    leal (%r11, %r9), %r15d
    movl %edi, %r9d
    xorl %edx, %r9d
    movq %r14, %rbp
    andl %r9d, %ebp
    xorl %edx, %ebp
    addl %ebp, %r15d
    leaq K(%rip), %rcx
    addl (%rcx, %r10, 4), %r15d
    movl 16(%rsp, %r10, 4), %r9d
    addl %r9d, %r15d
    rorxl $2, %r12d, %r9d
    movq %r12, %rbp
    rorxl $13, %ebp, %ebp
    xorl %ebp, %r9d
    movq %r12, %rbp
    rorxl $22, %ebp, %ebp
    xorl %ebp, %r9d
    movq %rsi, %rbp
    xorl %r8d, %ebp
    movl %r12d, %eax
    andl %ebp, %eax
    movl %eax, 12(%rsp)
    movq %rsi, %rbp
    andl %r8d, %ebp
    movq %rbp, %rax
    movl 12(%rsp), %ebp
    xorl %eax, %ebp
    addl %ebp, %r9d
    movq %r13, %rbp
    addl %r15d, %ebp
    addl %r9d, %r15d
    addq $1, %r10
    movq %rdx, %r11
    movq %rdi, %rdx
    movq %r8, %r13
    movq %r14, %rdi
    movq %rsi, %r8
    movq %rbp, %r14
    movq %r12, %rsi
    movq %r15, %r12
    cmpq $64, %r10
jl .LBB12
.LBB13:
    vmovdqu (%rbx), %xmm15
    vmovd %r12d, %xmm0
    movq %rsi, %rcx
    vmovd %esi, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %r8, %rax
    shlq $32, %r13
    orq %r13, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vmovdqa %xmm0, %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovdqu %xmm15, (%rbx)
    leaq 16(%rbx), %r10
    vmovdqu (%r10), %xmm15
    vmovd %r14d, %xmm0
    movq %rdi, %rcx
    vmovd %edi, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %rdx, %rax
    shlq $32, %r11
    orq %r11, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vmovdqa %xmm0, %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovdqu %xmm15, (%r10)
    vzeroupper
    addq $280, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB14:
    xorl %r9d, %r9d
    jmp .LBB5

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $504, %rsp
    movl $1779033703, 368(%rsp)
    movl $3144134277, 372(%rsp)
    movl $1013904242, 376(%rsp)
    movl $2773480762, 380(%rsp)
    movl $1359893119, 384(%rsp)
    movl $2600822924, 388(%rsp)
    movl $528734635, 392(%rsp)
    movl $1541459225, 396(%rsp)
    movl $1633837952, 304(%rsp)
    movl $0, 308(%rsp)
    movl $0, 312(%rsp)
    movl $0, 316(%rsp)
    pxor %xmm0, %xmm0
    movdqu %xmm0, 320(%rsp)
    movdqu %xmm0, 336(%rsp)
    movl $0, 352(%rsp)
    movl $0, 356(%rsp)
    movl $0, 360(%rsp)
    movl $24, 364(%rsp)
    leaq 368(%rsp), %rdi
    leaq 304(%rsp), %rsi
    call sha256_transform
    movl 368(%rsp), %edi
    cmpl $3128432319, %edi
je .LBB26
.LBB16:
    xorl %esi, %esi
    jmp .LBB17
.LBB26:
    movl 372(%rsp), %r10d
    cmpl $2399260650, %r10d
je .LBB28
.LBB27:
    xorl %esi, %esi
    jmp .LBB17
.LBB28:
    movl 396(%rsp), %r9d
    cmpl $4060091821, %r9d
    movl $0, %ecx
    movl $1, %esi
    cmovnel %ecx, %esi
.LBB17:
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
    leaq 416(%rsp), %rbx
    leaq 480(%rsp), %r12
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
    leaq 432(%rsp), %r13
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
    leaq 448(%rsp), %r14
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
    addq $504, %rsp
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
    movl %eax, 464(%rsp)
    movl $3144134277, 468(%rsp)
    movl $1013904242, 472(%rsp)
    movl $2773480762, 476(%rsp)
    movl $1359893119, (%r12)
    movl $2600822924, 484(%rsp)
    movl $528734635, 488(%rsp)
    movl $1541459225, 492(%rsp)
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
    movq %r15, %rax
    movd %r15d, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqa %xmm0, %xmm15
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm15, %xmm14
    movdqu 176(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    movdqu 464(%rsp), %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    movdqu %xmm14, 400(%rsp)
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm15, %xmm14
    movdqu 144(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    vpxor (%r12), %xmm14, %xmm14
    movdqu %xmm14, (%rbx)
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm15, %xmm14
    movdqu 112(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    leaq 464(%rsp), %rax
    movdqu (%rax), %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    movdqu %xmm14, (%r13)
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm15, %xmm15
    movdqu 80(%rsp), %xmm1
    vpaddd %xmm1, %xmm15, %xmm15
    vpxor (%r12), %xmm15, %xmm15
    movdqu %xmm15, (%r14)
    movq %rax, %rdi
    leaq 400(%rsp), %rsi
    call sha256_transform
    movl 464(%rsp), %edi
    shlq $32, %rdi
    movl 492(%rsp), %eax
    xorq %rax, %rdi
    movl 476(%rsp), %r11d
    shlq $16, %r11
    xorq %r11, %rdi
    addq %rdi, %rbp
    addl $1, %r15d
    cmpl $131072, %r15d
jb .LBB23
    jmp .LBB22


