.Lstr0:
.Lstr1:

sip24_valid.vectors.0:

sip24_init:
    movq (%rsi), %rax
    movabsq $8317987319222330741, %rcx
    xorq %rcx, %rax
    movq %rax, (%rdi)
    movq 8(%rsi), %rax
    movabsq $7237128888997146477, %rcx
    xorq %rcx, %rax
    movq %rax, 8(%rdi)
    movq (%rsi), %rax
    movabsq $7816392313619706465, %rcx
    xorq %rcx, %rax
    movq %rax, 16(%rdi)
    movq 8(%rsi), %rax
    movabsq $8387220255154660723, %rcx
    xorq %rcx, %rax
    movq %rax, 24(%rdi)
    leaq 32(%rdi), %rax
    movq %rax, 40(%rdi)
    movq $0, 48(%rdi)
    movq %rdi, %rax
    ret

sip24_update:
    pushq %rbx
    pushq %r12
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    leaq (%rsi, %rdx, 1), %r9
    leaq 40(%rdi), %r10
    leaq 32(%rdi), %r14
.LBB2:
.LBB3:
    cmpq %r9, %rsi
jae .LBB6
.LBB4:
    cmpq %r10, (%r10)
jae .LBB6
.LBB5:
    movq (%r10), %rdx
    leaq 1(%rdx), %rax
    movq %rax, (%r10)
    movzbl (%rsi), %eax
    movb %al, (%rdx)
    addq $1, %rsi
    cmpq %r9, %rsi
jb .LBB4
.LBB6:
    cmpq %r10, (%r10)
jb .LBB9
.LBB7:
    movq (%r14), %rdx
    movq 24(%rdi), %r15
    xorq %rdx, %r15
    movq 8(%rdi), %rbp
    movq (%rdi), %rbx
    leaq (%rbx, %rbp, 1), %r12
    rorxq $51, %rbp, %rbp
    xorq %r12, %rbp
    rorxq $32, %r12, %r12
    movq 16(%rdi), %rbx
    addq %r15, %rbx
    rorxq $48, %r15, %r15
    xorq %rbx, %r15
    addq %r15, %r12
    rorxq $43, %r15, %r15
    xorq %r12, %r15
    addq %rbp, %rbx
    rorxq $47, %rbp, %rbp
    xorq %rbx, %rbp
    rorxq $32, %rbx, %rbx
    addq %rbp, %r12
    rorxq $51, %rbp, %rbp
    xorq %r12, %rbp
    rorxq $32, %r12, %r12
    addq %r15, %rbx
    rorxq $48, %r15, %r15
    xorq %rbx, %r15
    addq %r15, %r12
    rorxq $43, %r15, %r15
    xorq %r12, %r15
    movq %r15, 24(%rdi)
    addq %rbp, %rbx
    rorxq $47, %rbp, %rbp
    movq %rbp, %rax
    xorq %rbx, %rax
    movq %rax, 8(%rdi)
    rorxq $32, %rbx, %rax
    movq %rax, 16(%rdi)
    xorq %rdx, %r12
    movq %r12, (%rdi)
    movq %r14, (%r10)
    addq $8, 48(%rdi)
    cmpq %r9, %rsi
jae .LBB9
.LBB8:
    jmp .LBB2
.LBB9:
    movq %rdi, %rax
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r12
    popq %rbx
    ret

sip24_final:
    pushq %rbx
    pushq %r13
    movq %rdi, %rbx
    movq 40(%rbx), %r10
    leaq 32(%rbx), %r8
    subq %r8, %r10
    movsbl %r10b, %r9d
    movq 48(%rbx), %rsi
    movsbq %r9b, %r11
    addq %r11, %rsi
    shlq $56, %rsi
    movsbq %r9b, %r10
    movslq %r10d, %rax
    cmpq $8, %rax
    jae .LBB18
    leaq .Ljt_0(%rip), %rcx
    movslq (%rcx,%rax,4), %rdx
    addq %rcx, %rdx
    jmpq *%rdx
.Ljt_0:
.LBB11:
    movq %rsi, %r8
    jmp .LBB20
