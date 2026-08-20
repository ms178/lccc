main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r12
  pushq %rbx
  subq $16, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $12345, %ebx
  xorl %ebp, %ebp
  jmp .LBB0_1
.LBB0_6:
  movl $16, %edi
  callq malloc
  movl %ebx, (%rax)
  movl %ebp, 4(%rax)
  movq %r15, 8(%rax)
  movq %rax, table(,%r14,8)
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
  movzwl %ax, %r14d
  shrl $16, %eax
  xorl %eax, %r14d
  movq table(,%r14,8), %r15
  testq %r15, %r15
  je .LBB0_6
  movq %r15, %rax
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
  xorl %eax, %eax
  xorl %ebx, %ebx
  jmp .LBB0_9
.LBB0_12:
  movl 4(%rdx), %ecx
.LBB0_13:
  movslq %ecx, %rcx
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
  movq table(,%rdx,8), %rdx
  movl $-1, %ecx
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
  xorl %r14d, %r14d
  jmp .LBB0_15
.LBB0_21:
  movl $16, %edi
  callq malloc
  movl %ebp, (%rax)
  movl %r14d, 4(%rax)
  movq %r15, 8(%rax)
  movq %rax, table(,%r12,8)
.LBB0_27:
  incl %r14d
  cmpl $2000000, %r14d
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
  movzwl %ax, %r12d
  shrl $16, %eax
  xorl %eax, %r12d
  movq table(,%r12,8), %r15
  testb $1, %r14b
  jne .LBB0_16
  movl $-1, %eax
  testq %r15, %r15
  je .LBB0_26
.LBB0_24:
  cmpl %ebp, (%r15)
  je .LBB0_25
  movq 8(%r15), %r15
  testq %r15, %r15
  jne .LBB0_24
  jmp .LBB0_26
.LBB0_16:
  testq %r15, %r15
  je .LBB0_21
  movq %r15, %rax
.LBB0_18:
  cmpl %ebp, (%rax)
  je .LBB0_19
  movq 8(%rax), %rax
  testq %rax, %rax
  jne .LBB0_18
  jmp .LBB0_21
.LBB0_25:
  movl 4(%r15), %eax
.LBB0_26:
  cltq
  addq %rax, %rbx
  jmp .LBB0_27
.LBB0_19:
  movl %r14d, 4(%rax)
  jmp .LBB0_27
.LBB0_28:
  movl $.L.str, %edi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $16, %rsp
  popq %rbx
  popq %r12
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "hash_table sum: %ld\n"

