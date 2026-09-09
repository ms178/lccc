.section .rodata
.Lstr0:
    .byte 37, 48, 49, 54, 108, 108, 120, 10, 0

.section .rodata
.align 16
.type K, @object
.size K, 256
K:
    .long 1116352408
    .long 1899447441
    .long 3049323471
    .long 3921009573
    .long 961987163
    .long 1508970993
    .long 2453635748
    .long 2870763221
    .long 3624381080
    .long 310598401
    .long 607225278
    .long 1426881987
    .long 1925078388
    .long 2162078206
    .long 2614888103
    .long 3248222580
    .long 3835390401
    .long 4022224774
    .long 264347078
    .long 604807628
    .long 770255983
    .long 1249150122
    .long 1555081692
    .long 1996064986
    .long 2554220882
    .long 2821834349
    .long 2952996808
    .long 3210313671
    .long 3336571891
    .long 3584528711
    .long 113926993
    .long 338241895
    .long 666307205
    .long 773529912
    .long 1294757372
    .long 1396182291
    .long 1695183700
    .long 1986661051
    .long 2177026350
    .long 2456956037
    .long 2730485921
    .long 2820302411
    .long 3259730800
    .long 3345764771
    .long 3516065817
    .long 3600352804
    .long 4094571909
    .long 275423344
    .long 430227734
    .long 506948616
    .long 659060556
    .long 883997877
    .long 958139571
    .long 1322822218
    .long 1537002063
    .long 1747873779
    .long 1955562222
    .long 2024104815
    .long 2227730452
    .long 2361852424
    .long 2428436474
    .long 2756734187
    .long 3204031479
    .long 3329325298

.text
.p2align 4
.type sha256_transform, @function
sha256_transform:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $392, %rsp
    .cfi_def_cfa_offset 448
    # LCCC_RET_XMM 0
    movq %rdi, 376(%rsp)
    leaq 120(%rsp), %rdi
    subq %rsi, %rdi
    subq $4, %rdi
    cmpq $25, %rdi
jb .LBB2
.LBB1:
    xorl %r8d, %r8d
    jmp .LBB3
.LBB2:
    xorl %r9d, %r9d
    jmp .LBB13
.LBB3:
    cmpq $64, %r8
jge .LBB12
.p2align 5,,15
.p2align 4
.LBB4:
    vmovdqu (%rsi,%r8), %ymm2
    vmovdqu %ymm2, 120(%rsp,%r8)
    addq $32, %r8
    cmpq $64, %r8
jl .LBB4
    jmp .LBB12
.LBB5:
    leaq 184(%rsp), %r11
    movl $16, %r10d
.LBB6:
    cmpq $64, %r10
jge .LBB8
.p2align 4,,10
.p2align 3
.LBB7:
    movl 112(%rsp, %r10, 4), %edx
    movl %edx, %r8d
    roll $15, %r8d
    movl %edx, %r9d
    roll $13, %r9d
    xorl %r9d, %r8d
    shrl $10, %edx
    xorl %edx, %r8d
    addl 92(%rsp, %r10, 4), %r8d
    movl 60(%rsp, %r10, 4), %esi
    movl %esi, %edx
    rorl $7, %edx
    movl %esi, %r9d
    roll $14, %r9d
    xorl %r9d, %edx
    shrl $3, %esi
    xorl %esi, %edx
    addl %edx, %r8d
    movl 56(%rsp, %r10, 4), %edx
    addl %r8d, %edx
    movl %edx, (%r11)
    addq $1, %r10
    leaq 4(%r11), %r11
    cmpq $64, %r10