.LBB12:
    movq %rsi, %r9
    jmp .LBB21
.LBB13:
    movq %rsi, %rdi
    jmp .LBB22
.LBB14:
    movq %rsi, %r11
    jmp .LBB23
.LBB15:
    movq %rsi, %r10
    jmp .LBB24
.LBB16:
    movq %rsi, %r8
    jmp .LBB25
.LBB17:
    movq %rsi, %r9
    jmp .LBB26
.LBB18:
    movq %rsi, %rdi
    jmp .LBB27
.LBB19:
    movzbl 38(%rbx), %r10d
    shlq $48, %r10
    orq %r10, %rsi
    movq %rsi, %r8
.LBB20:
    movzbl 37(%rbx), %esi
    shlq $40, %rsi
    orq %r8, %rsi
    movq %rsi, %r9
.LBB21:
    movzbl 36(%rbx), %r8d
    shlq $32, %r8
    orq %r9, %r8
    movq %r8, %rdi
.LBB22:
    movzbl 35(%rbx), %esi
    shlq $24, %rsi
    orq %rdi, %rsi
    movq %rsi, %r11
.LBB23:
    movzbl 34(%rbx), %r9d
    shlq $16, %r9
    orq %r11, %r9
    movq %r9, %r10
.LBB24:
    movzbl 33(%rbx), %esi
    shlq $8, %rsi
    orq %r10, %rsi
    movq %rsi, %r8
.LBB25:
    movzbl 32(%rbx), %edi
    orq %r8, %rdi
    movq %rdi, %r9
.LBB26:
    movq %r9, %rdi
.LBB27:
    movq 24(%rbx), %r11
    xorq %rdi, %r11
    movq 8(%rbx), %r8
    movq (%rbx), %r9
    addq %r8, %r9
    rorxq $51, %r8, %r8
    xorq %r9, %r8
    rorxq $32, %r9, %r9
    movq 16(%rbx), %r13
    leaq (%r13, %r11, 1), %r10
    rorxq $48, %r11, %r11
    xorq %r10, %r11
    addq %r11, %r9
    rorxq $43, %r11, %r11
    xorq %r9, %r11
    addq %r8, %r10
    rorxq $47, %r8, %r8
    xorq %r10, %r8
    rorxq $32, %r10, %r10
    addq %r8, %r9
    rorxq $51, %r8, %r8
    xorq %r9, %r8
    rorxq $32, %r9, %r9
    addq %r11, %r10
    rorxq $48, %r11, %r11
    xorq %r10, %r11
    addq %r11, %r9
    rorxq $43, %r11, %r11
    xorq %r9, %r11
    addq %r8, %r10
    rorxq $47, %r8, %r8
    xorq %r10, %r8
    rorxq $32, %r10, %r10
    xorq %rdi, %r9
    xorq $255, %r10
    addq %r8, %r9
    rorxq $51, %r8, %r8
    xorq %r9, %r8
    rorxq $32, %r9, %r9
    addq %r11, %r10
    rorxq $48, %r11, %r11
    xorq %r10, %r11
    addq %r11, %r9
    rorxq $43, %r11, %r11
    xorq %r9, %r11
    addq %r8, %r10
    rorxq $47, %r8, %r8
    xorq %r10, %r8
    rorxq $32, %r10, %r10
    addq %r8, %r9
    rorxq $51, %r8, %r8
    xorq %r9, %r8
    rorxq $32, %r9, %r9
    addq %r11, %r10
    rorxq $48, %r11, %r11
    xorq %r10, %r11
    addq %r11, %r9
    rorxq $43, %r11, %r11
    xorq %r9, %r11
    addq %r8, %r10
    rorxq $47, %r8, %r8
    xorq %r10, %r8
    rorxq $32, %r10, %r10
    addq %r8, %r9
    rorxq $51, %r8, %r8
    xorq %r9, %r8
    rorxq $32, %r9, %r9
    addq %r11, %r10
    rorxq $48, %r11, %r11
    xorq %r10, %r11
    addq %r11, %r9
    rorxq $43, %r11, %r11
    xorq %r9, %r11
    addq %r8, %r10
    rorxq $47, %r8, %r8
    xorq %r10, %r8
    rorxq $32, %r10, %r10
    addq %r8, %r9
    rorxq $51, %r8, %r8
    xorq %r9, %r8
    rorxq $32, %r9, %r9
    addq %r11, %r10
    rorxq $48, %r11, %r11
    xorq %r10, %r11
    addq %r11, %r9
    movq %r9, (%rbx)
    rorxq $43, %r11, %r11
    xorq %r9, %r11
    movq %r11, 24(%rbx)
    addq %r8, %r10
    rorxq $47, %r8, %r8
    xorq %r10, %r8
    movq %r8, 8(%rbx)
    rorxq $32, %r10, %r10
    movq %r10, 16(%rbx)
    xorq %r8, %r9
    xorq %r10, %r9
    xorq %r11, %r9
    movq %r9, %rax
    popq %r13
    popq %rbx
    ret

