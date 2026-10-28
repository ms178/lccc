# Each 32-bit self-move below is an observable zero-extension, not a no-op.
# These are ordinary backend instructions, not an inline-asm region.
.text
.globl upper32_partial_word
.type upper32_partial_word,@function
upper32_partial_word:
.cfi_startproc
    movabsq $0x100000080, %rax
    movzbw %al, %ax
    movl %eax, %eax
    ret
.cfi_endproc
.size upper32_partial_word, .-upper32_partial_word

.globl upper32_divide
.type upper32_divide,@function
upper32_divide:
.cfi_startproc
    movq %rdi, %rax
    andq $127, %rax
    movq $1, %rdx
    movq $2, %rcx
    divq %rcx
    movl %eax, %eax
    ret
.cfi_endproc
.size upper32_divide, .-upper32_divide

.globl upper32_cmpxchg_implicit
.type upper32_cmpxchg_implicit,@function
upper32_cmpxchg_implicit:
.cfi_startproc
    movq $6, %rax
    andq $127, %rax
    movabsq $0x100000006, %rdi
    movq $42, %rsi
    cmpxchgq %rsi, %rdi
    movl %eax, %eax
    ret
.cfi_endproc
.size upper32_cmpxchg_implicit, .-upper32_cmpxchg_implicit

.globl upper32_cmpxchg_explicit
.type upper32_cmpxchg_explicit,@function
upper32_cmpxchg_explicit:
.cfi_startproc
    movl $1, %eax
    movl $1, %edi
    movabsq $0x100000006, %rsi
    cmpxchgq %rsi, %rdi
    movl %edi, %edi
    movq %rdi, %rax
    ret
.cfi_endproc
.size upper32_cmpxchg_explicit, .-upper32_cmpxchg_explicit

.globl upper32_cmpxchg_locked
.type upper32_cmpxchg_locked,@function
upper32_cmpxchg_locked:
.cfi_startproc
    movq $6, %rax
    andq $127, %rax
    movq $42, %rsi
    lock cmpxchgq %rsi, (%rdi)
    movl %eax, %eax
    ret
.cfi_endproc
.size upper32_cmpxchg_locked, .-upper32_cmpxchg_locked

# GAS accepts the negative spelling of the sign-extended imm32 boundary.
.globl upper32_negative_mask
.type upper32_negative_mask,@function
upper32_negative_mask:
.cfi_startproc
    movabsq $0x100000080, %rax
    andq $-2147483648, %rax
    movl %eax, %eax
    ret
.cfi_endproc
.size upper32_negative_mask, .-upper32_negative_mask
.section .note.GNU-stack,"",@progbits
