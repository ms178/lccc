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
    xorl %edi, %edi
.LBB1:
    cmpq $64, %rdi
jge .LBB3
.LBB2:
    vmovdqu (%rsi,%rdi), %ymm2
    vmovdqu %ymm2, 16(%rsp, %rdi)
    addq $32, %rdi
    cmpq $64, %rdi
jl .LBB2
.LBB3:
    shrq $2, %rdi
    movslq %edi, %r8
    leaq 0(,%r8,4), %r9
    leaq 16(%rsp), %rcx
    addq %r9, %rcx
    movq %rcx, %r11
    leaq (%rsi, %r9), %r10
.LBB4:
    cmpq $16, %r8
jge .LBB6
.LBB5:
    movl (%r10), %eax
    movl %eax, (%r11)
    addq $1, %r8
    leaq 4(%r11), %r11
    leaq 4(%r10), %r10
    cmpq $16, %r8
jl .LBB5
.LBB6:
    movl $16, %edi
.LBB7:
    cmpl $64, %edi
jge .LBB9
.LBB8:
    movslq %edi, %rsi
    shlq $2, %rsi
    addl $2, %edi
    leaq 16(%rsp), %rcx
    addq %rsi, %rcx
    leaq 16(%rsp), %rax
    vmovq -8(%rax,%rsi), %xmm3
    vpslld $15, %xmm3, %xmm15
    vpsrld $17, %xmm3, %xmm14
    vpor %xmm14, %xmm15, %xmm15
    vpslld $13, %xmm3, %xmm14
    vpsrld $19, %xmm3, %xmm13
    vpor %xmm13, %xmm14, %xmm14
    vpxor %xmm14, %xmm15, %xmm15
    vpsrld $10, %xmm3, %xmm14
    vpxor %xmm14, %xmm15, %xmm15
    vmovq -28(%rax,%rsi), %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq -60(%rax,%rsi), %xmm4
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
    vmovq -64(%rax,%rsi), %xmm14
    vpaddd %xmm14, %xmm15, %xmm15
    vmovq %xmm15, (%rax,%rsi)
    cmpl $64, %edi
jl .LBB8
.LBB9:
    movl (%rbx), %r12d
    movl 4(%rbx), %r11d
    movl 8(%rbx), %edi
    movl 12(%rbx), %r13d
    movl 16(%rbx), %r14d
    movl 20(%rbx), %r9d
    movl 24(%rbx), %r10d
    movl 28(%rbx), %ebp
    xorl %r8d, %r8d
    movq %rbp, %rdx
.LBB10:
    cmpq $64, %r8
jge .LBB12
    leaq K(%rip), %rcx
.LBB11:
    rorxl $6, %r14d, %esi
    rorxl $11, %r14d, %r15d
    xorl %r15d, %esi
    rorxl $25, %r14d, %ebp
    xorl %ebp, %esi
    leal (%rdx, %rsi), %r15d
    movl %r14d, %esi
    andl %r9d, %esi
    andnl %r10d, %r14d, %ebp
    xorl %ebp, %esi
    addl %esi, %r15d
    addl (%rcx, %r8, 4), %r15d
    addl 16(%rsp, %r8, 4), %r15d
    rorxl $2, %r12d, %esi
    rorxl $13, %r12d, %ebp
    xorl %ebp, %esi
    rorxl $22, %r12d, %ebp
    xorl %ebp, %esi
    movq %r12, %rbp
    andl %r11d, %ebp
    movl %r12d, %eax
    xorl %r11d, %eax
    andl %edi, %eax
    xorl %eax, %ebp
    addl %ebp, %esi
    movq %r13, %rbp
    addl %r15d, %ebp
    addl %esi, %r15d
    addq $1, %r8
    movq %r10, %rdx
    movq %r9, %r10
    movq %rdi, %r13
    movq %r14, %r9
    movq %r11, %rdi
    movq %rbp, %r14
    movq %r12, %r11
    movq %r15, %r12
    cmpq $64, %r8
