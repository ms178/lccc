.section .rodata
.Lstr0:
    .byte 37, 108, 117, 10, 0

.section .rodata
.align 16
.type .LCA_0, @object
.size .LCA_0, 24
.LCA_0:
    .byte 32
    .zero 7
    .byte 4
    .zero 15
.align 16
.type .LCA_1, @object
.size .LCA_1, 24
.LCA_1:
    .zero 8
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
.align 16
.type .LCA_2, @object
.size .LCA_2, 24
.LCA_2:
    .byte 32
    .zero 7
    .byte 4
    .zero 15
.align 16
.type .LCA_3, @object
.size .LCA_3, 24
.LCA_3:
    .zero 8
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255
    .byte 255

.section .bss
.align 16
.type linux_bitmap_a, @object
.size linux_bitmap_a, 131072
linux_bitmap_a:
    .zero 131072
.align 16
.type linux_bitmap_b, @object
.size linux_bitmap_b, 131072
linux_bitmap_b:
    .zero 131072

.text
.p2align 4
.type linux_find_next_andnot_bit, @function
linux_find_next_andnot_bit:
.cfi_startproc
    pushq %r12
    pushq %rbp
    subq $24, %rsp
    .cfi_def_cfa_offset 48
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    cmpq %rdx, %rcx
jb .LBB2
.LBB1:
    movq %rdx, %rax
.L__lccc_epilogue_1:
    addq $24, %rsp
    popq %rbp
    popq %r12
    ret
.LBB2:
    movq %rcx, %r9
    andq $63, %r9
    movq $-1, %r11
    shlxq %r9, %r11, %r11
    movq %rcx, %r10
    shrq $6, %r10
    movq (%rdi, %r10, 8), %r12
    movq (%rsi, %r10, 8), %rax
    andnq %r12, %rax, %r9
    movq %r9, %rcx
    andq %r11, %rcx
.LBB3:
    testq %rcx, %rcx
jne .LBB6
.p2align 4,,10
.p2align 3
.LBB4:
    leaq 1(%r10), %r9
    shlq $6, %r9
    cmpq %rdx, %r9
jb .LBB5
.LBB7:
    movq %rdx, %rax
    jmp .L__lccc_epilogue_1
.LBB5:
    addq $1, %r10
    movq (%rdi, %r10, 8), %rbp
    movq (%rsi, %r10, 8), %rax
    andnq %rbp, %rax, %rcx
je .LBB4
.LBB6:
    shlq $6, %r10
    tzcntq %rcx, %r11
    movl %r11d, %edi
    addq %rdi, %r10
    cmpq %rdx, %r10
    movq %rdx, %rcx
    cmovbq %r10, %rcx
    movq %rcx, %rax
    jmp .L__lccc_epilogue_1
.cfi_endproc
.size linux_find_next_andnot_bit, .-linux_find_next_andnot_bit

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
    subq $24, %rsp
    .cfi_def_cfa_offset 80
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    leaq .LCA_2(%rip), %rdi
    leaq .LCA_3(%rip), %rsi
    movl $192, %edx
    xorl %ecx, %ecx
    call linux_find_next_andnot_bit
    # LCCC_CALL_ARGS 4
    # LCCC_CALL_FP 0
    cmpq $5, %rax
jne .LBB10
.LBB9:
    leaq .LCA_2(%rip), %rdi
    leaq .LCA_3(%rip), %rsi
    movl $192, %edx
    movl $6, %ecx
    call linux_find_next_andnot_bit
    # LCCC_CALL_ARGS 4
    # LCCC_CALL_FP 0
    cmpq $192, %rax
    movl $0, %ecx
    movl $1, %r8d
    cmovnel %ecx, %r8d
    testl %r8d, %r8d
jne .LBB11
.LBB10:
    movl $2, %eax
.L__lccc_epilogue_2:
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB11:
    leaq linux_bitmap_b(%rip), %rbx
    leaq linux_bitmap_a(%rip), %r12
    xorl %r13d, %r13d
.LBB12:
    cmpq $16384, %r13
jae .LBB16
.p2align 4,,10
.p2align 3
.LBB13:
    movq $0, (%r12, %r13, 8)
    movq $-1, (%rbx, %r13, 8)
    movq %r13, %rdx
    andq $63, %rdx
    cmpq $5, %rdx
jne .LBB15
.LBB14:
    imulq $13, %r13, %r11
    leaq 7(%r11), %r10
    andq $63, %r10
    movl $1, %r8d
    shlxq %r10, %r8, %r8
    movq %r13, %r9
    shlq $3, %r9
    movq %r8, (%r12, %r13, 8)
    movq $0, (%rbx, %r13, 8)
.LBB15:
    addq $1, %r13
    cmpq $16384, %r13
jb .LBB13
.LBB16:
    xorl %r14d, %r14d
    xorl %r11d, %r11d
.LBB17:
    cmpl $32768, %r14d
jae .LBB21
.p2align 4,,10
.p2align 3
.LBB18:
    movq %r14, %rax
    andq $63, %rax
    movl %eax, %r15d
    movl %r14d, %edx
    movq %rdx, %rbp
    shlq $19, %rbp
    movq %r11, %r12
.p2align 4,,10
.p2align 3
.LBB19:
    leaq linux_bitmap_a(%rip), %rdi
    leaq linux_bitmap_b(%rip), %rsi
    movl $1048576, %edx
    movq %r15, %rcx
    call linux_find_next_andnot_bit
    # LCCC_CALL_ARGS 4
    # LCCC_CALL_FP 0
    leaq (%rax, %rbp, 1), %r9
    movq %r12, %rdi
    xorq %r9, %rdi
    leaq 1(%rax), %rsi
    cmpq $1048576, %rax
    movq %r12, %r11
    cmovbq %rdi, %r11
    cmpq $1048576, %rax
    cmovbq %rsi, %r15
    movq %r11, %r12
    cmpq $1048576, %rax
jb .LBB19
.LBB20:
    movl %r14d, %edx
    imulq $13, %rdx, %rdx
    andq $255, %rdx
    shlq $6, %rdx
    addq $5, %rdx
    leal 0(,%r14,8), %r10d
    subl %r14d, %r10d
    andl $63, %r10d
    movl $1, %r8d
    shlxq %r10, %r8, %r8
    xorq (%rbx, %rdx, 8), %r8
    movq %r8, (%rbx, %rdx, 8)
    addl $1, %r14d
    cmpl $32768, %r14d
jb .LBB18
.LBB21:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    # LCCC_VA_CALL
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    xorl %eax, %eax
    jmp .L__lccc_epilogue_2
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
