lookup:
  movl %edi, %eax
  shrl $16, %eax
  xorl %edi, %eax
  imull $73244475, %eax, %eax
  movl %eax, %edx
  shrl $16, %edx
  xorl %edx, %eax
  imull $73244475, %eax, %eax
  movl %eax, %edx
  shrl $16, %edx
  xorl %edx, %eax
  movzwl %ax, %eax
  movq table(,%rax,8), %rax
  testq %rax, %rax
  jne .L2
  jmp .L5
.L4:
  movq 8(%rax), %rax
  testq %rax, %rax
  je .L5
.L2:
  cmpl %edi, (%rax)
  jne .L4
  movl 4(%rax), %eax
  ret
.L5:
  movl $-1, %eax
  ret
insert:
  movl %edi, %eax
  shrl $16, %eax
  xorl %edi, %eax
  imull $73244475, %eax, %eax
  movl %eax, %edx
  shrl $16, %edx
  xorl %edx, %eax
  imull $73244475, %eax, %eax
  movl %eax, %edx
  shrl $16, %edx
  xorl %edx, %eax
  movzwl %ax, %edx
  movq table(,%rdx,8), %r8
  testq %r8, %r8
  je .L10
  movq %r8, %rax
  jmp .L13
.L11:
  movq 8(%rax), %rax
  testq %rax, %rax
  je .L10
.L13:
  cmpl %edi, (%rax)
  jne .L11
  movl %esi, 4(%rax)
  ret
.L10:
  subq $40, %rsp
  movl %edi, 8(%rsp)
  movl $16, %edi
  movq %rdx, 24(%rsp)
  movq %r8, 16(%rsp)
  movl %esi, 12(%rsp)
  call malloc
  movl 8(%rsp), %ecx
  movl 12(%rsp), %esi
  movq 16(%rsp), %r8
  movq 24(%rsp), %rdx
  movl %ecx, (%rax)
  movl %esi, 4(%rax)
  movq %r8, 8(%rax)
  movq %rax, table(,%rdx,8)
  addq $40, %rsp
  ret
.LC0:
  .string "hash_table sum: %ld\n"
main:
  pushq %r12
  pushq %rbp
  xorl %ebp, %ebp
  pushq %rbx
  movl $12345, %ebx
.L23:
  imull $1664525, %ebx, %ebx
  movl %ebp, %esi
  addl $1, %ebp
  addl $1013904223, %ebx
  movl %ebx, %edi
  call insert
  cmpl $2000000, %ebp
  jne .L23
  movl $2000000, %ecx
  xorl %r12d, %r12d
  movl $12345, %ebx
.L24:
  imull $1664525, %ebx, %ebx
  addl $1013904223, %ebx
  movl %ebx, %edi
  call lookup
  cltq
  addq %rax, %r12
  subl $1, %ecx
  jne .L24
  xorl %ebp, %ebp
.L28:
  imull $1664525, %ebx, %ebx
  addl $1013904223, %ebx
  testb $1, %bpl
  je .L25
  movl %ebp, %esi
  movl %ebx, %edi
  addl $1, %ebp
  call insert
  cmpl $2000000, %ebp
  jne .L28
  movq %r12, %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  popq %rbx
  xorl %eax, %eax
  popq %rbp
  popq %r12
  ret
.L25:
  movl %ebx, %edi
  addl $1, %ebp
  call lookup
  cltq
  addq %rax, %r12
  jmp .L28