siphash24:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $120, %rsp
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
.Ljt_1:
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
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

expat_siphash24:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $120, %rsp
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
    jae .LBB54
    leaq .Ljt_2(%rip), %rcx
    movslq (%rcx,%rax,4), %rdx
    addq %rcx, %rdx
    jmpq *%rdx
.Ljt_2:
.LBB47:
    movq %r10, %rdi
    jmp .LBB56
.LBB48:
    movq %r10, %rsi
    jmp .LBB57
.LBB49:
    movq %r10, %r8
    jmp .LBB58
.LBB50:
    movq %r10, %r9
    jmp .LBB59
.LBB51:
    movq %r10, %rdi
    jmp .LBB60
.LBB52:
    movq %r10, %rsi
    jmp .LBB61
.LBB53:
    movq %r10, %r8
    jmp .LBB62
.LBB54:
    movq %r10, %r9
    jmp .LBB63
.LBB55:
    movzbl 38(%r11), %r8d
    shlq $48, %r8
    orq %r8, %r10
    movq %r10, %rdi
.LBB56:
    movzbl 37(%r11), %r10d
    shlq $40, %r10
    orq %rdi, %r10
    movq %r10, %rsi
.LBB57:
    movzbl 36(%r11), %edi
    shlq $32, %rdi
    orq %rsi, %rdi
    movq %rdi, %r8
.LBB58:
    movzbl 35(%r11), %r10d
    shlq $24, %r10
    orq %r8, %r10
    movq %r10, %r9
.LBB59:
    movzbl 34(%r11), %r10d
    shlq $16, %r10
    orq %r9, %r10
    movq %r10, %rdi
.LBB60:
    movzbl 33(%r11), %r10d
    shlq $8, %r10
    orq %rdi, %r10
    movq %r10, %rsi
.LBB61:
    movzbl 32(%r11), %edi
    orq %rsi, %rdi
    movq %rdi, %r8
.LBB62:
    movq %r8, %r9
.LBB63:
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
    subq $424, %rsp
    movabsq $506097522914230528, %rax
    movq %rax, 256(%rsp)
    movabsq $1084818905618843912, %rax
    movq %rax, 264(%rsp)
    movl $376, %r10d
    xorl %ebx, %ebx
.LBB65:
    cmpq $144, %rbx
jae .LBB67
.LBB66:
    imull $1664525, %r10d, %r8d
    leal 1013904223(%r8), %r10d
    movq %rbx, %r9
    movl %r10d, %esi
    shrl $24, %esi
    movb %sil, 272(%rsp, %r9)
    addq $1, %rbx
    cmpq $144, %rbx
jb .LBB66
.LBB67:
    leaq sip24_valid.vectors.0(%rip), %r12
    movq .Lstr0(%rip), %rax
    movq %rax, 120(%rsp)
    movq .Lstr0+8(%rip), %rax
    movq %rax, 128(%rsp)
    xorl %r13d, %r13d
.LBB68:
    cmpq $64, %r13
