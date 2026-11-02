.text
.globl _start
_start:
    movb %gs:0x1(%rax), %cl
    movw %gs:0x2(%rax), %cx
    movl %gs:0x4(%rax), %ecx
    movq %gs:0x8(%rax), %rcx
    movb %cl, %gs:0x1(%rax)
    movw %cx, %gs:0x2(%rax)
    movl %ecx, %gs:0x4(%rax)
    movq %rcx, %gs:0x8(%rax)
    movb $7, %gs:0x1(%rax)
    movw $7, %gs:0x2(%rax)
    movl $7, %gs:0x4(%rax)
    movq $7, %gs:0x8(%rax)
    addb %gs:(%rax), %cl
    addw %gs:(%rax), %cx
    addl %gs:(%rax), %ecx
    addq %gs:(%rax), %rcx
    subb %gs:(%rax), %cl
    subw %gs:(%rax), %cx
    subl %gs:(%rax), %ecx
    subq %gs:(%rax), %rcx
    andb %gs:(%rax), %cl
    andw %gs:(%rax), %cx
    andl %gs:(%rax), %ecx
    andq %gs:(%rax), %rcx
    orb %gs:(%rax), %cl
    orw %gs:(%rax), %cx
    orl %gs:(%rax), %ecx
    orq %gs:(%rax), %rcx
    xorb %gs:(%rax), %cl
    xorw %gs:(%rax), %cx
    xorl %gs:(%rax), %ecx
    xorq %gs:(%rax), %rcx
    adcb %gs:(%rax), %cl
    adcw %gs:(%rax), %cx
    adcl %gs:(%rax), %ecx
    adcq %gs:(%rax), %rcx
    sbbb %gs:(%rax), %cl
    sbbw %gs:(%rax), %cx
    sbbl %gs:(%rax), %ecx
    sbbq %gs:(%rax), %rcx
    cmpb %gs:(%rax), %cl
    cmpw %gs:(%rax), %cx
    cmpl %gs:(%rax), %ecx
    cmpq %gs:(%rax), %rcx
    testb %gs:(%rax), %cl
    testw %gs:(%rax), %cx
    testl %gs:(%rax), %ecx
    testq %gs:(%rax), %rcx
    incb %gs:(%rax)
    incw %gs:(%rax)
    incl %gs:(%rax)
    incq %gs:(%rax)
    decb %gs:(%rax)
    decw %gs:(%rax)
    decl %gs:(%rax)
    decq %gs:(%rax)
    negb %gs:(%rax)
    negw %gs:(%rax)
    negl %gs:(%rax)
    negq %gs:(%rax)
    notb %gs:(%rax)
    notw %gs:(%rax)
    notl %gs:(%rax)
    notq %gs:(%rax)
    imulb %gs:(%rax)
    imulw %gs:(%rax)
    imull %gs:(%rax)
    imulq %gs:(%rax)
    imull %gs:(%rax), %ecx
    imulq %gs:(%rax), %rcx
    idivb %gs:(%rax)
    idivw %gs:(%rax)
    idivl %gs:(%rax)
    idivq %gs:(%rax)
    shlb $1, %gs:(%rax)
    shlw $1, %gs:(%rax)
    shll $1, %gs:(%rax)
    shlq $1, %gs:(%rax)
    shrb $1, %gs:(%rax)
    shrw $1, %gs:(%rax)
    shrl $1, %gs:(%rax)
    shrq $1, %gs:(%rax)
    sarb $1, %gs:(%rax)
    sarw $1, %gs:(%rax)
    sarl $1, %gs:(%rax)
    sarq $1, %gs:(%rax)
    rolb $1, %gs:(%rax)
    rolw $1, %gs:(%rax)
    roll $1, %gs:(%rax)
    rolq $1, %gs:(%rax)
    rorb $1, %gs:(%rax)
    rorw $1, %gs:(%rax)
    rorl $1, %gs:(%rax)
    rorq $1, %gs:(%rax)
    rclb $1, %gs:(%rax)
    rcrb $1, %gs:(%rax)
    shlq $5, %gs:(%rax)
    shrb %cl, %gs:(%rax)
    btl %ecx, %gs:(%rax)
    btsq %rcx, %gs:(%rax)
    btq %rcx, %gs:(%rax)
    btl $3, %gs:(%rax)
    btsl $3, %gs:(%rax)
    btrl $3, %gs:(%rax)
    btcl $3, %gs:(%rax)
    btq $63, %gs:0x10(%rax)
    btsq $63, %gs:0x10(%rax)
    cmpxchgb %cl, %gs:(%rax)
    cmpxchgw %cx, %gs:(%rax)
    cmpxchgl %ecx, %gs:(%rax)
    cmpxchgq %rcx, %gs:(%rax)
    cmpxchg8b %gs:(%rax)
    cmpxchg16b %gs:(%rax)
    xchgb %cl, %gs:(%rax)
    xchgw %cx, %gs:(%rax)
    xchgl %ecx, %gs:(%rax)
    xchgq %rcx, %gs:(%rax)
    xaddb %cl, %gs:(%rax)
    xaddw %cx, %gs:(%rax)
    xaddl %ecx, %gs:(%rax)
    xaddq %rcx, %gs:(%rax)
    movzbw %gs:(%rax), %cx
    movzbl %gs:(%rax), %ecx
    movzbq %gs:(%rax), %rcx
    movzwl %gs:(%rax), %ecx
    movzwq %gs:(%rax), %rcx
    movsbw %gs:(%rax), %cx
    movsbl %gs:(%rax), %ecx
    movsbq %gs:(%rax), %rcx
    movswl %gs:(%rax), %ecx
    movswq %gs:(%rax), %rcx
    movslq %gs:(%rax), %rcx
    leaw %gs:(%rax), %cx
    leal %gs:(%rax), %ecx
    leaq %gs:(%rax), %rcx
    pushq %gs:(%rax)
    popq %gs:(%rax)
    lock incq %gs:(%rax)
    lock decq %gs:(%rax)
    lock addl %ecx, %gs:(%rax)
    lock subl %ecx, %gs:(%rax)
    lock andl %ecx, %gs:(%rax)
    lock orl %ecx, %gs:(%rax)
    lock xaddl %ecx, %gs:(%rax)
    lock xaddq %rcx, %gs:(%rax)
    lock cmpxchgl %ecx, %gs:(%rax)
    lock cmpxchgq %rcx, %gs:(%rax)
    lock cmpxchg8b %gs:(%rax)
    lock btsl $3, %gs:(%rax)
    clflush %gs:(%rax)
    clflushopt %gs:(%rax)
    prefetcht0 %gs:(%rax)
    prefetcht1 %gs:(%rax)
    prefetcht2 %gs:(%rax)
    prefetchnta %gs:(%rax)
    fxsave %gs:(%rax)
    fxrstor %gs:(%rax)
    flds %gs:(%rax)
    fldl %gs:(%rax)
    fldt %gs:(%rax)
    fildl %gs:(%rax)
    fildq %gs:(%rax)
    fists %gs:(%rax)
    fistl %gs:(%rax)
    fistpq %gs:(%rax)
    fstps %gs:(%rax)
    fstpl %gs:(%rax)
    fstpt %gs:(%rax)
    fadds %gs:(%rax)
    faddl %gs:(%rax)
    fcomps %gs:(%rax)
    movd %gs:(%rax), %mm0
    movq %gs:(%rax), %mm0
    movd %gs:(%rax), %xmm0
    movq %gs:(%rax), %xmm0
    movd %xmm0, %gs:(%rax)
    movq %xmm0, %gs:(%rax)
    movaps %gs:(%rax), %xmm0
    movapd %gs:(%rax), %xmm0
    movups %gs:(%rax), %xmm0
    movupd %gs:(%rax), %xmm0
    movaps %xmm0, %gs:(%rax)
    movups %xmm0, %gs:(%rax)
    movss %gs:(%rax), %xmm0
    movsd %gs:(%rax), %xmm0
    movss %xmm0, %gs:(%rax)
    movsd %xmm0, %gs:(%rax)
    movhps %gs:(%rax), %xmm0
    movlps %gs:(%rax), %xmm0
    movdqa %gs:(%rax), %xmm0
    movdqu %gs:(%rax), %xmm0
    movdqa %xmm0, %gs:(%rax)
    movdqu %xmm0, %gs:(%rax)
    addps %gs:(%rax), %xmm0
    addpd %gs:(%rax), %xmm0
    addss %gs:(%rax), %xmm0
    addsd %gs:(%rax), %xmm0
    mulss %gs:(%rax), %xmm0
    mulsd %gs:(%rax), %xmm0
    subss %gs:(%rax), %xmm0
    subsd %gs:(%rax), %xmm0
    cvtsi2ss %gs:(%rax), %xmm0
    cvtsi2sd %gs:(%rax), %xmm0
    cvtsi2sdq %gs:(%rax), %xmm0
    cvttss2si %gs:(%rax), %eax
    cvttsd2si %gs:(%rax), %eax
    comiss %gs:(%rax), %xmm0
    ucomiss %gs:(%rax), %xmm0
    comisd %gs:(%rax), %xmm0
    ucomisd %gs:(%rax), %xmm0
    pminub %gs:(%rax), %mm0
    emms
    vmovdqa %gs:(%rax), %xmm0
    vmovdqu %gs:(%rax), %xmm0
    vmovaps %gs:(%rax), %ymm0
    vmovups %gs:(%rax), %ymm0
    vmovaps %ymm0, %gs:(%rax)
    vmovups %ymm0, %gs:(%rax)
    vmovdqa %ymm0, %gs:(%rax)
    vpaddd %gs:(%rax), %ymm1, %ymm2
    vpsubb %gs:(%rax), %ymm1, %ymm2
    vaddsd %gs:(%rax), %xmm1, %xmm2
    vmulps %gs:(%rax), %ymm1, %ymm2
    vandpd %gs:(%rax), %ymm1, %ymm2
    vcvtsi2sd %gs:(%rax), %xmm1, %xmm2
    vcvtsi2sdq %gs:(%rax), %xmm1, %xmm2
    vpmovzxbd %gs:(%rax), %xmm0
    vbroadcastss %gs:(%rax), %ymm0
    vmovd %gs:(%rax), %xmm0
    vmovq %gs:(%rax), %xmm0
    vmovq %gs:(%rax), %xmm1
    # fs + control forms
    movl %fs:0x30, %eax
    movq %fs:0x0, %rax
    movl %es:(%rax), %eax
    movl %cs:(%rax), %eax
    movl %ss:(%rax), %eax
    movl %ds:(%rax), %eax
    # SIB + scale + disp
    movq %gs:(%rbx,%rcx,8), %rax
    movq %gs:0x42(%rbx,%rcx,4), %rax
    movq %fs:(%r8,%r9,2), %r10
    # addr32
    movl %fs:0x40(%r8d), %eax
    movl %fs:(%ecx), %eax
    # negative disp
    movq %gs:-0x8(%rax), %rcx
