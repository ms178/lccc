.section .bss
.globl g_shared
.align 4
.type g_shared, @object
.size g_shared, 4
g_shared:
    .zero 4

.text
.globl from_cmp
.p2align 4
.type from_cmp, @function
from_cmp:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    cmpl %esi, %edi
    movl $1, %ecx
    movl $0, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size from_cmp, .-from_cmp

.globl from_cmp_neg
.p2align 4
.type from_cmp_neg, @function
from_cmp_neg:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    cmpl %esi, %edi
    movl $0, %ecx
    movl $1, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size from_cmp_neg, .-from_cmp_neg

.globl from_lt
.p2align 4
.type from_lt, @function
from_lt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    cmpl %esi, %edi
    movl $1, %ecx
    movl $0, %eax
    cmovll %ecx, %eax
    ret
.cfi_endproc
.size from_lt, .-from_lt

.globl from_lt_neg
.p2align 4
.type from_lt_neg, @function
from_lt_neg:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    cmpl %esi, %edi
    movl $0, %ecx
    movl $1, %eax
    cmovll %ecx, %eax
    ret
.cfi_endproc
.size from_lt_neg, .-from_lt_neg

.globl zero_or_one
.p2align 4
.type zero_or_one, @function
zero_or_one:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    movl $1, %ecx
    movl $0, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size zero_or_one, .-zero_or_one

.globl one_or_zero
.p2align 4
.type one_or_zero, @function
one_or_zero:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    movl $0, %ecx
    movl $1, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size one_or_zero, .-one_or_zero

.globl mask_of_sign
.p2align 4
.type mask_of_sign, @function
mask_of_sign:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    movl $0, %ecx
    movl $1, %eax
    cmovll %ecx, %eax
    ret
.cfi_endproc
.size mask_of_sign, .-mask_of_sign

.globl zero_or_one64
.p2align 4
.type zero_or_one64, @function
zero_or_one64:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testq %rdi, %rdi
    movl $1, %ecx
    movl $0, %eax
    cmovneq %rcx, %rax
    ret
.cfi_endproc
.size zero_or_one64, .-zero_or_one64

.globl or_or
.p2align 4
.type or_or, @function
or_or:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    setne %dl
    movzbl %dl, %edx
    testl %esi, %esi
    setne %r8b
    movzbl %r8b, %r8d
    orl %r8d, %edx
    movl %edx, %eax
    ret
.cfi_endproc
.size or_or, .-or_or

.globl and_eq
.p2align 4
.type and_eq, @function
and_eq:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    sete %dl
    movzbl %dl, %edx
    testl %esi, %esi
    sete %r8b
    movzbl %r8b, %r8d
    andl %r8d, %edx
    movl %edx, %eax
    ret
.cfi_endproc
.size and_eq, .-and_eq

.globl or3
.p2align 4
.type or3, @function
or3:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    setne %r8b
    movzbl %r8b, %r8d
    testl %esi, %esi
    setne %r9b
    movzbl %r9b, %r9d
    orl %r9d, %r8d
    testl %edx, %edx
    setne %r11b
    movzbl %r11b, %r11d
    orl %r11d, %r8d
    movl %r8d, %eax
    ret
.cfi_endproc
.size or3, .-or3

.globl keep_and_ne
.p2align 4
.type keep_and_ne, @function
keep_and_ne:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    setne %dl
    movzbl %dl, %edx
    testl %esi, %esi
    setne %r8b
    movzbl %r8b, %r8d
    andl %r8d, %edx
    movl %edx, %eax
    ret
.cfi_endproc
.size keep_and_ne, .-keep_and_ne

.globl keep_or_eq
.p2align 4
.type keep_or_eq, @function
keep_or_eq:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    sete %dl
    movzbl %dl, %edx
    testl %esi, %esi
    sete %r8b
    movzbl %r8b, %r8d
    orl %r8d, %edx
    movl %edx, %eax
    ret
.cfi_endproc
.size keep_or_eq, .-keep_or_eq

.globl keep_xor
.p2align 4
.type keep_xor, @function
keep_xor:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    setne %dl
    movzbl %dl, %edx
    testl %esi, %esi
    setne %r8b
    movzbl %r8b, %r8d
    xorl %r8d, %edx
    movl %edx, %eax
    ret
.cfi_endproc
.size keep_xor, .-keep_xor

.globl keep_three_two
.p2align 4
.type keep_three_two, @function
keep_three_two:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    movl $3, %ecx
    movl $2, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size keep_three_two, .-keep_three_two

.globl keep_one_two
.p2align 4
.type keep_one_two, @function
keep_one_two:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    testl %edi, %edi
    movl $1, %ecx
    movl $2, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size keep_one_two, .-keep_one_two

.globl keep_shared
.p2align 4
.type keep_shared, @function
keep_shared:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    cmpl %esi, %edi
    setne %dl
    movzbl %dl, %edx
    movl %edx, g_shared(%rip)
    testl %edx, %edx
    movl $0, %ecx
    movl $1, %eax
    cmovnel %ecx, %eax
    ret
.cfi_endproc
.size keep_shared, .-keep_shared


.section .note.GNU-stack,"",@progbits
