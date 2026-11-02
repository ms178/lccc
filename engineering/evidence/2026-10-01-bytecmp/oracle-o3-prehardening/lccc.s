bc_scan:
.LBB1:
    leaq 32(%rdi), %r8
    cmpq %rsi, %r8
jae .LBB4
.LBB2:
    vmovdqu (%rdi), %ymm2
    vmovdqu (%rdx), %ymm3
    vpcmpeqb %ymm3, %ymm2, %ymm0
    vpmovmskb %ymm0, %eax
    cmpl $-1, %eax
jne .LBB9
.LBB3:
    addq $32, %rdi
    addq $32, %rdx
    leaq 32(%rdi), %r9
    cmpq %rsi, %r9
jb .LBB2
.LBB4:
    movq %rdi, %r11
    movq %rdx, %r10
.LBB5:
    cmpq %rsi, %r11
jae .LBB8
.LBB6:
    movzbl (%r11), %edi
    cmpb (%r10), %dil
jne .LBB8
.LBB7:
    addq $1, %r11
    addq $1, %r10
    cmpq %rsi, %r11
jb .LBB6
.LBB8:
    movq %r11, %rax
    vzeroupper
    ret
.LBB9:
    movq %rdi, %r11
    movq %rdx, %r10
    jmp .LBB5

bc_scan_signed:
.LBB11:
    leaq 32(%rdi), %r8
    cmpq %rsi, %r8
jae .LBB14
.LBB12:
    vmovdqu (%rdi), %ymm2
    vmovdqu (%rdx), %ymm3
    vpcmpeqb %ymm3, %ymm2, %ymm0
    vpmovmskb %ymm0, %eax
    cmpl $-1, %eax
jne .LBB19
.LBB13:
    addq $32, %rdi
    addq $32, %rdx
    leaq 32(%rdi), %r9
    cmpq %rsi, %r9
jb .LBB12
.LBB14:
    movq %rdi, %r11
    movq %rdx, %r10
.LBB15:
    cmpq %rsi, %r11
jae .LBB18
.LBB16:
    movsbl (%r11), %edi
    movsbl (%r10), %edx
    cmpb %dl, %dil
jne .LBB18
.LBB17:
    addq $1, %r11
    addq $1, %r10
    cmpq %rsi, %r11
jb .LBB16
.LBB18:
    movq %r11, %rax
    vzeroupper
    ret
.LBB19:
    movq %rdi, %r11
    movq %rdx, %r10
    jmp .LBB15

match_extend:
    movq %rdi, %r8
.LBB21:
    leaq 8(%r8), %r9
    cmpq %rdx, %r9
ja .LBB24
.LBB22:
    movq (%r8), %r11
    cmpq (%rsi), %r11
jne .LBB29
.LBB23:
    addq $8, %r8
    addq $8, %rsi
    leaq 8(%r8), %r9
    cmpq %rdx, %r9
jbe .LBB22
.LBB24:
    movq %r8, %r11
    movq %rsi, %r10
.LBB25:
    cmpq %rdx, %r11
jae .LBB28
.LBB26:
    movzbl (%r11), %esi
    cmpb (%r10), %sil
jne .LBB28
.LBB27:
    addq $1, %r11
    addq $1, %r10
    cmpq %rdx, %r11
jb .LBB26
.LBB28:
    movq %r11, %rax
    subq %rdi, %rax
    ret
.LBB29:
    movq %r8, %r11
    movq %rsi, %r10
    jmp .LBB25


