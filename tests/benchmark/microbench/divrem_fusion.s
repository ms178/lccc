.text
.globl unfused
unfused:
 movq %rdi, -8(%rsp)
 movq -8(%rsp), %rax
 xorq %rdx, %rdx
 divq %rsi
 movq %rdx, %r8
 movq -8(%rsp), %rax
 xorq %rdx, %rdx
 divq %rsi
 xorq %r8, %rax
 ret
.globl fused
fused:
 movq %rdi, -8(%rsp)
 movq -8(%rsp), %rax
 xorq %rdx, %rdx
 divq %rsi
 movq %rdx, %r8
 xorq %r8, %rax
 ret
.section .note.GNU-stack,"",@progbits