jae .LBB71
.LBB69:
    movzbl %r13b, %r9d
    movb %r9b, 136(%rsp, %r13)
    leaq 136(%rsp), %rdi
    leaq 120(%rsp), %rdx
    movq %r13, %rsi
    call siphash24
    cmpq (%r12, %r13, 8), %rax
jne .LBB85
.LBB70:
    addq $1, %r13
    cmpq $64, %r13
jb .LBB69
.LBB71:
    movl $1, %r10d
.LBB72:
    vpxor %ymm0, %ymm0, %ymm0
    vmovdqu %ymm0, 80(%rsp)
    vpxor %ymm0, %ymm0, %ymm0
    vmovdqu %ymm0, 32(%rsp)
    testl %r10d, %r10d
je .LBB84
.LBB73:
    xorl %r13d, %r13d
.LBB74:
    cmpq $128, %r13
ja .LBB80
.LBB75:
    leaq 272(%rsp), %rdi
    leaq 256(%rsp), %rdx
    movq %r13, %rsi
    call expat_siphash24@PLT
    movq %rax, 8(%rsp)
    xorl %r12d, %r12d
.LBB76:
    cmpq %r13, %r12
ja .LBB79
.LBB77:
    vmovdqu 80(%rsp), %ymm0
    vmovdqu %ymm0, 200(%rsp)
    movq $0, 232(%rsp)
    movq $0, 240(%rsp)
    movq $0, 248(%rsp)
    vmovdqu 32(%rsp), %ymm0
    vmovdqu %ymm0, 200(%rsp)
    movb $0, 232(%rsp)
    movq $0, 240(%rsp)
    movq $0, 248(%rsp)
    leaq 200(%rsp), %rdi
    leaq 256(%rsp), %rsi
    call sip24_init
    leaq 200(%rsp), %rdi
    leaq 272(%rsp), %rsi
    movq %r12, %rdx
    call sip24_update
    leaq 272(%rsp), %r11
    addq %r12, %r11
    movq %r13, %r10
    subq %r12, %r10
    leaq 200(%rsp), %rdi
    movq %r11, %rsi
    movq %r10, %rdx
    call sip24_update
    leaq 200(%rsp), %rdi
    call sip24_final
    movq %rax, %r9
    cmpq 8(%rsp), %rax
jne .LBB84
.LBB78:
    addq $1, %r12
    cmpq %r13, %r12
jbe .LBB77
.LBB79:
    addq $1, %r13
    cmpq $128, %r13
jbe .LBB75
.LBB80:
    xorl %r14d, %r14d
    movabsq $-7046029254386353131, %r15
.LBB81:
    cmpq $2000000, %r14
jae .LBB83
.LBB82:
    movq %r15, %rdi
    shrq $32, %rdi
    movq %rdi, %r11
    andq $127, %r11
    movq %r15, %rsi
    andq $7, %rsi
    movq %r15, %rdx
    orq $1, %rdx
    movq 256(%rsp), %rax
    movq %rdx, %rcx
    addq %rdx, %rax
    movq %rax, 256(%rsp)
    movq %r15, %r8
    shrq $48, %r8
    movzbl %r8b, %r9d
    movq %r14, %rax
    andq $127, %rax
    movzbl 272(%rsp, %rax), %r8d
    xorl %r9d, %r8d
    movzbl %r8b, %r9d
    movb %r9b, 272(%rsp, %rax)
    movq %rsi, %rdi
    leaq 272(%rsp), %rsi
    addq %rdi, %rsi
    leaq 256(%rsp), %rdx
    movq %rsi, %rdi
    movq %r11, %rsi
    call expat_siphash24@PLT
    xorq %rax, %r15
    addq $1, %r14
    cmpq $2000000, %r14
jb .LBB82
.LBB83:
    leaq .Lstr1(%rip), %rdi
    movq %r15, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    vzeroupper
.L__lccc_epilogue_1:
    addq $424, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB84:
    movl $1, %eax
    vzeroupper
    jmp .L__lccc_epilogue_1
.LBB85:
    xorl %r10d, %r10d
    jmp .LBB72


