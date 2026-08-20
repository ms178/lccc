main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  movl $12345, %ebx
  xorl %ebp, %ebp
  leaq table(%rip), %r14
  jmp .LBB0_1
.LBB0_6:
  movl $16, %edi
  callq malloc@PLT
  movl %ebx, (%rax)
  movl %ebp, 4(%rax)
  movq %r12, 8(%rax)
  movq %rax, (%r14,%r15,8)
.LBB0_7:
  incl %ebp
  cmpl $2000000, %ebp
  je .LBB0_8
.LBB0_1:
  imull $1664525, %ebx, %ebx
  addl $1013904223, %ebx
  movl %ebx, %eax
  shrl $16, %eax
  xorl %ebx, %eax
  imull $73244475, %eax, %eax
  movl %eax, %ecx
  shrl $16, %ecx
  xorl %eax, %ecx
  imull $73244475, %ecx, %eax
  movzwl %ax, %r15d
  shrl $16, %eax
  xorl %eax, %r15d
  movq (%r14,%r15,8), %r12
  testq %r12, %r12
  je .LBB0_6
  movq %r12, %rax
.LBB0_3:
  cmpl %ebx, (%rax)
  je .LBB0_4
  movq 8(%rax), %rax
  testq %rax, %rax
  jne .LBB0_3
  jmp .LBB0_6
.LBB0_4:
  movl %ebp, 4(%rax)
  jmp .LBB0_7
.LBB0_8:
  movl $12345, %ebp
  xorl %ebx, %ebx
  xorl %eax, %eax
  jmp .LBB0_9
.LBB0_12:
  movslq 4(%rdx), %rcx
.LBB0_13:
  addq %rcx, %rbx
  incl %eax
  cmpl $2000000, %eax
  je .LBB0_14
.LBB0_9:
  imull $1664525, %ebp, %ebp
  addl $1013904223, %ebp
  movl %ebp, %ecx
  shrl $16, %ecx
  xorl %ebp, %ecx
  imull $73244475, %ecx, %ecx
  movl %ecx, %edx
  shrl $16, %edx
  xorl %ecx, %edx
  imull $73244475, %edx, %ecx
  movzwl %cx, %edx
  shrl $16, %ecx
  xorl %ecx, %edx
  movq (%r14,%rdx,8), %rdx
  movq $-1, %rcx
  testq %rdx, %rdx
  je .LBB0_13
.LBB0_11:
  cmpl %ebp, (%rdx)
  je .LBB0_12
  movq 8(%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_11
  jmp .LBB0_13
.LBB0_14:
  xorl %r15d, %r15d
  jmp .LBB0_15
.LBB0_21:
  movl $16, %edi
  callq malloc@PLT
  movl %ebp, (%rax)
  movl %r15d, 4(%rax)
  movq %r12, 8(%rax)
  movq %rax, (%r14,%r13,8)
.LBB0_27:
  incl %r15d
  cmpl $2000000, %r15d
  je .LBB0_28
.LBB0_15:
  imull $1664525, %ebp, %ebp
  addl $1013904223, %ebp
  movl %ebp, %eax
  shrl $16, %eax
  xorl %ebp, %eax
  imull $73244475, %eax, %eax
  movl %eax, %ecx
  shrl $16, %ecx
  xorl %eax, %ecx
  imull $73244475, %ecx, %eax
  movzwl %ax, %r13d
  shrl $16, %eax
  xorl %eax, %r13d
  movq (%r14,%r13,8), %r12
  testb $1, %r15b
  jne .LBB0_16
  movq $-1, %rax
  testq %r12, %r12
  je .LBB0_26
.LBB0_24:
  cmpl %ebp, (%r12)
  je .LBB0_25
  movq 8(%r12), %r12
  testq %r12, %r12
  jne .LBB0_24
  jmp .LBB0_26
.LBB0_16:
  testq %r12, %r12
  je .LBB0_21
  movq %r12, %rax
.LBB0_18:
  cmpl %ebp, (%rax)
  je .LBB0_19
  movq 8(%rax), %rax
  testq %rax, %rax
  jne .LBB0_18
  jmp .LBB0_21
.LBB0_25:
  movslq 4(%r12), %rax
.LBB0_26:
  addq %rax, %rbx
  jmp .LBB0_27
.LBB0_19:
  movl %r15d, 4(%rax)
  jmp .LBB0_27
.LBB0_28:
  leaq .L.str(%rip), %rdi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "hash_table sum: %ld\n"

