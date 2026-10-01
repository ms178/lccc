bc_scan:
    pushq %rbx
    subq $64, %rsp
    jmp .LBB1
.LBB6:
    cmpq %rsi, %r8
jae .LBB9
.LBB7:
    movzbl (%r8), %r11d
    cmpb (%r9), %r11b
jne .LBB9
.LBB8:
    addq $1, %r8
    addq $1, %r9
    cmpq %rsi, %r8
jb .LBB7
.LBB9:
    movq %r8, %rax
    addq $64, %rsp
    popq %rbx
    ret
.LBB1:
    leaq 31(%rdi), %r8
    cmpq %rsi, %r8
jb .LBB2
.LBB5:
    movq %rdi, %r8
    movq %rdx, %r9
    jmp .LBB6
.LBB2:
    leaq 31(%rdx), %r9
    movq %rdx, %r11
    orq $4095, %r11
    cmpq %r11, %r9
jbe .LBB3
.LBB11:
    movq %rdi, %r8
    movq %rdx, %r9
    jmp .LBB6
.LBB3:
    vmovdqu (%rdi), %ymm2
    vmovdqu (%rdx), %ymm3
    vpcmpeqb %ymm3, %ymm2, %ymm0
    vpmovmskb %ymm0, %eax
    cmpl $-1, %eax
je .LBB4
.LBB10:
    xorq $-1, %rax
    tzcntl %eax, %eax
    movl %eax, %r10d
    leaq (%rdi, %r10, 1), %r11
    leaq (%rdx, %r10, 1), %rbx
    movq %r11, %r8
    movq %rbx, %r9
    jmp .LBB6
.LBB4:
    addq $32, %rdi
    addq $32, %rdx
    leaq 31(%rdi), %r10
    cmpq %rsi, %r10
jb .LBB2
    jmp .LBB5

bc_scan_signed:
    pushq %rbx
    subq $64, %rsp
    jmp .LBB13
.LBB18:
    cmpq %rsi, %r8
jae .LBB21
.LBB19:
    movsbl (%r8), %r11d
    movsbl (%r9), %r10d
    cmpb %r10b, %r11b
jne .LBB21
.LBB20:
    addq $1, %r8
    addq $1, %r9
    cmpq %rsi, %r8
jb .LBB19
.LBB21:
    movq %r8, %rax
    addq $64, %rsp
    popq %rbx
    ret
.LBB13:
    leaq 31(%rdi), %r8
    cmpq %rsi, %r8
jb .LBB14
.LBB17:
    movq %rdi, %r8
    movq %rdx, %r9
    jmp .LBB18
.LBB14:
    leaq 31(%rdx), %r9
    movq %rdx, %r11
    orq $4095, %r11
    cmpq %r11, %r9
jbe .LBB15
.LBB23:
    movq %rdi, %r8
    movq %rdx, %r9
    jmp .LBB18
.LBB15:
    vmovdqu (%rdi), %ymm2
    vmovdqu (%rdx), %ymm3
    vpcmpeqb %ymm3, %ymm2, %ymm0
    vpmovmskb %ymm0, %eax
    cmpl $-1, %eax
je .LBB16
.LBB22:
    xorq $-1, %rax
    tzcntl %eax, %eax
    movl %eax, %r10d
    leaq (%rdi, %r10, 1), %r11
    leaq (%rdx, %r10, 1), %rbx
    movq %r11, %r8
    movq %rbx, %r9
    jmp .LBB18
.LBB16:
    addq $32, %rdi
    addq $32, %rdx
    leaq 31(%rdi), %r10
    cmpq %rsi, %r10
jb .LBB14
    jmp .LBB17

match_extend:
    movq %rdi, %r8
.LBB25:
    leaq 8(%r8), %r9
    cmpq %rdx, %r9
ja .LBB28
.LBB26:
    movq (%r8), %r11
    cmpq (%rsi), %r11
jne .LBB33
.LBB27:
    addq $8, %r8
    addq $8, %rsi
    leaq 8(%r8), %r9
    cmpq %rdx, %r9
jbe .LBB26
.LBB28:
    movq %r8, %r11
    movq %rsi, %r10
.LBB29:
    cmpq %rdx, %r11
jae .LBB32
.LBB30:
    movzbl (%r11), %esi
    cmpb (%r10), %sil
jne .LBB32
.LBB31:
    addq $1, %r11
    addq $1, %r10
    cmpq %rdx, %r11
jb .LBB30
.LBB32:
    movq %r11, %rax
    subq %rdi, %rax
    ret
.LBB33:
    movq %r8, %r11
    movq %rsi, %r10
    jmp .LBB29