jl .LBB7
.LBB8:
    movq 376(%rsp), %rcx
    movl (%rcx), %eax
    movq %rax, 112(%rsp)
    movq 376(%rsp), %rax
    leaq 4(%rax), %rax
    movl (%rax), %r9d
    movq 376(%rsp), %rax
    leaq 8(%rax), %rax
    movl (%rax), %r10d
    movq 376(%rsp), %rax
    leaq 12(%rax), %rax
    movl (%rax), %esi
    movq 376(%rsp), %rax
    leaq 16(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    movq %rax, 104(%rsp)
    movq 376(%rsp), %rax
    leaq 20(%rax), %rax
    movl (%rax), %r11d
    movq 376(%rsp), %rax
    leaq 24(%rax), %rax
    movl (%rax), %edx
    movq 376(%rsp), %rax
    leaq 28(%rax), %rax
    movl (%rax), %edi
    leaq K(%rip), %rbx
    movl %r9d, %eax
    andl %r10d, %eax
    movq %rax, 96(%rsp)
    movq %rdi, %r13
    movq %r9, %r14
    xorl %r15d, %r15d
    jmp .LBB9
.p2align 4,,10
.p2align 3
.LBB9:
    movl 112(%rsp), %ebp
    movq %r13, 88(%rsp)
    movq 104(%rsp), %rax
    movq %rax, 64(%rsp)
    movq %r14, 40(%rsp)
    movq %r11, 72(%rsp)
    movq %r10, 48(%rsp)
    movq %rdx, 80(%rsp)
    movq %rsi, 56(%rsp)
    cmpq $64, %r15
jge .LBB11
.LBB10:
    movl %eax, %r8d
    rorl $6, %r8d
    movl %eax, %r12d
    rorl $11, %r12d
    xorl %r12d, %r8d
    movl %eax, %r12d
    roll $7, %r12d
    xorl %r12d, %r8d
    movl %r13d, %r12d
    addl %r8d, %r12d
    movl %eax, %r8d
    andl 72(%rsp), %r8d
    notl %eax
    andl 80(%rsp), %eax
    movl %eax, 36(%rsp)
    xorl 36(%rsp), %r8d
    addl %r8d, %r12d
    movl (%rbx, %r15, 4), %eax
    movl %eax, 36(%rsp)
    addl 36(%rsp), %r12d
    movl %r12d, %edi
    addl 120(%rsp, %r15, 4), %edi
    movl %ebp, %r8d
    rorl $2, %r8d
    movl %ebp, %r12d
    rorl $13, %r12d
    xorl %r12d, %r8d
    movl %ebp, %r12d
    roll $10, %r12d
    xorl %r12d, %r8d
    movl %ebp, %r9d
    andl 40(%rsp), %r9d
    movl %ebp, %r12d
    andl 48(%rsp), %r12d
    xorl %r9d, %r12d
    xorl 96(%rsp), %r12d
    addl %r12d, %r8d
    movl %edi, %eax
    addl 56(%rsp), %eax
    movq %rax, 104(%rsp)
    movl %edi, %eax
    addl %r8d, %eax
    movq %rax, 112(%rsp)
    addq $1, %r15
    movq 80(%rsp), %r13
    movq %rbp, %r14
    movq 64(%rsp), %r11
    movq 40(%rsp), %r10
    movq 72(%rsp), %rdx
    movq 48(%rsp), %rsi
    movq %r9, 96(%rsp)
    jmp .LBB9
.LBB11:
    movq 376(%rsp), %rcx
    movl %ebp, %eax
    addl (%rcx), %eax
    movq 376(%rsp), %rcx
    movl %eax, (%rcx)
    movq 376(%rsp), %rax
    leaq 4(%rax), %rax
    movq %rax, %r11
    movl (%rax), %eax
    addl 40(%rsp), %eax
    movl %eax, (%r11)
    movq 376(%rsp), %rax
    leaq 8(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    addl 48(%rsp), %eax
    movl %eax, (%rdi)
    movq 376(%rsp), %rax
    leaq 12(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    addl 56(%rsp), %eax
    movl %eax, (%rdx)
    movq 376(%rsp), %rax
    leaq 16(%rax), %rax
    movq %rax, %r9
    movl (%rax), %eax
    addl 64(%rsp), %eax
    movl %eax, (%r9)
    movq 376(%rsp), %rax
    leaq 20(%rax), %rax
    movq %rax, %r10
    movl (%rax), %eax
    addl 72(%rsp), %eax
    movl %eax, (%r10)
    movq 376(%rsp), %rsi
    leaq 24(%rsi), %rsi
    movl (%rsi), %edx
    movl %edx, %eax
    addl 80(%rsp), %eax
    movl %eax, (%rsi)
    movq 376(%rsp), %rax
    leaq 28(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    addl 88(%rsp), %eax
    movl %eax, (%r8)
    vzeroupper
    addq $392, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB12:
    shrq $2, %r8
    movslq %r8d, %r9
.LBB13:
    cmpl $16, %r9d
jge .LBB5
.p2align 4,,10
.p2align 3
.LBB14:
    movslq %r9d, %r11
    movl (%rsi, %r11, 4), %eax
    movl %eax, 120(%rsp, %r11, 4)
    addl $1, %r9d
    cmpl $16, %r9d
jl .LBB14
    jmp .LBB5
.cfi_endproc
.size sha256_transform, .-sha256_transform

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
    subq $376, %rsp
    .cfi_def_cfa_offset 432
    # LCCC_RET_XMM 0
    movl $1779033703, 240(%rsp)
    movl $3144134277, 244(%rsp)
    movl $1013904242, 248(%rsp)
    movl $2773480762, 252(%rsp)
    movl $1359893119, 256(%rsp)
    movl $2600822924, 260(%rsp)
    movl $528734635, 264(%rsp)
    movl $1541459225, 268(%rsp)
    movl $1633837952, 176(%rsp)
    movl $0, 180(%rsp)
    movl $0, 184(%rsp)
    movl $0, 188(%rsp)
    movl $0, 192(%rsp)
    movl $0, 196(%rsp)
    movl $0, 200(%rsp)
    movl $0, 204(%rsp)
    movl $0, 208(%rsp)
    movl $0, 212(%rsp)
    movl $0, 216(%rsp)
    movl $0, 220(%rsp)
    movl $0, 224(%rsp)
    movl $0, 228(%rsp)
    movl $0, 232(%rsp)
    movl $24, 236(%rsp)
    leaq 240(%rsp), %rdi
    leaq 176(%rsp), %rsi
    call sha256_transform
    movl 240(%rsp), %r10d
    cmpl $3128432319, %r10d
je .LBB17
.LBB16:
    xorl %r8d, %r8d
    jmp .LBB20
.LBB17:
    movl 244(%rsp), %edi
    cmpl $2399260650, %edi
je .LBB19
.LBB18:
    xorl %r8d, %r8d
    jmp .LBB20
.LBB19:
    movl 268(%rsp), %edx
    cmpl $4060091821, %edx
    movl $0, %ecx
    movl $1, %r8d
    cmovnel %ecx, %r8d
.LBB20:
    testl %r8d, %r8d
je .LBB22
.LBB21:
    xorl %r11d, %r11d
    movq $0, 168(%rsp)
    jmp .LBB23
.LBB22:
    movl $2, %eax
.L__lccc_epilogue_1:
    addq $376, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB23:
    movq 168(%rsp), %rax
    cmpl $8, %eax
jae .LBB28
.p2align 4,,10
.p2align 3
.LBB24:
    movq 168(%rsp), %rax
    xorq $1779033703, %rax
    movl %eax, 336(%rsp)
    movl $3144134277, 340(%rsp)
    movl $1013904242, 344(%rsp)
    movl $2773480762, 348(%rsp)
    movl $1359893119, 352(%rsp)
    movl $2600822924, 356(%rsp)
    movl $528734635, 360(%rsp)
    movl $1541459225, 364(%rsp)
    movq %r11, 160(%rsp)
    movq $0, 152(%rsp)
.LBB25:
    movq 152(%rsp), %rax
    cmpl $131072, %eax
jb .LBB27
.LBB26:
    movq 168(%rsp), %rax
    addl $1, %eax
    movq %rax, 168(%rsp)
    movq 160(%rsp), %r11
    cmpl $8, %eax
jb .LBB24
    jmp .LBB28
.p2align 4,,10
.p2align 3
.LBB27:
    movq 152(%rsp), %r10
    imull $1664525, %r10d, %r10d
    movl 336(%rsp), %r8d
    movl %r10d, %eax
    xorl %r8d, %eax
    movl %eax, 272(%rsp)
    leal 1013904223(%r10), %r9d
    movl 340(%rsp), %edi
    movl %r9d, %eax
    xorl %edi, %eax
    movl %eax, 276(%rsp)
    leal 2027808446(%r10), %esi
    movl 344(%rsp), %edx
    movl %esi, %eax
    xorl %edx, %eax
    movl %eax, 280(%rsp)
    leal -1253254627(%r10), %r8d
    movl 348(%rsp), %r9d
    movl %r8d, %eax
    xorl %r9d, %eax
    movl %eax, 284(%rsp)
    leal -239350404(%r10), %edi
    movl 352(%rsp), %esi
    movl %edi, %eax
    xorl %esi, %eax
    movl %eax, 288(%rsp)
    leal 774553819(%r10), %edx
    movl 356(%rsp), %r8d
    movl %edx, %eax
    xorl %r8d, %eax
    movl %eax, 292(%rsp)
    leal 1788458042(%r10), %r9d
    movl 360(%rsp), %edi
    movl %r9d, %eax
    xorl %edi, %eax
    movl %eax, 296(%rsp)
    leal -1492605031(%r10), %esi
    movl 364(%rsp), %edx
    movl %esi, %eax
    xorl %edx, %eax
    movl %eax, 300(%rsp)
    leal -478700808(%r10), %r8d
    movl 336(%rsp), %r9d
    movl %r8d, %eax
    xorl %r9d, %eax
    movl %eax, 304(%rsp)
    leal 535203415(%r10), %edi
    movl 340(%rsp), %esi
    movl %edi, %eax
    xorl %esi, %eax
    movl %eax, 308(%rsp)
    leal 1549107638(%r10), %edx
    movl 344(%rsp), %r8d
    movl %edx, %eax
    xorl %r8d, %eax
    movl %eax, 312(%rsp)
    leal -1731955435(%r10), %r9d
    movl 348(%rsp), %edi
    movl %r9d, %eax
    xorl %edi, %eax
    movl %eax, 316(%rsp)
    leal -718051212(%r10), %esi
    movl 352(%rsp), %edx
    movl %esi, %eax
    xorl %edx, %eax
    movl %eax, 320(%rsp)
    leal 295853011(%r10), %r8d
    movl 356(%rsp), %r9d
    movl %r8d, %eax
    xorl %r9d, %eax
    movl %eax, 324(%rsp)
    leal 1309757234(%r10), %edi
    movl 360(%rsp), %esi
    movl %edi, %eax
    xorl %esi, %eax
    movl %eax, 328(%rsp)
    addl $-1971305839, %r10d
    movl 364(%rsp), %edx
    movl %r10d, %eax
    movl %edx, %ecx
    xorl %ecx, %eax
    movl %eax, 332(%rsp)
    leaq 336(%rsp), %rdi
    leaq 272(%rsp), %rsi
    call sha256_transform
    movl 336(%rsp), %r10d
    shlq $32, %r10
    movl 364(%rsp), %r8d
    xorq %r8, %r10
    movl 348(%rsp), %r9d
    shlq $16, %r9
    xorq %r9, %r10
    addq %r10, 160(%rsp)
    movq 152(%rsp), %rax
    addl $1, %eax
    movq %rax, 152(%rsp)
    cmpl $131072, %eax
jb .LBB27
    jmp .LBB26
.LBB28:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    jmp .L__lccc_epilogue_1
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
