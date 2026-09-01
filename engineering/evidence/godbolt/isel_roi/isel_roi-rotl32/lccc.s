rotl32:
.cfi_startproc
    .cfi_def_cfa_offset 8
    andl $31, %esi
    movslq %esi, %rdx
    movl %edi, %r8d
    movl %edx, %ecx
    shll %cl, %r8d
    movl $32, %eax
    subl %esi, %eax
    movq %rax, %r9
    movl %edi, %r11d
    movl %eax, %ecx
    shrl %cl, %r11d
    orl %r11d, %r8d
    testq %rsi, %rsi
    cmovneq %r8, %rdi
    movq %rdi, %rax
    ret