jl .LBB11
.LBB12:
    vmovdqu (%rbx), %xmm15
    vmovd %r12d, %xmm0
    vmovd %r11d, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %rdi, %rax
    shlq $32, %r13
    orq %r13, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vpaddd %xmm0, %xmm15, %xmm15
    vmovdqu %xmm15, (%rbx)
    leaq 16(%rbx), %r8
    vmovdqu (%r8), %xmm15
    vmovd %r14d, %xmm0
    vmovd %r9d, %xmm1
    vpunpckldq %xmm1, %xmm0, %xmm0
    movq %r10, %rax
    shlq $32, %rdx
    orq %rdx, %rax
    vmovq %rax, %xmm1
    vpunpcklqdq %xmm1, %xmm0, %xmm0
    vpaddd %xmm0, %xmm15, %xmm15
    vmovdqu %xmm15, (%r8)
    vzeroupper
    addq $280, %rsp
    popq %rbp
    popq %r15
    popq %r14
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
je .LBB24
.LBB14:
    xorl %esi, %esi
    jmp .LBB15
.LBB24:
    movl 372(%rsp), %r10d
    cmpl $2399260650, %r10d
je .LBB26
.LBB25:
    xorl %esi, %esi
    jmp .LBB15
.LBB26:
    movl 396(%rsp), %r9d
    cmpl $4060091821, %r9d
    movl $0, %ecx
    movl $1, %esi
    cmovnel %ecx, %esi
.LBB15:
    movl $1664525, %eax
    movd %eax, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqu %xmm0, 208(%rsp)
    vmovdqa .LCVEC_0(%rip), %xmm0
    movdqu %xmm0, 176(%rsp)
    leaq 416(%rsp), %rbx
    leaq 480(%rsp), %r12
    vmovdqa .LCVEC_1(%rip), %xmm0
    movdqu %xmm0, 144(%rsp)
    leaq 432(%rsp), %r13
    vmovdqa .LCVEC_2(%rip), %xmm0
    movdqu %xmm0, 112(%rsp)
    leaq 448(%rsp), %r14
    vmovdqa .LCVEC_3(%rip), %xmm0
    movdqu %xmm0, 80(%rsp)
    testl %esi, %esi
je .LBB23
.LBB16:
    xorl %r11d, %r11d
    movq $0, 72(%rsp)
    jmp .LBB17
.LBB23:
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
.LBB17:
    movl 72(%rsp), %eax
    cmpl $8, %eax
jae .LBB22
.LBB18:
    movl 72(%rsp), %eax
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
.LBB19:
    cmpl $131072, %r15d
jb .LBB21
.LBB20:
    movl 72(%rsp), %eax
    addl $1, %eax
    movq %rax, 72(%rsp)
    movq %rbp, %r11
    movl 72(%rsp), %eax
    cmpl $8, %eax
jb .LBB18
.LBB22:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1
.LBB21:
    movd %r15d, %xmm0
    pshufd $0x00, %xmm0, %xmm0
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm0, %xmm14
    movdqu 176(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    movdqu 464(%rsp), %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    movdqu %xmm14, 400(%rsp)
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm0, %xmm14
    movdqu 144(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    vpxor (%r12), %xmm14, %xmm14
    movdqu %xmm14, (%rbx)
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm0, %xmm14
    movdqu 112(%rsp), %xmm1
    vpaddd %xmm1, %xmm14, %xmm14
    leaq 464(%rsp), %rax
    movdqu (%rax), %xmm13
    vpxor %xmm13, %xmm14, %xmm14
    movdqu %xmm14, (%r13)
    movdqu 208(%rsp), %xmm1
    vpmulld %xmm1, %xmm0, %xmm15
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
jb .LBB21
    jmp .LBB20

.LCVEC_0:
.LCVEC_1:
.LCVEC_2:
.LCVEC_3:
