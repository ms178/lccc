siphash24:
.cfi_startproc
    pushq %rbx
    .cfi_def_cfa_offset 16
    .cfi_offset %rbx, -16
    pushq %r12
    .cfi_def_cfa_offset 24
    .cfi_offset %r12, -24
    pushq %r13
    .cfi_def_cfa_offset 32
    .cfi_offset %r13, -32
    pushq %r14
    .cfi_def_cfa_offset 40
    .cfi_offset %r14, -40
    pushq %r15
    .cfi_def_cfa_offset 48
    .cfi_offset %r15, -48
    pushq %rbp
    .cfi_def_cfa_offset 56
    .cfi_offset %rbp, -56
    subq $120, %rsp
    .cfi_def_cfa_offset 176
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq %rdi, %rbx
    movq %rsi, %r12
    movq %rdx, %r13
    vpxor %ymm0, %ymm0, %ymm0
    vmovdqu %ymm0, 56(%rsp)
    movq $0, 88(%rsp)
    movq $0, 56(%rsp)
    movq $0, 64(%rsp)
    movq $0, 72(%rsp)
    movq $0, 80(%rsp)
    movb $0, 88(%rsp)
    movq $0, 96(%rsp)
    movq $0, 104(%rsp)
    movq (%rdx), %rax
    movabsq $8317987319222330741, %rcx
    xorq %rcx, %rax
    movq %rax, 56(%rsp)
    movq 8(%rdx), %rax
    movabsq $7237128888997146477, %rcx
    xorq %rcx, %rax
    movq %rax, 64(%rsp)
    movq (%rdx), %rax
    movabsq $7816392313619706465, %rcx
    xorq %rcx, %rax
    movq %rax, 72(%rsp)
    movq 8(%rdx), %rax
    movabsq $8387220255154660723, %rcx
    xorq %rcx, %rax
    movq %rax, 80(%rsp)
    leaq 88(%rsp), %rax
    movq %rax, 96(%rsp)
    movq $0, 104(%rsp)
    leaq 56(%rsp), %rdi
    movq %rbx, %rsi
    movq %r12, %rdx
    call sip24_update
    movq %rax, %r11
    movq 40(%r11), %r8
    leaq 32(%r11), %r9
    subq %r9, %r8
    movsbl %r8b, %edi
    movq 48(%r11), %r10
    movsbq %dil, %r8
    addq %r8, %r10
    shlq $56, %r10
    movsbq %dil, %r9
    movslq %r9d, %rax
    cmpq $8, %rax
    jae .LBB36
    leaq .Ljt_1(%rip), %rcx
    movslq (%rcx,%rax,4), %rdx
    addq %rcx, %rdx
    jmpq *%rdx
.section .rodata
.align 4
.Ljt_1:
    .long .LBB35 - .Ljt_1
    .long .LBB34 - .Ljt_1
    .long .LBB33 - .Ljt_1
    .long .LBB32 - .Ljt_1
    .long .LBB31 - .Ljt_1
    .long .LBB30 - .Ljt_1
    .long .LBB29 - .Ljt_1
    .long .LBB37 - .Ljt_1
.section .text,"ax",@progbits
.LBB29:
    movq %r10, %rdi
    jmp .LBB38
.LBB30:
    movq %r10, %rsi
    jmp .LBB39
.LBB31:
    movq %r10, %r8
    jmp .LBB40
.LBB32:
    movq %r10, %r9
    jmp .LBB41
.LBB33:
    movq %r10, %rdi
    jmp .LBB42
.LBB34:
    movq %r10, %rsi
    jmp .LBB43
.LBB35:
    movq %r10, %r8
    jmp .LBB44
.LBB36:
    movq %r10, %r9
    jmp .LBB45
.LBB37:
    movzbl 38(%r11), %r8d
    shlq $48, %r8
    orq %r8, %r10
    movq %r10, %rdi
.LBB38:
    movzbl 37(%r11), %r10d
    shlq $40, %r10
    orq %rdi, %r10
    movq %r10, %rsi
.LBB39:
    movzbl 36(%r11), %edi
    shlq $32, %rdi
    orq %rsi, %rdi
    movq %rdi, %r8
.LBB40:
    movzbl 35(%r11), %r10d
    shlq $24, %r10
    orq %r8, %r10
    movq %r10, %r9
.LBB41:
    movzbl 34(%r11), %r10d
    shlq $16, %r10
    orq %r9, %r10
    movq %r10, %rdi
.LBB42:
    movzbl 33(%r11), %r10d
    shlq $8, %r10
    orq %rdi, %r10
    movq %r10, %rsi
.LBB43:
    movzbl 32(%r11), %edi
    orq %rsi, %rdi
    movq %rdi, %r8
.LBB44:
    movq %r8, %r9
.LBB45:
    leaq 24(%r11), %rsi
    movq (%rsi), %r10
    xorq %r9, %r10
    movq 8(%r11), %rdi
    movq (%r11), %r8
    addq %rdi, %r8
    leaq 8(%r11), %r14
    rorxq $51, %rdi, %rdi
    xorq %r8, %rdi
    rorxq $32, %r8, %r8
    leaq 16(%r11), %r15
    movq (%r15), %rbp
    leaq (%rbp, %r10, 1), %rdx
    rorxq $48, %r10, %r10
    xorq %rdx, %r10
    addq %r10, %r8
    rorxq $43, %r10, %r10
    xorq %r8, %r10
    addq %rdi, %rdx
    rorxq $47, %rdi, %rdi
    xorq %rdx, %rdi
    rorxq $32, %rdx, %rdx
    addq %rdi, %r8
    rorxq $51, %rdi, %rdi
    xorq %r8, %rdi
    rorxq $32, %r8, %r8
    addq %r10, %rdx
    rorxq $48, %r10, %r10
    xorq %rdx, %r10
    addq %r10, %r8
    rorxq $43, %r10, %r10
    xorq %r8, %r10
    addq %rdi, %rdx
    rorxq $47, %rdi, %rdi
    xorq %rdx, %rdi
    rorxq $32, %rdx, %rdx
    xorq %r9, %r8
    movq %rdx, %r9
    xorq $255, %r9
    addq %rdi, %r8
    rorxq $51, %rdi, %rdi
    xorq %r8, %rdi
    rorxq $32, %r8, %r8
    addq %r10, %r9
    rorxq $48, %r10, %r10
    xorq %r9, %r10
    addq %r10, %r8
    rorxq $43, %r10, %r10
    xorq %r8, %r10
    addq %rdi, %r9
    rorxq $47, %rdi, %rdi
    xorq %r9, %rdi
    rorxq $32, %r9, %r9
    addq %rdi, %r8
    rorxq $51, %rdi, %rdi
    xorq %r8, %rdi
    rorxq $32, %r8, %r8
    addq %r10, %r9
    rorxq $48, %r10, %r10
    xorq %r9, %r10
    addq %r10, %r8
    rorxq $43, %r10, %r10
    xorq %r8, %r10
    addq %rdi, %r9
    rorxq $47, %rdi, %rdi
    xorq %r9, %rdi
    rorxq $32, %r9, %r9
    addq %rdi, %r8
    rorxq $51, %rdi, %rdi
    xorq %r8, %rdi
    rorxq $32, %r8, %r8
    addq %r10, %r9
    rorxq $48, %r10, %r10
    xorq %r9, %r10
    addq %r10, %r8
    rorxq $43, %r10, %r10
    xorq %r8, %r10
    addq %rdi, %r9
    rorxq $47, %rdi, %rdi
    xorq %r9, %rdi
    rorxq $32, %r9, %r9
    addq %rdi, %r8
    rorxq $51, %rdi, %rdi
    xorq %r8, %rdi
    rorxq $32, %r8, %r8
    addq %r10, %r9
    rorxq $48, %r10, %r10
    xorq %r9, %r10
    addq %r10, %r8
    movq %r8, (%r11)
    rorxq $43, %r10, %r10
    xorq %r8, %r10
    movq %r10, (%rsi)
    addq %rdi, %r9
    rorxq $47, %rdi, %rdi
    xorq %r9, %rdi
    movq %rdi, (%r14)
    rorxq $32, %r9, %r9
    movq %r9, (%r15)
    xorq %rdi, %r8
    xorq %r9, %r8
    xorq %r10, %r8
    movq %r8, %rax
    vzeroupper
    addq $120, %rsp
    .cfi_def_cfa_offset 56
    popq %rbp
    .cfi_def_cfa_offset 48
    popq %r15
    .cfi_def_cfa_offset 40
    popq %r14
    .cfi_def_cfa_offset 32
    popq %r13
    .cfi_def_cfa_offset 24
    popq %r12
    .cfi_def_cfa_offset 16
    popq %rbx
    .cfi_def_cfa_offset 8
    ret
